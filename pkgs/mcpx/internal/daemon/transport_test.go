package daemon

import (
	"bufio"
	"context"
	"fmt"
	"net"
	"os"
	"os/exec"
	"path/filepath"
	"strconv"
	"strings"
	"testing"
	"time"

	"github.com/developing-today/code/pkgs/mcpx/internal/config"
)

// The two listeners the daemon has always had now run through the
// Transport interface; this pins that the seam did not change what they
// do — bind, and be reachable at the address they report.
func TestBuiltinTransportsListen(t *testing.T) {
	for _, tr := range []Transport{
		unixTransport{socket: filepath.Join(t.TempDir(), "mcpx.sock")},
		tcpTransport{address: "127.0.0.1:0"},
	} {
		ln, err := tr.Listen()
		if err != nil {
			t.Fatalf("%s transport: %v", tr.Name(), err)
		}
		defer func() { _ = ln.Close() }()
		go func() {
			conn, err := ln.Accept()
			if err == nil {
				_ = conn.Close()
			}
		}()
		ctx, cancel := context.WithTimeout(context.Background(), 5*time.Second)
		defer cancel()
		var d net.Dialer
		conn, err := d.DialContext(ctx, ln.Addr().Network(), ln.Addr().String())
		if err != nil {
			t.Fatalf("%s transport: dial %s: %v", tr.Name(), ln.Addr(), err)
		}
		_ = conn.Close()
	}
}

// The oracle is net.TCPListener: when systemd gives the daemon its TCP
// socket, the daemon must serve through that same listener without binding
// the configured address a second time.
func TestSystemdTCPTransportUsesPassedListener(t *testing.T) {
	original, err := net.Listen("tcp", "127.0.0.1:0")
	if err != nil {
		t.Fatal(err)
	}
	defer original.Close()

	file, err := original.(*net.TCPListener).File()
	if err != nil {
		t.Fatal(err)
	}
	defer file.Close()

	ln, err := systemdTCPListenerFromFile(file)
	if err != nil {
		t.Fatalf("systemd listener: %v", err)
	}
	defer ln.Close()

	if got := ln.Addr().String(); got != original.Addr().String() {
		t.Fatalf("listener address = %q, want inherited address %q", got, original.Addr())
	}
	if ln.Addr().Network() != "tcp" {
		t.Fatalf("listener network = %q, want tcp", ln.Addr().Network())
	}

	go func() {
		conn, err := ln.Accept()
		if err == nil {
			_ = conn.Close()
		}
	}()
	ctx, cancel := context.WithTimeout(context.Background(), 5*time.Second)
	defer cancel()
	conn, err := (&net.Dialer{}).DialContext(ctx, "tcp", original.Addr().String())
	if err != nil {
		t.Fatalf("dial inherited listener: %v", err)
	}
	_ = conn.Close()
}

func TestSystemdTCPTransportActivationChild(t *testing.T) {
	if os.Getenv("MCPX_SYSTEMD_ACTIVATION_CHILD") != "1" {
		return
	}
	if err := os.Setenv("LISTEN_PID", strconv.Itoa(os.Getpid())); err != nil {
		t.Fatal(err)
	}
	if err := os.Setenv("LISTEN_FDS", "1"); err != nil {
		t.Fatal(err)
	}
	ln, err := (tcpTransport{address: "127.0.0.1:1"}).Listen()
	if err != nil {
		t.Fatal(err)
	}
	defer ln.Close()
	fmt.Println(ln.Addr().String())
	conn, err := ln.Accept()
	if err != nil {
		t.Fatal(err)
	}
	_ = conn.Close()
}

func TestTCPTransportAdoptsSystemdListener(t *testing.T) {
	original, err := net.Listen("tcp", "127.0.0.1:0")
	if err != nil {
		t.Fatal(err)
	}
	defer original.Close()
	file, err := original.(*net.TCPListener).File()
	if err != nil {
		t.Fatal(err)
	}
	defer file.Close()

	executable, err := os.Executable()
	if err != nil {
		t.Fatal(err)
	}
	cmd := exec.Command(executable, "-test.run=^TestSystemdTCPTransportActivationChild$")
	cmd.ExtraFiles = []*os.File{file}
	cmd.Env = append(os.Environ(), "MCPX_SYSTEMD_ACTIVATION_CHILD=1")
	stdout, err := cmd.StdoutPipe()
	if err != nil {
		t.Fatal(err)
	}
	if err := cmd.Start(); err != nil {
		t.Fatal(err)
	}
	defer func() {
		if cmd.ProcessState == nil {
			_ = cmd.Process.Kill()
			_ = cmd.Wait()
		}
	}()
	scanner := bufio.NewScanner(stdout)
	if !scanner.Scan() {
		t.Fatalf("activation child did not report listener address: %v", scanner.Err())
	}
	got := scanner.Text()
	if got != original.Addr().String() {
		t.Fatalf("child listener address = %q, want inherited %q", got, original.Addr())
	}
	conn, err := (&net.Dialer{}).Dial("tcp", got)
	if err != nil {
		t.Fatalf("dial inherited listener: %v", err)
	}
	_ = conn.Close()
	if err := cmd.Wait(); err != nil {
		t.Fatalf("activation child: %v", err)
	}
}

