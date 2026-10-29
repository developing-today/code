package daemon

import (
	"fmt"
	"net"
	"os"

	"github.com/dezren39/mcpx/internal/defaults"
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
		ln.Close()
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
	ln, err := net.Listen("tcp", t.address)
	if err != nil {
		return nil, fmt.Errorf("listen tcp: %w", err)
	}
	return ln, nil
}
