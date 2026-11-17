package daemon

import (
	"net/http"
	"net/http/httptest"
	"strings"
	"testing"

	"github.com/developing-today/code/pkgs/mcpx/internal/mcpserver"
)

// CORS for allow-listed origins (#1876 stage 3): the middleware unit
// behaviour, plus the one integration property the browser depends
// on -- a preflight is answered before the bearer gate, because a
// preflight carries no credentials.

const corsDocsOrigin = "https://docs.example.invalid"

func corsServer() *Server {
	return &Server{Origins: mcpserver.OriginPolicy{
		Origins: []string{corsDocsOrigin},
	}}
}

func TestCORSPreflightFromAllowListedOrigin(t *testing.T) {
	reached := false
	next := http.HandlerFunc(func(w http.ResponseWriter, _ *http.Request) {
		reached = true
		w.WriteHeader(http.StatusOK)
	})
	req := httptest.NewRequest(http.MethodOptions, "/v1/call", nil)
	req.Header.Set("Origin", corsDocsOrigin)
	req.Header.Set("Access-Control-Request-Method", "POST")
	req.Header.Set("Access-Control-Request-Headers", "content-type")
	rec := httptest.NewRecorder()
	corsServer().corsForAllowedOrigins(next).ServeHTTP(rec, req)
	if reached {
		t.Fatal("preflight reached the wrapped handler; it must be answered by the CORS layer")
	}
	if rec.Code != http.StatusNoContent {
		t.Fatalf("status %d, want 204", rec.Code)
	}
	if got := rec.Header().Get("Access-Control-Allow-Origin"); got != corsDocsOrigin {
		t.Errorf("Allow-Origin %q, want the requesting origin %q", got, corsDocsOrigin)
	}
	if got := rec.Header().Get("Access-Control-Allow-Methods"); got != "POST" {
		t.Errorf("Allow-Methods %q, want POST (the requested method)", got)
	}
	if got := rec.Header().Get("Access-Control-Allow-Headers"); got != "content-type" {
		t.Errorf("Allow-Headers %q, want content-type (the requested headers)", got)
	}
}

func TestCORSPreflightFromUnlistedOriginPassesThrough(t *testing.T) {
	reached := false
	next := http.HandlerFunc(func(w http.ResponseWriter, _ *http.Request) {
		reached = true
		w.WriteHeader(http.StatusOK)
	})
	req := httptest.NewRequest(http.MethodOptions, "/v1/call", nil)
	req.Header.Set("Origin", "https://evil.example.invalid")
	req.Header.Set("Access-Control-Request-Method", "POST")
	rec := httptest.NewRecorder()
	corsServer().corsForAllowedOrigins(next).ServeHTTP(rec, req)
	if !reached {
		t.Fatal("unlisted origin's preflight was intercepted; refuseBrowserPages owns that refusal, not this layer")
	}
	if got := rec.Header().Get("Access-Control-Allow-Origin"); got != "" {
		t.Errorf("Allow-Origin %q stamped for an unlisted origin, want none", got)
	}
}

func TestCORSActualRequestStampsAllowOrigin(t *testing.T) {
	next := http.HandlerFunc(func(w http.ResponseWriter, _ *http.Request) {
		w.WriteHeader(http.StatusTeapot)
		_, _ = w.Write([]byte("short and stout"))
	})
	req := httptest.NewRequest(http.MethodPost, "/v1/call", strings.NewReader(`{}`))
	req.Header.Set("Origin", corsDocsOrigin)
	rec := httptest.NewRecorder()
	corsServer().corsForAllowedOrigins(next).ServeHTTP(rec, req)
	if rec.Code != http.StatusTeapot {
		t.Fatalf("status %d, want the handler's 418", rec.Code)
	}
	if got := rec.Header().Get("Access-Control-Allow-Origin"); got != corsDocsOrigin {
		t.Errorf("Allow-Origin %q, want %q", got, corsDocsOrigin)
	}
	if got := rec.Header().Get("Vary"); !strings.Contains(got, "Origin") {
		t.Errorf("Vary %q, want it to name Origin", got)
	}
}

func TestCORSNoOriginNoHeaders(t *testing.T) {
	next := http.HandlerFunc(func(w http.ResponseWriter, _ *http.Request) {
		w.WriteHeader(http.StatusOK)
	})
	req := httptest.NewRequest(http.MethodPost, "/v1/call", strings.NewReader(`{}`))
	rec := httptest.NewRecorder()
	corsServer().corsForAllowedOrigins(next).ServeHTTP(rec, req)
	if got := rec.Header().Get("Access-Control-Allow-Origin"); got != "" {
		t.Errorf("Allow-Origin %q on a request with no Origin, want none", got)
	}
}

func TestCORSLoopbackOriginAllowedByPolicy(t *testing.T) {
	// OriginPolicy always allows loopback origins (local dashboards);
	// the CORS layer follows the policy, so a loopback page gets
	// headers too -- the same set refuseBrowserPages admits.
	next := http.HandlerFunc(func(w http.ResponseWriter, _ *http.Request) {
		w.WriteHeader(http.StatusOK)
	})
	req := httptest.NewRequest(http.MethodPost, "/v1/call", strings.NewReader(`{}`))
	req.Header.Set("Origin", "http://127.0.0.1:9999")
	rec := httptest.NewRecorder()
	corsServer().corsForAllowedOrigins(next).ServeHTTP(rec, req)
	if got := rec.Header().Get("Access-Control-Allow-Origin"); got != "http://127.0.0.1:9999" {
		t.Errorf("Allow-Origin %q, want the loopback origin", got)
	}
}

func TestCORSPreflightBypassesBearerGate(t *testing.T) {
	// The full Handler() chain with a bearer token configured: a
	// preflight (which never carries Authorization) must still be
	// answered 204, and an unauthenticated actual request must be a
	// 401 the browser is allowed to read (Allow-Origin stamped).
	s := &Server{
		set:     bearerSet(t, fixtureToken),
		Origins: mcpserver.OriginPolicy{Origins: []string{corsDocsOrigin}},
	}
	pre := httptest.NewRequest(http.MethodOptions, "/v1/call", nil)
	pre.Header.Set("Origin", corsDocsOrigin)
	pre.Header.Set("Access-Control-Request-Method", "POST")
	rec := httptest.NewRecorder()
	s.Handler().ServeHTTP(rec, pre)
	if rec.Code != http.StatusNoContent {
		t.Fatalf("preflight status %d, want 204 (body %q)", rec.Code, rec.Body.String())
	}
	post := httptest.NewRequest(http.MethodPost, "/v1/call", strings.NewReader(`{}`))
	post.Header.Set("Origin", corsDocsOrigin)
	rec2 := httptest.NewRecorder()
	s.Handler().ServeHTTP(rec2, post)
	if rec2.Code != http.StatusUnauthorized {
		t.Fatalf("unauthenticated POST status %d, want 401", rec2.Code)
	}
	if got := rec2.Header().Get("Access-Control-Allow-Origin"); got != corsDocsOrigin {
		t.Errorf("401 response Allow-Origin %q, want %q so the page can read the error", got, corsDocsOrigin)
	}
}
