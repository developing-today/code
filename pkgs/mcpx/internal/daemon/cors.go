package daemon

import "net/http"

// CORS for allow-listed browser origins (#1876 stage 3, "Run on my
// machine").
//
// refuseBrowserPages (origin.go) decides which browser origins may use
// the daemon at all: an Origin that is not loopback and not in
// transport.allowedOrigins is refused. Passing that check was never
// enough for a browser to actually use the answer -- without CORS
// headers the browser hides every response from the page, and the
// generated client's JSON POST dies at preflight because nothing
// answers OPTIONS. This layer supplies exactly that protocol, and
// only for origins the operator has already allow-listed:
//
//   - A preflight (OPTIONS with Access-Control-Request-Method) from
//     an allow-listed origin is answered here, before the bearer and
//     authorisation gates -- a preflight carries no credentials by
//     design, so gating it on them would fail every browser call.
//   - Any other request from an allow-listed origin is served as
//     usual, with Access-Control-Allow-Origin stamped on the way out
//     (error responses included, so the page can read the daemon's
//     own error text instead of a bare "fetch failed").
//
// No origin is allow-listed by this code: the set is exactly
// s.Origins (transport.allowedOrigins plus loopback), unchanged. A
// request with no Origin, or one refuseBrowserPages will refuse,
// passes through untouched. The recorded surface choice (#1876,
// #1890): build on the existing shared allow-list rather than a
// second docs-only one; the docs opt-in lives on the page.
func (s *Server) corsForAllowedOrigins(next http.Handler) http.Handler {
	return http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		origin := r.Header.Get("Origin")
		if origin == "" || !s.Origins.Allows(origin) {
			next.ServeHTTP(w, r)
			return
		}
		if r.Method == http.MethodOptions &&
			r.Header.Get("Access-Control-Request-Method") != "" {
			h := w.Header()
			h.Set("Access-Control-Allow-Origin", origin)
			h.Set("Access-Control-Allow-Methods",
				r.Header.Get("Access-Control-Request-Method"))
			if hdrs := r.Header.Get("Access-Control-Request-Headers"); hdrs != "" {
				h.Set("Access-Control-Allow-Headers", hdrs)
			}
			h.Set("Access-Control-Max-Age", "600")
			h.Add("Vary", "Origin")
			h.Add("Vary", "Access-Control-Request-Method")
			h.Add("Vary", "Access-Control-Request-Headers")
			w.WriteHeader(http.StatusNoContent)
			return
		}
		next.ServeHTTP(&corsResponseWriter{ResponseWriter: w, origin: origin}, r)
	})
}

// corsResponseWriter stamps the CORS headers just before the first
// write, so every response -- success or error -- carries them.
type corsResponseWriter struct {
	http.ResponseWriter
	origin string
	wrote  bool
}

func (w *corsResponseWriter) stamp() {
	if w.wrote {
		return
	}
	w.wrote = true
	h := w.Header()
	h.Set("Access-Control-Allow-Origin", w.origin)
	h.Add("Vary", "Origin")
}

func (w *corsResponseWriter) WriteHeader(code int) {
	w.stamp()
	w.ResponseWriter.WriteHeader(code)
}

func (w *corsResponseWriter) Write(b []byte) (int, error) {
	w.stamp()
	return w.ResponseWriter.Write(b)
}

// Unwrap lets http.ResponseController reach the underlying writer
// (flushing for streamed responses, deadlines, hijacking), so
// wrapping a handler here cannot silently change its capabilities.
func (w *corsResponseWriter) Unwrap() http.ResponseWriter { return w.ResponseWriter }
