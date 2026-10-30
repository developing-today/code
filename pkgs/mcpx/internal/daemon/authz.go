package daemon

import (
	"context"
	"net"
	"net/http"

	"github.com/dezren39/mcpx/internal/api"
)

// The authorisation funnel (#1968): every surface constructs a principal,
// the principal travels in the request context, and each request that
// matches a declared operation has its permission decision recorded by
// api.Authorize. The stage is permissive — the check always answers yes
// and the daemon behaves exactly as it did when the API was simply
// unauthenticated — but the vocabulary is populated on every request, so
// a future policy constrains something real instead of something
// invented at enforcement time.
//
// The model the vocabulary documents, unchanged from before it existed:
// the API is unauthenticated; the unix socket's file permissions are the
// access control on that surface, and on TCP the network the listener is
// bound to is. Binding wider than loopback stays a deliberate act (see
// Listen). What is new is only that who-is-calling and what-would-be-
// required now have names.

// connPrincipal is the http.Server ConnContext hook: it builds the
// principal for a connection from the transport that accepted it, while
// the connection's own type is still visible — by the time a request
// exists, an HTTP request has forgotten which listener it arrived on.
func connPrincipal(ctx context.Context, c net.Conn) context.Context {
	return api.WithPrincipal(ctx, principalForConn(c))
}

// principalForConn constructs the principal a connection's surface can
// honestly offer: the peer uid for a unix socket (peer credentials, read
// for the first time — they were always available), the remote address
// for TCP, anonymous for anything else.
func principalForConn(c net.Conn) api.Principal {
	switch v := unwrapConn(c).(type) {
	case *net.UnixConn:
		if uid, ok := peerUID(v); ok {
			return api.UnixPeerPrincipal(uid)
		}
		return api.UnixPrincipal()
	case *net.TCPConn:
		return api.TCPPrincipal(v.RemoteAddr().String())
	default:
		return api.AnonymousPrincipal()
	}
}

// unwrapConn looks through the daemon's own connection wrappers (the
// handoff connection a predecessor passed on) to the transport
// connection underneath.
func unwrapConn(c net.Conn) net.Conn {
	for {
		if hc, ok := c.(*handoffConn); ok {
			c = hc.Conn
			continue
		}
		return c
	}
}

// principalForRequest constructs the principal for a request that did
// not arrive through a listener this process owns — Handler() used
// in-process, tests, a hosting application. No connection was ever
// accepted, so the caller is the process itself or reachable only by
// address; both are recorded honestly as such.
func principalForRequest(r *http.Request) api.Principal {
	if p, ok := api.PrincipalFrom(r.Context()); ok {
		return p
	}
	if r.RemoteAddr == "" {
		return api.InProcessPrincipal()
	}
	return api.TCPPrincipal(r.RemoteAddr)
}

// authorize is the middleware half of the funnel, wrapping the mux in
// both serving paths (startHTTP and Handler). Before dispatch it makes
// sure the request context carries a principal; after dispatch, when
// the mux has named the route it matched, it records the permission
// decision for the operation that route declares. Requests matching no
// declared operation (the dashboard, hand-written routes) get a
// principal but no decision — there is no declared permission to check
// them against.
func (s *Server) authorize(next http.Handler) http.Handler {
	return http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		p := principalForRequest(r)
		r = r.WithContext(api.WithPrincipal(r.Context(), p))
		next.ServeHTTP(w, r)
		if op, ok := api.ByRoute(r.Pattern); ok {
			api.Authorize(p, op)
		}
	})
}
