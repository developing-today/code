package daemon

import (
	"context"
	"net"
	"net/http"

	"github.com/developing-today/code/pkgs/mcpx/internal/api"
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
// both serving paths (startHTTP and Handler). It makes sure the request
// context carries a principal: the connection's own from ConnContext
// where a listener accepted it, the explicitly stamped one on the
// in-process path, or the request's best-effort answer otherwise. The
// permission decision itself is recorded by route, before the handler
// runs — recording it here, after dispatch, would leave long-lived
// handlers (GET /v1/events streams until the client disconnects)
// unrecorded for their whole lifetime, and unrecorded forever if the
// daemon stops first. Requests matching no declared operation (the
// dashboard, hand-written routes) get a principal but no decision —
// there is no declared permission to check them against.
func (s *Server) authorize(next http.Handler) http.Handler {
	return http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		p := principalForRequest(r)
		r = r.WithContext(api.WithPrincipal(r.Context(), p))
		next.ServeHTTP(w, r)
	})
}

// route registers one API route, recording the permission decision for
// the operation its pattern declares before the handler runs. Patterns
// with no declared operation (the MCP surface, hand-written routes)
// register plainly: a principal but no decision, exactly as before.
func (s *Server) route(mux *http.ServeMux, pattern string, h func(http.ResponseWriter, *http.Request)) {
	op, ok := api.ByRoute(pattern)
	if !ok {
		mux.HandleFunc(pattern, h)
		return
	}
	mux.HandleFunc(pattern, func(w http.ResponseWriter, r *http.Request) {
		if p, ok := api.PrincipalFrom(r.Context()); ok {
			api.Authorize(p, op)
		}
		h(w, r)
	})
}

// withPrincipal stamps every request with the given principal before it
// reaches the wrapped handler. It is how a serving path whose callers
// are known by construction states them explicitly instead of letting
// the request's address imply them.
func withPrincipal(p api.Principal, next http.Handler) http.Handler {
	return http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		next.ServeHTTP(w, r.WithContext(api.WithPrincipal(r.Context(), p)))
	})
}
