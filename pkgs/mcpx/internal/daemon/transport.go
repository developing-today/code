package daemon

import (
	"fmt"
	"net"
	"os"
	"strconv"

	"github.com/developing-today/code/pkgs/mcpx/internal/defaults"
)

// Transport produces one of the daemon's listeners. The daemon's accept
// path used to be two hardcoded net.Listen calls in Server.Listen; naming
// the seam is what lets an identity transport (tsnet, transport_tsnet.go)
// serve the same HTTP API without the daemon learning anything about how
// the listener was reached (#1967: reach a daemon by identity, not by
// address). A Transport that cannot listen returns a clear error — there
// is deliberately no fallback to another transport, because silently
// serving on a different route than the one configured would be worse
// than failing.
type Transport interface {
	// Name identifies the transport in logs and status ("unix", "tcp",
	// "tsnet").
	Name() string
	// Listen binds the transport's listener.
	Listen() (net.Listener, error)
}

// unixTransport serves the local socket: no network stack, and the
// filesystem permissions are the access control.
type unixTransport struct {
	socket string
}

func (t unixTransport) Name() string { return "unix" }

func (t unixTransport) Listen() (net.Listener, error) {
	ln, err := net.Listen("unix", t.socket)
	if err != nil {
		return nil, fmt.Errorf("listen %s: %w", t.socket, err)
	}
	if err := os.Chmod(t.socket, defaults.PrivateMode); err != nil {
		_ = ln.Close()
		return nil, err
	}
	return ln, nil
}

// tcpTransport serves a host:port address. The caller decides the host;
// Listen in server.go keeps the loopback-by-default rule, because the API
// is unauthenticated and the network it is on is the access control.
type tcpTransport struct {
	address string
}

func (t tcpTransport) Name() string { return "tcp" }

func (t tcpTransport) Listen() (net.Listener, error) {
	if ln, activated, err := systemdTCPListener(); activated || err != nil {
		return ln, err
	}
	ln, err := net.Listen("tcp", t.address)
	if err != nil {
		return nil, fmt.Errorf("listen tcp: %w", err)
	}
	return ln, nil
}

// systemdTCPListener adopts systemd's first passed socket (fd 3) when this
// process is the activation target. net.FileListener duplicates the fd, so
// close the os.File wrapper after converting it to a Go listener.
func systemdTCPListener() (net.Listener, bool, error) {
	pid, pidSet := os.LookupEnv("LISTEN_PID")
	fdsValue, fdsSet := os.LookupEnv("LISTEN_FDS")
	activated, err := systemdActivationRequested(pid, pidSet, fdsValue, fdsSet, os.Getpid())
	if err != nil {
		return nil, true, err
	}
	if !activated {
		return nil, false, nil
	}
	if err := os.Unsetenv("LISTEN_PID"); err != nil {
		return nil, true, fmt.Errorf("clear LISTEN_PID: %w", err)
	}
	if err := os.Unsetenv("LISTEN_FDS"); err != nil {
		return nil, true, fmt.Errorf("clear LISTEN_FDS: %w", err)
	}
	file := os.NewFile(uintptr(3), "systemd-tcp-listener")
	if file == nil {
		return nil, true, fmt.Errorf("systemd listener fd 3 is unavailable")
	}
	defer file.Close()
	ln, err := systemdTCPListenerFromFile(file)
	if err != nil {
		return nil, true, err
	}
	return ln, true, nil
}

func systemdActivationRequested(pidValue string, pidSet bool, fdsValue string, fdsSet bool, pid int) (bool, error) {
	if !pidSet && !fdsSet {
		return false, nil
	}
	if !pidSet || !fdsSet {
		return false, fmt.Errorf("incomplete systemd activation environment: LISTEN_PID and LISTEN_FDS are both required")
	}
	parsedPID, err := strconv.Atoi(pidValue)
	if err != nil || parsedPID <= 0 {
		return false, fmt.Errorf("invalid LISTEN_PID %q", pidValue)
	}
	if parsedPID != pid {
		return false, nil
	}
	fds, err := strconv.Atoi(fdsValue)
	if err != nil || fds != 1 {
		return false, fmt.Errorf("invalid LISTEN_FDS %q: mcpx requires exactly one TCP listener", fdsValue)
	}
	return true, nil
}

func systemdTCPListenerFromFile(file *os.File) (net.Listener, error) {
	ln, err := net.FileListener(file)
	if err != nil {
		return nil, fmt.Errorf("adopt systemd listener: %w", err)
	}
	if _, ok := ln.(*net.TCPListener); !ok {
		_ = ln.Close()
		return nil, fmt.Errorf("systemd listener is %s, want TCP", ln.Addr().Network())
	}
	return ln, nil
}
