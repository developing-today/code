// Package sdnotify reports daemon state to systemd over $NOTIFY_SOCKET, the
// datagram protocol described in sd_notify(3). It is a few lines of socket
// code, which is why it is here rather than a dependency.
package sdnotify

import (
	"context"
	"fmt"
	"net"
	"os"
	"time"
)

const notifyTimeout = time.Second

type unixgramDialer interface {
	DialContext(context.Context, string, string) (net.Conn, error)
}

// Send writes one state assignment, such as "READY=1", to $NOTIFY_SOCKET.
// Outside systemd the variable is unset and Send does nothing.
func Send(state string) error {
	path := os.Getenv("NOTIFY_SOCKET")
	if path == "" {
		return nil
	}
	// DialContext accepts Go's leading-@ convention for abstract sockets and
	// bounds connection setup as well as the write below.
	return sendWithDialer(path, state, &net.Dialer{}, notifyTimeout)
}

func sendWithDialer(path, state string, dialer unixgramDialer, timeout time.Duration) error {
	ctx, cancel := context.WithTimeout(context.Background(), timeout)
	defer cancel()
	conn, err := dialer.DialContext(ctx, "unixgram", path)
	if err != nil {
		return fmt.Errorf("notify %s: %w", path, err)
	}
	defer func() { _ = conn.Close() }()
	if err := conn.SetWriteDeadline(time.Now().Add(timeout)); err != nil {
		return fmt.Errorf("notify %s: set write deadline: %w", path, err)
	}
	if _, err := conn.Write([]byte(state)); err != nil {
		return fmt.Errorf("notify %s: %w", path, err)
	}
	return nil
}

// Ready reports that the daemon is serving.
func Ready() error { return Send("READY=1") }

// MainPID makes pid the service's main process and reports it ready. A
// process that takes over a unit this way must be in the unit's cgroup and
// the unit must allow it to notify (NotifyAccess=all).
func MainPID(pid int) error {
	return Send(fmt.Sprintf("MAINPID=%d\nREADY=1", pid))
}
