package daemon

import (
	"fmt"
	"net"
	"os"

	"golang.org/x/sys/unix"
)

// Confirm fd 3 is an Internet stream listener before taking ownership. An
// absent descriptor could have been reused by the Go runtime or Unix
// transport during startup; never close an unrelated descriptor.
func adoptSystemdTCP(address string) (net.Listener, error) {
	const fd = 3
	accepting, err := unix.GetsockoptInt(fd, unix.SOL_SOCKET, unix.SO_ACCEPTCONN)
	if err != nil {
		return nil, fmt.Errorf("systemd TCP activation: inspect fd 3: %w", err)
	}
	if accepting != 1 {
		return nil, fmt.Errorf("systemd TCP activation: fd 3 is not listening")
	}
	for _, check := range []struct {
		option int
		accept func(int) bool
	}{
		{unix.SO_DOMAIN, func(n int) bool { return n == unix.AF_INET || n == unix.AF_INET6 }},
		// Multipath TCP wraps TCP semantics; Go enables it for listeners by default.
		{unix.SO_PROTOCOL, func(n int) bool { return n == unix.IPPROTO_TCP || n == unix.IPPROTO_MPTCP }},
	} {
		value, err := unix.GetsockoptInt(fd, unix.SOL_SOCKET, check.option)
		if err != nil {
			return nil, fmt.Errorf("systemd TCP activation: inspect fd 3: %w", err)
		}
		if !check.accept(value) {
			return nil, fmt.Errorf("systemd TCP activation: fd 3 is not a TCP listener")
		}
	}
	// net.FileListener duplicates the verified descriptor. Close the
	// original even if conversion or address validation later fails.
	unix.CloseOnExec(fd)
	f := os.NewFile(fd, "systemd listener")
	defer func() { _ = f.Close() }()
	ln, err := net.FileListener(f)
	if err != nil {
		return nil, fmt.Errorf("systemd TCP activation: adopt fd 3: %w", err)
	}
	tcp, ok := ln.(*net.TCPListener)
	if !ok {
		_ = ln.Close()
		return nil, fmt.Errorf("systemd TCP activation: fd 3 is not a TCP listener")
	}
	want, err := net.ResolveTCPAddr("tcp", address)
	if err != nil {
		_ = tcp.Close()
		return nil, fmt.Errorf("systemd TCP activation: resolve configured address: %w", err)
	}
	got := tcp.Addr().(*net.TCPAddr)
	// An inherited listener must not broaden the unauthenticated API's
	// configured address. Port zero retains the normal ephemeral-port meaning.
	sameIP := got.IP.Equal(want.IP) || len(want.IP) == 0 && got.IP.IsUnspecified()
	if !sameIP || got.Zone != want.Zone || (want.Port != 0 && got.Port != want.Port) {
		_ = tcp.Close()
		return nil, fmt.Errorf("systemd TCP activation: listener %s does not match configured address %s", got, address)
	}
	// Do not advertise this process's activation to subprocesses.
	for _, name := range []string{"LISTEN_PID", "LISTEN_FDS", "LISTEN_FDNAMES"} {
		_ = os.Unsetenv(name)
	}
	return tcp, nil
}
