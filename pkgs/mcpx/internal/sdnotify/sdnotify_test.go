package sdnotify

import (
	"context"
	"errors"
	"fmt"
	"net"
	"os"
	"path/filepath"
	"runtime"
	"syscall"
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
	if runtime.GOOS != "linux" {
		t.Skip("abstract Unix sockets are Linux-specific")
	}

	name := fmt.Sprintf("@mcpx-sdnotify-%d-%d", os.Getpid(), time.Now().UnixNano())
	conn, err := net.ListenUnixgram("unixgram", &net.UnixAddr{Name: name, Net: "unixgram"})
	if err != nil {
		if abstractSocketUnavailable(err) {
			t.Skipf("abstract Unix datagram sockets are unavailable here: %v", err)
		}
		t.Fatalf("listen on abstract socket: %v", err)
	}
	t.Cleanup(func() { _ = conn.Close() })
	t.Setenv("NOTIFY_SOCKET", name)

	if err := Ready(); err != nil {
		if abstractSocketUnavailable(err) {
			t.Skipf("abstract Unix datagram sockets are unavailable here: %v", err)
		}
		t.Fatal(err)
	}
	if got := next(t, conn); got != "READY=1" {
		t.Fatalf("Ready sent %q", got)
	}
}

func abstractSocketUnavailable(err error) bool {
	var networkErr net.Error
	if errors.As(err, &networkErr) && networkErr.Timeout() {
		return true
	}
	return errors.Is(err, syscall.EAFNOSUPPORT) ||
		errors.Is(err, syscall.EPROTONOSUPPORT) ||
		errors.Is(err, syscall.EOPNOTSUPP) ||
		errors.Is(err, syscall.ENOTSUP)
}

type timeoutError struct{}

func (timeoutError) Error() string   { return "timed out" }
func (timeoutError) Timeout() bool   { return true }
func (timeoutError) Temporary() bool { return true }

func TestAbstractSocketUnavailable(t *testing.T) {
	tests := []struct {
		name string
		err  error
		want bool
	}{
		{name: "timeout", err: timeoutError{}, want: true},
		{name: "address family unsupported", err: syscall.EAFNOSUPPORT, want: true},
		{name: "protocol unsupported", err: syscall.EPROTONOSUPPORT, want: true},
		{name: "operation unsupported", err: syscall.EOPNOTSUPP, want: true},
		{name: "not supported", err: syscall.ENOTSUP, want: true},
		{name: "invalid argument is unexpected", err: syscall.EINVAL},
		{name: "other error is unexpected", err: errors.New("unexpected socket error")},
	}
	for _, test := range tests {
		t.Run(test.name, func(t *testing.T) {
			if got := abstractSocketUnavailable(fmt.Errorf("wrapped: %w", test.err)); got != test.want {
				t.Fatalf("abstractSocketUnavailable(%v) = %v, want %v", test.err, got, test.want)
			}
		})
	}
}

type blockingDialer struct{}

func (blockingDialer) DialContext(ctx context.Context, _, _ string) (net.Conn, error) {
	<-ctx.Done()
	return nil, ctx.Err()
}

func TestSendDialHasDeadline(t *testing.T) {
	const timeout = 20 * time.Millisecond
	started := time.Now()
	err := sendWithDialer("@unused", "READY=1", blockingDialer{}, timeout)
	if !errors.Is(err, context.DeadlineExceeded) {
		t.Fatalf("sendWithDialer error = %v, want deadline exceeded", err)
	}
	if elapsed := time.Since(started); elapsed > 250*time.Millisecond {
		t.Fatalf("dial timeout took %s, want it bounded by %s", elapsed, timeout)
	}
}

type deadlineRecordingConn struct {
	net.Conn
	writeDeadline time.Time
	written       string
}

func (c *deadlineRecordingConn) SetWriteDeadline(deadline time.Time) error {
	c.writeDeadline = deadline
	return nil
}

func (c *deadlineRecordingConn) Write(p []byte) (int, error) {
	c.written = string(p)
	return len(p), nil
}

func (c *deadlineRecordingConn) Close() error { return nil }

type fixedDialer struct{ conn net.Conn }

func (d fixedDialer) DialContext(context.Context, string, string) (net.Conn, error) {
	return d.conn, nil
}

func TestSendWriteHasDeadline(t *testing.T) {
	conn := &deadlineRecordingConn{}
	started := time.Now()
	const timeout = time.Second
	if err := sendWithDialer("/unused", "READY=1", fixedDialer{conn: conn}, timeout); err != nil {
		t.Fatal(err)
	}
	finished := time.Now()
	// The deadline is set after dialing, at some instant between started
	// and finished, so it must fall in [started+timeout, finished+timeout].
	// Bounding it by timestamps captured around the call avoids assuming
	// the setup itself finishes within any fixed scheduling slack.
	if conn.writeDeadline.Before(started.Add(timeout)) || conn.writeDeadline.After(finished.Add(timeout)) {
		t.Fatalf("write deadline = %s, want between %s and %s", conn.writeDeadline, started.Add(timeout), finished.Add(timeout))
	}
	if conn.written != "READY=1" {
		t.Fatalf("write = %q, want READY=1", conn.written)
	}
}

func TestMissingSocketIsReported(t *testing.T) {
	t.Setenv("NOTIFY_SOCKET", filepath.Join(t.TempDir(), "absent.sock"))
	if err := Ready(); err == nil {
		t.Fatal("expected an error for a socket nobody listens on")
	}
}
