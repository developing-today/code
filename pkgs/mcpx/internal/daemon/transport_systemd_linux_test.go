package daemon

import (
	"fmt"
	"net"
	"net/http"
	"net/http/httptest"
	"os"
	"os/exec"
	"path/filepath"
	"strconv"
	"strings"
	"testing"
	"time"

	"golang.org/x/sys/unix"
)

func TestTCPSystemd(t *testing.T) {
	for _, mode := range []string{"success", "bad-pid", "missing-pid", "bad-count", "missing-count", "file", "unix", "closed", "address", "port", "absent"} {
		t.Run(mode, func(t *testing.T) {
			listener, err := net.ListenTCP("tcp4", &net.TCPAddr{IP: net.ParseIP("127.0.0.1")})
			if err != nil {
				t.Fatal(err)
			}
			defer func() { _ = listener.Close() }()
			f, err := listener.File()
			if err != nil {
				t.Fatal(err)
			}
			defer func() { _ = f.Close() }()
			if mode == "file" {
				f, err = os.Open(os.DevNull)
				if err != nil {
					t.Fatal(err)
				}
				defer func() { _ = f.Close() }()
			}
			if mode == "unix" {
				other, err := net.ListenUnix("unix", &net.UnixAddr{Name: filepath.Join(t.TempDir(), "listener.sock"), Net: "unix"})
				if err != nil {
					t.Fatal(err)
				}
				defer func() { _ = other.Close() }()
				f, err = other.File()
				if err != nil {
					t.Fatal(err)
				}
				defer func() { _ = f.Close() }()
			}
			cmd := exec.Command(os.Args[0], "-test.run=^TestTCPSystemdChild$")
			for _, entry := range os.Environ() {
				key := strings.SplitN(entry, "=", 2)[0]
				switch key {
				case "LISTEN_PID", "LISTEN_FDS", "LISTEN_FDNAMES", "MCPX_TEST_SYSTEMD_MODE", "MCPX_TEST_SYSTEMD_ADDR":
					continue
				}
				cmd.Env = append(cmd.Env, entry)
			}
			cmd.Env = append(cmd.Env, "MCPX_TEST_SYSTEMD_MODE="+mode, "MCPX_TEST_SYSTEMD_ADDR="+listener.Addr().String())
			cmd.ExtraFiles = []*os.File{f}
			if out, err := cmd.CombinedOutput(); err != nil {
				t.Fatalf("%v: %s", err, out)
			}
		})
	}
}

func TestTCPSystemdChild(t *testing.T) {
	mode := os.Getenv("MCPX_TEST_SYSTEMD_MODE")
	if mode == "" {
		t.Skip("subprocess helper")
	}
	target := os.Getenv("MCPX_TEST_SYSTEMD_ADDR")
	transport := tcpTransport{address: target}
	if mode != "absent" && mode != "missing-count" {
		t.Setenv("LISTEN_FDS", "1")
	}
	if mode != "absent" && mode != "missing-pid" {
		t.Setenv("LISTEN_PID", strconv.Itoa(os.Getpid()))
	}
	switch mode {
	case "bad-pid":
		t.Setenv("LISTEN_PID", strconv.Itoa(os.Getpid()+1))
	case "bad-count":
		t.Setenv("LISTEN_FDS", "2")
	case "absent":
		transport.address = "127.0.0.1:0"
	case "closed":
		if err := unix.Close(3); err != nil {
			t.Fatal(err)
		}
	case "port":
		transport.address = "127.0.0.1:1"
	case "success":
		t.Setenv("LISTEN_FDNAMES", "mcpx")
	case "address":
		transport.address = "127.0.0.2:0"
	}
	listener, activated, err := transport.listen()
	if activated != (mode == "success") {
		t.Fatalf("activation=%v for %s", activated, mode)
	}
	if err != nil && (mode == "absent" || mode == "success") {
		t.Fatal(err)
	}
	switch mode {
	case "bad-pid", "missing-pid", "bad-count", "missing-count", "file", "unix", "closed", "address", "port":
		if err == nil {
			_ = listener.Close()
			t.Fatal("activation silently succeeded or fell back")
		}
		expected := map[string]string{"bad-pid": "LISTEN_PID", "missing-pid": "LISTEN_PID", "bad-count": "LISTEN_FDS", "missing-count": "LISTEN_FDS", "file": "inspect fd 3", "unix": "not a TCP listener", "closed": "inspect fd 3", "address": "does not match", "port": "does not match"}[mode]
		if !strings.Contains(err.Error(), expected) {
			t.Fatalf("got %v, want %q", err, expected)
		}
		if mode == "file" || mode == "unix" {
			if _, err := unix.FcntlInt(3, unix.F_GETFD, 0); err != nil {
				t.Fatalf("closed unverified fd: %v", err)
			}
		}
		if mode != "address" && mode != "port" {
			return
		}
	default:
		defer func() { _ = listener.Close() }()
		if mode == "absent" {
			if listener.Addr().String() == target {
				t.Fatal("adopted without activation")
			}
			return
		}
		if listener.Addr().String() != target {
			t.Fatalf("listener %s differs from inherited %s", listener.Addr(), target)
		}
	}
	if _, err := unix.FcntlInt(3, unix.F_GETFD, 0); err != unix.EBADF {
		t.Fatalf("original activation fd not closed: %v", err)
	}
	if mode != "success" {
		return
	}
	for _, name := range []string{"LISTEN_PID", "LISTEN_FDS", "LISTEN_FDNAMES"} {
		if _, ok := os.LookupEnv(name); ok {
			t.Fatalf("%s was not consumed", name)
		}
	}
	// The parent is not accepting connections. The child must use the
	// inherited listener and successfully exchange data through it.
	if err := listener.(*net.TCPListener).SetDeadline(time.Now().Add(3 * time.Second)); err != nil {
		t.Fatal(err)
	}
	client, err := net.DialTimeout("tcp", target, 3*time.Second)
	if err != nil {
		t.Fatal(err)
	}
	defer func() { _ = client.Close() }()
	server, err := listener.Accept()
	if err != nil {
		t.Fatal(err)
	}
	defer func() { _ = server.Close() }()
	if _, err := fmt.Fprintln(server, "ok"); err != nil {
		t.Fatal(err)
	}
	_ = client.SetReadDeadline(time.Now().Add(3 * time.Second))
	var result string
	if _, err := fmt.Fscan(client, &result); err != nil || result != "ok" {
		t.Fatalf("read %q, %v", result, err)
	}
}

func TestSocketActivatedDaemonRejectsInPlaceRestart(t *testing.T) {
	s := &Server{tcpSocketActivated: true}
	w := httptest.NewRecorder()
	s.handleDaemonRestart(w, httptest.NewRequest(http.MethodPost, "/v1/daemon/restart", nil))
	if w.Code != http.StatusConflict || !strings.Contains(w.Body.String(), "through systemd") {
		t.Fatalf("status %d: %s", w.Code, w.Body.String())
	}
}
