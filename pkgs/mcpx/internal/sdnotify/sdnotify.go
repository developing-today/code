// Package sdnotify reports daemon state to systemd over $NOTIFY_SOCKET, the
// datagram protocol described in sd_notify(3). It is a few lines of socket
// code, which is why it is here rather than a dependency.
package sdnotify

import (
	"fmt"
	"net"
	"os"
	"time"
)

// writeTimeout bounds one notify write. sd_notify is fire-and-forget:
// a daemon must never wedge on an unresponsive socket (in some
// container environments an abstract unixgram write blocks instead of
// erroring — run 41819578987 hung 1800s there), so the write carries
// a deadline and the failure surfaces as an ordinary error.
const writeTimeout = 5 * time.Second

// Send writes one state assignment, such as "READY=1", to $NOTIFY_SOCKET.
// Outside systemd the variable is unset and Send does nothing.
func Send(state string) error {
	path := os.Getenv("NOTIFY_SOCKET")
	if path == "" {
		return nil
	}
	// A leading '@' names an abstract socket. Go's net package takes the same
	// convention, so the name is passed through unchanged.
	conn, err := net.DialUnix("unixgram", nil, &net.UnixAddr{Name: path, Net: "unixgram"})
	if err != nil {
		return fmt.Errorf("notify %s: %w", path, err)
	}
	defer func() { _ = conn.Close() }()
	_ = conn.SetWriteDeadline(time.Now().Add(writeTimeout))
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