func TestSystemdActivationRequestedValidation(t *testing.T) {
	cases := []struct {
		name    string
		pid     string
		pidSet  bool
		fds     string
		fdsSet  bool
		want    bool
		wantErr bool
	}{
		{name: "not activated"},
		{name: "pid mismatch ignores inherited state", pid: "1", pidSet: true, fds: "bad", fdsSet: true},
		{name: "missing fd count", pid: "42", pidSet: true, wantErr: true},
		{name: "malformed pid", pid: "bad", pidSet: true, fds: "1", fdsSet: true, wantErr: true},
		{name: "zero descriptors", pid: "42", pidSet: true, fds: "0", fdsSet: true, wantErr: true},
		{name: "multiple descriptors", pid: "42", pidSet: true, fds: "2", fdsSet: true, wantErr: true},
		{name: "one descriptor", pid: "42", pidSet: true, fds: "1", fdsSet: true, want: true},
	}
	for _, tc := range cases {
		t.Run(tc.name, func(t *testing.T) {
			got, err := systemdActivationRequested(tc.pid, tc.pidSet, tc.fds, tc.fdsSet, 42)
			if got != tc.want || (err != nil) != tc.wantErr {
				t.Fatalf("systemdActivationRequested() = %v, %v; want %v, error=%v", got, err, tc.want, tc.wantErr)
			}
		})
	}
}

func TestSystemdTCPTransportRejectsNonTCPListener(t *testing.T) {
	socketPath := filepath.Join(t.TempDir(), "mcpx.sock")
	original, err := net.Listen("unix", socketPath)
	if err != nil {
		t.Fatal(err)
	}
	defer original.Close()

	file, err := original.(*net.UnixListener).File()
	if err != nil {
		t.Fatal(err)
	}
	defer file.Close()

	if ln, err := systemdTCPListenerFromFile(file); err == nil {
		_ = ln.Close()
		t.Fatal("systemd TCP listener accepted a Unix socket")
	}
}

func TestParseTSNetEndpoint(t *testing.T) {
	cases := []struct {
		endpoint string
		host     string
		port     int
		ok       bool
		wantErr  bool
	}{
		{"tsnet://laptop.tailnet.ts.net", "laptop.tailnet.ts.net", TSNetPort, true, false},
		{"tsnet://laptop.tailnet.ts.net:8443", "laptop.tailnet.ts.net", 8443, true, false},
		{"tsnet://", "", 0, true, true},
		{"tsnet://host:notaport", "", 0, true, true},
		{"http://127.0.0.1:8065", "", 0, false, false},
		{"unix:///tmp/mcpx.sock", "", 0, false, false},
		{"", "", 0, false, false},
	}
	for _, tc := range cases {
		host, port, ok, err := ParseTSNetEndpoint(tc.endpoint)
		if ok != tc.ok || host != tc.host || port != tc.port || (err != nil) != tc.wantErr {
			t.Errorf("ParseTSNetEndpoint(%q) = %q, %d, %v, %v; want %q, %d, %v, err=%v",
				tc.endpoint, host, port, ok, err, tc.host, tc.port, tc.ok, tc.wantErr)
		}
	}
}

// A tsnet listener with no auth key must fail with the reason, never
// quietly serve somewhere else: the whole point of the identity route
// is that it is the route the operator named.
func TestTSNetTransportRequiresAuthKey(t *testing.T) {
	_, err := tsnetTransport{hostname: "mcpx-daemon", stateDir: t.TempDir()}.Listen()
	if err == nil || !strings.Contains(err.Error(), TSNetAuthKeyEnv) {
		t.Fatalf("Listen without auth key = %v; want an error naming %s", err, TSNetAuthKeyEnv)
	}
}

func TestTSNetDialerRequiresAuthKey(t *testing.T) {
	d := &TSNetDialer{hostname: "mcpx-client"}
	_, err := d.DialContext(context.Background(), "tcp", "laptop.tailnet.ts.net:80")
	if err == nil || !strings.Contains(err.Error(), TSNetAuthKeyEnv) {
		t.Fatalf("Dial without auth key = %v; want an error naming %s", err, TSNetAuthKeyEnv)
	}
}

// The tsnet node state directory is per-daemon, keyed by the
// configuration file set: two daemons sharing one state directory
// must not load each other's node keys (Codex finding on PR #2203),
// and one daemon's directory must not move when its configuration is
// reloaded or handed over -- the fingerprint is over the file set,
// not the contents.
func TestTSNetStateDirKeyedByConfig(t *testing.T) {
	state := t.TempDir()
	mk := func(sources ...string) *Server {
		s := &Server{paths: Paths{State: state}}
		if sources != nil {
			s.cfg = &config.Config{Sources: sources}
		}
		return s
	}
	a := mk("/proj-a/.mcpx.json")
	b := mk("/proj-b/.mcpx.json")
	if a.tsnetStateDir() == b.tsnetStateDir() {
		t.Fatalf("two configurations share a tsnet state dir: %s", a.tsnetStateDir())
	}
	if again := mk("/proj-a/.mcpx.json"); again.tsnetStateDir() != a.tsnetStateDir() {
		t.Fatalf("same configuration, different tsnet state dir: %s vs %s", again.tsnetStateDir(), a.tsnetStateDir())
	}
	want := filepath.Join(state, "tsnet")
	if got := mk().tsnetStateDir(); got != want {
		t.Fatalf("no configuration: got %s, want the unkeyed %s", got, want)
	}
}
