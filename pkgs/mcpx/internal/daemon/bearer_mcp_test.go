package daemon

import (
	"net/http"
	"net/http/httptest"
	"strings"
	"testing"
)

// #1950 conformance scenario (bound in internal/conformance as
// authz-accept-only-own-tokens): on a bearer-configured transport the
// daemon's own MCP surface rejects unauthenticated requests. The gate
// is requireBearer over the whole handler chain (bearer.go: every /v1
// and /mcp route); TestRequireBearer pins the gate on a /v1 route, so
// this drives the real Handler() mux with a stub MCP handler mounted
// the way production mounts it, and the /mcp route itself answers.
// Fixture tokens only -- never a real credential.
func TestMCPRejectsUnauthenticatedWhenBearerSet(t *testing.T) {
	cases := []struct {
		name    string
		set     string
		header  string
		want    int
		reached bool
	}{
		{"set, no header", fixtureToken, "", http.StatusUnauthorized, false},
		{"set, wrong token", fixtureToken, "Bearer wrong-token-fixture-0123456789", http.StatusUnauthorized, false},
		{"set, wrong scheme", fixtureToken, "Basic " + fixtureToken, http.StatusUnauthorized, false},
		{"set, correct token", fixtureToken, "Bearer " + fixtureToken, http.StatusOK, true},
		{"unset, no header", "", "", http.StatusOK, true},
		{"unset, stray header ignored", "", "Bearer stray-fixture-0123456789abcd", http.StatusOK, true},
	}
	for _, tc := range cases {
		t.Run(tc.name, func(t *testing.T) {
			reached := false
			mcp := http.HandlerFunc(func(w http.ResponseWriter, _ *http.Request) {
				reached = true
				w.WriteHeader(http.StatusOK)
			})
			s := &Server{set: bearerSet(t, tc.set), MCP: mcp}
			req := httptest.NewRequest(http.MethodPost, "/mcp", strings.NewReader(`{"jsonrpc":"2.0","id":1,"method":"ping"}`))
			if tc.header != "" {
				req.Header.Set("Authorization", tc.header)
			}
			rec := httptest.NewRecorder()
			s.Handler().ServeHTTP(rec, req)
			if rec.Code != tc.want {
				t.Fatalf("status %d, want %d (body %q)", rec.Code, tc.want, rec.Body.String())
			}
			if reached != tc.reached {
				t.Errorf("MCP handler reached = %v, want %v", reached, tc.reached)
			}
			if tc.want == http.StatusUnauthorized && rec.Header().Get("WWW-Authenticate") != "Bearer" {
				t.Errorf("401 without a WWW-Authenticate: Bearer header")
			}
		})
	}
}
