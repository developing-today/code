package pool_test

import (
	"net/http"
	"net/http/httptest"
	"strings"
	"sync"
	"testing"

	"github.com/developing-today/code/pkgs/mcpx/internal/config"
	"github.com/developing-today/code/pkgs/mcpx/internal/mcpauth"
	"github.com/developing-today/code/pkgs/mcpx/internal/pool"
	"github.com/developing-today/code/pkgs/mcpx/internal/testsupport"
)

// #240: a server's `auth` block was parsed and never applied, so a configured
// credential silently never reached the upstream.
func TestAuthReachesTheUpstream(t *testing.T) {
	var mu sync.Mutex
	var gotAuth, gotKey string
	var hits int
	srv := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		mu.Lock()
		hits++
		if gotAuth == "" {
			gotAuth = r.Header.Get("Authorization")
			gotKey = r.URL.Query().Get("key")
		}
		mu.Unlock()
		http.Error(w, "no", http.StatusInternalServerError)
	}))
	defer srv.Close()

	resolve := func(t *testing.T, a *mcpauth.Auth) *pool.Pool {
		t.Helper()
		cfg := &config.Config{MCPServers: map[string]*config.Server{
			"remote": {Name: "remote", URL: srv.URL + "/mcp", Auth: a},
		}}
		r, err := cfg.Resolve("remote")
		if err != nil {
			t.Fatal(err)
		}
		p := pool.New(r)
		t.Cleanup(p.Close)
		return p
	}

	t.Run("bearer token from the environment is sent", func(t *testing.T) {
		t.Setenv("MCPX_TEST_TOKEN", "s3cret")
		p := resolve(t, &mcpauth.Auth{Type: "bearer", Token: "${MCPX_TEST_TOKEN}"})
		_, _, _ = p.RefreshSchemas(ctx20(t))
		mu.Lock()
		defer mu.Unlock()
		if gotAuth != "Bearer s3cret" {
			t.Fatalf("Authorization = %q, want %q", gotAuth, "Bearer s3cret")
		}
	})

	t.Run("an unset variable fails before any request", func(t *testing.T) {
		mu.Lock()
		before := hits
		mu.Unlock()
		p := resolve(t, &mcpauth.Auth{Type: "bearer", Token: "${MCPX_TEST_UNSET_TOKEN}"})
		_, _, err := p.RefreshSchemas(ctx20(t))
		if err == nil || !strings.Contains(err.Error(), "MCPX_TEST_UNSET_TOKEN") {
			t.Fatalf("err = %v, want it to name MCPX_TEST_UNSET_TOKEN", err)
		}
		mu.Lock()
		defer mu.Unlock()
		if hits != before {
			t.Errorf("made %d request(s) with a credential known to be missing", hits-before)
		}
	})

	t.Run("query parameter is added to the URL", func(t *testing.T) {
		mu.Lock()
		gotAuth, gotKey = "", ""
		mu.Unlock()
		p := resolve(t, &mcpauth.Auth{Type: "query", Param: "key", ParamValue: "abc"})
		_, _, _ = p.RefreshSchemas(ctx20(t))
		mu.Lock()
		defer mu.Unlock()
		if gotKey != "abc" {
			t.Fatalf("query key = %q, want abc", gotKey)
		}
	})
}

// #1950: for a stdio server the authorization specification says to
// retrieve credentials from the environment rather than follow the
// OAuth flow ("Implementations using an STDIO transport SHOULD NOT
// follow this specification, and instead retrieve credentials from
// the environment"). This traces that path end to end: the credential
// exists only in the daemon's environment, the server's `env` auth
// block retrieves it, and the spawned child itself reports what it
// received. The reported name is set nowhere but the auth block, so
// ordinary environment inheritance cannot produce it.
func TestStdioChildReceivesEnvCredential(t *testing.T) {
	bin := testsupport.FakeMCPBinary(t)

	t.Run("a ${VAR} reference is expanded from the daemon's environment", func(t *testing.T) {
		t.Setenv("MCPX_TEST_STDIO_SOURCE", "retrieved-credential")
		r := resolvedWith(t, &config.Server{
			Command: bin,
			// The fake server answers `echo` with the value of the
			// named variable in its own environment.
			Env: map[string]string{"MCPX_REPORT_ENV": "MCPX_CHILD_CREDENTIAL"},
			Auth: &mcpauth.Auth{Type: "env", Env: map[string]string{
				"MCPX_CHILD_CREDENTIAL": "${MCPX_TEST_STDIO_SOURCE}",
			}},
		})
		p := pool.New(r)
		t.Cleanup(p.Close)
		raw, err := p.Call(ctx20(t), config.ResolvedScope{Key: "global"}, "echo", map[string]any{"message": "x"})
		if err != nil {
			t.Fatalf("call: %v", err)
		}
		if got := textOf(t, raw); got != "retrieved-credential" {
			t.Fatalf("child reports %q, want %q (the credential held in the daemon's environment)", got, "retrieved-credential")
		}
	})

	t.Run("an unset variable fails before the child spawns", func(t *testing.T) {
		r := resolvedWith(t, &config.Server{
			Command: bin,
			Auth: &mcpauth.Auth{Type: "env", Env: map[string]string{
				"MCPX_CHILD_CREDENTIAL": "${MCPX_TEST_STDIO_UNSET}",
			}},
		})
		p := pool.New(r)
		t.Cleanup(p.Close)
		_, err := p.Call(ctx20(t), config.ResolvedScope{Key: "global"}, "echo", map[string]any{"message": "x"})
		if err == nil || !strings.Contains(err.Error(), "MCPX_TEST_STDIO_UNSET") {
			t.Fatalf("err = %v, want it to name MCPX_TEST_STDIO_UNSET", err)
		}
	})
}
