//go:build linux

package daemon

import (
	"net"

	"golang.org/x/sys/unix"
)

// peerUID reads the uid of the process on the other end of a unix socket
// from its peer credentials (SO_PEERCRED). The kernel has always offered
// this; mcpx simply never looked (#1968).
func peerUID(c *net.UnixConn) (uint32, bool) {
	rc, err := c.SyscallConn()
	if err != nil {
		return 0, false
	}
	var uid uint32
	var ok bool
	err = rc.Control(func(fd uintptr) {
		cred, err := unix.GetsockoptUcred(int(fd), unix.SOL_SOCKET, unix.SO_PEERCRED)
		if err == nil {
			uid, ok = cred.Uid, true
		}
	})
	if err != nil {
		return 0, false
	}
	return uid, ok
}
