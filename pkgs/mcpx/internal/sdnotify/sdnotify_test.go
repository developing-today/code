package sdnotify

import (
	"fmt"
	"net"
	"os"
	"path/filepath"
	"testing"
	"time"

	"github.com/developing-today/code/pkgs/mcpx/internal/testsupport"
)

func listen(t *testing.T, name string) *net.UnixConn {
	t.Helper()
	conn, err := net.ListenUnixgram("unixgram", &net.UnixAddr{Name: name, Net: "unixgram"})
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { _ = conn.Close() })
	return conn
}

func next(t *testing.T, conn *net.UnixConn) string {
	t.Helper()
	_ = conn.SetReadDeadline(time.Now().Add(5 * time.Second))
	buf := make([]byte, 256)
	n, _, err := conn.ReadFromUnix(buf)
	if err != nil {
		t.Fatalf("no datagram: %v", err)
	}
	return string(buf[:n])
}

func TestSendIsANoOpOutsideSystemd(t *testing.T) {
	t.Setenv("NOTIFY_SOCKET", "")
	if err := Ready(); err != nil {
		t.Fatal(err)
	}
}

func TestReadyAndMainPIDReachTheSocket(t *testing.T) {
	// Same short-directory requirement as every directly bound socket:
	// t.TempDir() under a long TMPDIR exceeds the sockaddr_un limit (#434).
	sock := filepath.Join(testsupport.SocketDir(t), "notify.sock")
	conn := listen(t, sock)
	t.Setenv("NOTIFY_SOCKET", sock)

	if err := Ready(); err != nil {
		t.Fatal(err)
	}
	if got := next(t, conn); got != "READY=1" {
		t.Fatalf("Ready sent %q", got)
	}
	if err := MainPID(4242); err != nil {
		t.Fatal(err)
	}
	if got := next(t, conn); got != "MAINPID=4242\nREADY=1" {
		t.Fatalf("MainPID sent %q", got)
	}
}

func TestAbstractSocketName(t *testing.T) {
	name := fmt.Sprintf("@mcpx-sdnotify-%d-%d", os.Getpid(), time.Now().UnixNano())
	conn := listen(t, name)
	t.Setenv("NOTIFY_SOCKET", name)

	if err := Ready(); err != nil {
		t.Fatal(err)
	}
	if got := next(t, conn); got != "READY=1" {
		t.Fatalf("Ready sent %q", got)
	}
}

func TestMissingSocketIsReported(t *testing.T) {
	t.Setenv("NOTIFY_SOCKET", filepath.Join(t.TempDir(), "absent.sock"))
	if err := Ready(); err == nil {
		t.Fatal("expected an error for a socket nobody listens on")
	}
}

// TestSendFailsFastWhenTheSocketIsNotDrained is the #5428 regression:
// a bound-but-never-read notify socket must not make Send block until
// the caller's own timeout (1800s in the fleet-build container, where
// a fresh sender's write to a full abstract-socket queue blocks). On
// kernels where a fresh sender's datagram is delivered despite the
// full queue, the blocking path cannot be exercised and the test
// skips with that reason.
func TestSendFailsFastWhenTheSocketIsNotDrained(t *testing.T) {
	name := fmt.Sprintf("@mcpx-sdnotify-full-%d-%d", os.Getpid(), time.Now().UnixNano())
	listen(t, name)
	t.Setenv("NOTIFY_SOCKET", name)

	// Fill the listener's receive queue: datagrams nobody reads.
	spam, err := net.DialUnix("unixgram", nil, &net.UnixAddr{Name: name, Net: "unixgram"})
	if err != nil {
		t.Fatal(err)
	}
	defer func() { _ = spam.Close() }()
	payload := make([]byte, 8192)
	for i := 0; i < 8192; i++ {
		_ = spam.SetWriteDeadline(time.Now().Add(100 * time.Millisecond))
		if _, err := spam.Write(payload); err != nil {
			break // queue full
		}
	}

	done := make(chan error, 1)
	go func() { done <- Ready() }()
	select {
	case err := <-done:
		if err == nil {
			t.Skip("kernel delivered a fresh sender's datagram despite the full queue; the blocking path is not exercisable here")
		}
		// An error is the pass: Send failed fast instead of hanging.
	case <-time.After(20 * time.Second):
		t.Fatal("Send blocked on an undrained socket past its deadline (#5428)")
	}
}
