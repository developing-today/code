package pool_test

import (
	"context"
	"encoding/json"
	"fmt"
	"net/http"
	"net/http/httptest"
	"os"
	"path/filepath"
	"strings"
	"sync"
	"testing"
	"time"

	"github.com/developing-today/code/pkgs/mcpx/internal/config"
	"github.com/developing-today/code/pkgs/mcpx/internal/mcpclient"
	"github.com/developing-today/code/pkgs/mcpx/internal/pool"
	"github.com/developing-today/code/pkgs/mcpx/internal/testsupport"
)

// selfUpstream is a legacy MCP endpoint that identifies as mcpx
// itself and serves mcpx's aggregate surface; its catalogue grows by
// one proxied tool per list -- the unbounded growth #1877 names, in
// miniature.
type selfUpstream struct {
	*httptest.Server
	mu        sync.Mutex
	toolLists int
	// instance is reported as serverInfo.instance when non-empty.
	instance string
}

func newSelfUpstream(t *testing.T) *selfUpstream {
	t.Helper()
	u := &selfUpstream{}
	u.Server = httptest.NewServer(http.HandlerFunc(u.serve))
	t.Cleanup(u.Close)
	return u
}

func (u *selfUpstream) lists() int {
	u.mu.Lock()
	defer u.mu.Unlock()
	return u.toolLists
}

func (u *selfUpstream) growingTools(w http.ResponseWriter, id json.RawMessage) {
	u.mu.Lock()
	u.toolLists++
	n := u.toolLists
	u.mu.Unlock()
	tools := []map[string]any{{
		"name": "mcpx_namespaces", "description": "mcpx's own meta tool",
		"inputSchema": map[string]any{"type": "object"},
	}}
	for i := 0; i < n; i++ {
		tools = append(tools, map[string]any{
			"name":        fmt.Sprintf("proxied_%d", i),
			"inputSchema": map[string]any{"type": "object"},
		})
	}
	rpcReply(w, id, map[string]any{"tools": tools})
}

func (u *selfUpstream) serve(w http.ResponseWriter, r *http.Request) {
	var req struct {
		ID     json.RawMessage `json:"id"`
		Method string          `json:"method"`
	}
	_ = json.NewDecoder(r.Body).Decode(&req)
	switch req.Method {
	case "server/discover":
		rpcError(w, req.ID, -32601, "method not found")
	case "initialize":
		rpcReply(w, req.ID, map[string]any{
			"protocolVersion": "2025-03-26",
			"capabilities":    map[string]any{"tools": map[string]any{}},
			"serverInfo":      map[string]any{"name": "mcpx", "version": "dev", "instance": u.instance},
		})
	case "notifications/initialized":
		w.WriteHeader(http.StatusAccepted)
	case "tools/list":
		u.growingTools(w, req.ID)
	default:
		rpcReply(w, req.ID, map[string]any{})
	}
}

// A server entry pointing at mcpx's own MCP endpoint is refused at
// start: identity plus aggregate surface convict it, so none of its
// tools are ever admitted, however often the pool tries (#1877).
func TestPoolRefusesSelfConnectionByIdentity(t *testing.T) {
	u := newSelfUpstream(t)
	p := upstreamPool(t, u.URL)
	for attempt := 0; attempt < 3; attempt++ {
		_, err := p.Acquire(context.Background(), "global")
		if err == nil {
			t.Fatalf("attempt %d: self-connection was admitted", attempt)
		}
		if !strings.Contains(err.Error(), "self-connection") {
			t.Fatalf("attempt %d: error does not name the guard: %v", attempt, err)
		}
	}
	if tools, _, err := p.RefreshSchemas(context.Background()); err == nil || len(tools) != 0 {
		t.Fatalf("refresh of a self entry: tools=%v err=%v, want no tools and an error", tools, err)
	}
	if cached, _, _ := p.CachedSchemas(); len(cached) != 0 {
		t.Fatalf("self entry left %d cached tools advertised", len(cached))
	}
}

// modernSelfUpstream is the same self endpoint speaking the modern
// era: identity arrives in the server/discover _meta, not initialize.
type modernSelfUpstream struct {
	*httptest.Server
	mu        sync.Mutex
	toolLists int
}

func newModernSelfUpstream(t *testing.T) *modernSelfUpstream {
	t.Helper()
	u := &modernSelfUpstream{}
	u.Server = httptest.NewServer(http.HandlerFunc(u.serve))
	t.Cleanup(u.Close)
	return u
}

func (u *modernSelfUpstream) serve(w http.ResponseWriter, r *http.Request) {
	var req struct {
		ID     json.RawMessage `json:"id"`
		Method string          `json:"method"`
	}
	_ = json.NewDecoder(r.Body).Decode(&req)
	switch req.Method {
	case "server/discover":
		rpcReply(w, req.ID, map[string]any{
			"resultType":        "complete",
			"supportedVersions": []string{"2026-07-28"},
			"capabilities":      map[string]any{"tools": map[string]any{}},
			"_meta": map[string]any{
				"io.modelcontextprotocol/serverInfo": map[string]any{"name": "mcpx", "version": "dev"},
			},
		})
	case "tools/list":
		u.mu.Lock()
		u.toolLists++
		u.mu.Unlock()
		rpcReply(w, req.ID, map[string]any{"tools": []map[string]any{{
			"name": "mcpx_call", "description": "mcpx's own meta tool",
			"inputSchema": map[string]any{"type": "object"},
		}}})
	default:
		rpcReply(w, req.ID, map[string]any{})
	}
}

// The guard must fire on the modern handshake too: that is the
// default path, and its identity never passes through initialize.
func TestPoolRefusesModernSelfConnection(t *testing.T) {
	u := newModernSelfUpstream(t)
	p := upstreamPool(t, u.URL)
	_, err := p.Acquire(context.Background(), "global")
	if err == nil || !strings.Contains(err.Error(), "self-connection") {
		t.Fatalf("modern self-connection: err=%v, want a self-connection refusal", err)
	}
}

// namesakeUpstream identifies as "mcpx" but serves only its own
// declared tools -- an adapter or OpenAPI declaration that happens to
// be named mcpx. It cannot proxy mcpx's catalogue, so it is admitted.
type namesakeUpstream struct{ *httptest.Server }

func newNamesakeUpstream(t *testing.T) *namesakeUpstream {
	t.Helper()
	u := &namesakeUpstream{}
	u.Server = httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		var req struct {
			ID     json.RawMessage `json:"id"`
			Method string          `json:"method"`
		}
		_ = json.NewDecoder(r.Body).Decode(&req)
		switch req.Method {
		case "server/discover":
			rpcError(w, req.ID, -32601, "method not found")
		case "initialize":
			rpcReply(w, req.ID, map[string]any{
				"protocolVersion": "2025-03-26",
				"capabilities":    map[string]any{"tools": map[string]any{}},
				"serverInfo":      map[string]any{"name": "mcpx", "version": "dev"},
			})
		case "notifications/initialized":
			w.WriteHeader(http.StatusAccepted)
		case "tools/list":
			rpcReply(w, req.ID, map[string]any{"tools": []map[string]any{{
				"name": "frobnicate", "description": "the adapter's one tool",
				"inputSchema": map[string]any{"type": "object"},
			}}})
		default:
			rpcReply(w, req.ID, map[string]any{})
		}
	}))
	t.Cleanup(u.Close)
	return u
}

func TestPoolAdmitsNamesakeWithoutAggregateSurface(t *testing.T) {
	u := newNamesakeUpstream(t)
	p := upstreamPool(t, u.URL)
	tools, _, err := p.RefreshSchemas(context.Background())
	if err != nil {
		t.Fatalf("namesake upstream refused: %v", err)
	}
	if len(tools) != 1 || tools[0].Name != "frobnicate" {
		t.Fatalf("namesake tools = %v, want just frobnicate", tools)
	}
}

// An installation upgraded with a self entry already in its schema
// cache must stop advertising the proxied catalogue once the guard
// fires: the refusal drops the cached schemas (#1877).
func TestPoolRefusalDropsCachedSchemas(t *testing.T) {
	u := newSelfUpstream(t)
	p := upstreamPool(t, u.URL)
	p.SetSchemas([]mcpclient.Tool{{Name: "mcpx_namespaces"}, {Name: "proxied_0"}},
		nil, nil, "", time.Now(), time.Time{}, false)
	if cached, _, _ := p.CachedSchemas(); len(cached) != 2 {
		t.Fatalf("seeded cache not in place: %d tools", len(cached))
	}
	if _, err := p.Acquire(context.Background(), "global"); err == nil {
		t.Fatal("self-connection was admitted over a seeded cache")
	}
	if cached, _, _ := p.CachedSchemas(); len(cached) != 0 {
		t.Fatalf("refusal left %d cached tools advertised", len(cached))
	}
}

// reactiveSelfUpstream identifies as mcpx and serves only the
// reactive surface's activation pair (request_tools + batch_call):
// a genuine aggregate in --mode reactive before any tool has been
// activated. Its mcpx_* tools exist but are not yet listed.
type reactiveSelfUpstream struct{ *httptest.Server }

func newReactiveSelfUpstream(t *testing.T) *reactiveSelfUpstream {
	t.Helper()
	u := &reactiveSelfUpstream{}
	u.Server = httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		var req struct {
			ID     json.RawMessage `json:"id"`
			Method string          `json:"method"`
		}
		_ = json.NewDecoder(r.Body).Decode(&req)
		switch req.Method {
		case "server/discover":
			rpcError(w, req.ID, -32601, "method not found")
		case "initialize":
			rpcReply(w, req.ID, map[string]any{
				"protocolVersion": "2025-03-26",
				"capabilities":    map[string]any{"tools": map[string]any{}},
				"serverInfo":      map[string]any{"name": "mcpx", "version": "dev"},
			})
		case "notifications/initialized":
			w.WriteHeader(http.StatusAccepted)
		case "tools/list":
			rpcReply(w, req.ID, map[string]any{"tools": []map[string]any{
				{"name": "request_tools", "inputSchema": map[string]any{"type": "object"}},
				{"name": "batch_call", "inputSchema": map[string]any{"type": "object"}},
			}})
		default:
			rpcReply(w, req.ID, map[string]any{})
		}
	}))
	t.Cleanup(u.Close)
	return u
}

// The guard must convict a reactive-mode aggregate on its activation
// pair, independently of the currently activated tools (Codex review
// on PR 2975): request_tools can activate the mcpx_* surface later,
// and refreshes on a reused instance never re-run the start check.
func TestPoolRefusesReactiveSelfConnection(t *testing.T) {
	u := newReactiveSelfUpstream(t)
	p := upstreamPool(t, u.URL)
	_, err := p.Acquire(context.Background(), "global")
	if err == nil || !strings.Contains(err.Error(), "self-connection") {
		t.Fatalf("reactive self-connection: err=%v, want a self-connection refusal", err)
	}
}

// A daemon upgraded while a self entry is already connected hands
// the live session to its successor: the guard must run on the
// adopted session before it is published, not only on fresh starts
// (Codex review on PR 2975).
func TestPoolRefusesAdoptedSelfSession(t *testing.T) {
	u := newSelfUpstream(t)
	p := upstreamPool(t, u.URL)
	p.SetSchemas([]mcpclient.Tool{{Name: "mcpx_namespaces"}, {Name: "proxied_0"}},
		nil, nil, "", time.Now(), time.Time{}, false)
	h := pool.InstanceHandoff{
		ID: "inst-self", Key: "global",
		HTTP: &mcpclient.HTTPHandoff{SessionID: "sess-1", Negotiated: "2025-03-26"},
		Session: mcpclient.Session{Era: mcpclient.EraLegacy, Negotiated: "2025-03-26",
			ServerInfo: mcpclient.ServerInfo{Name: "mcpx", Version: "dev"}},
	}
	ads, err := p.AdoptSessions([]pool.InstanceHandoff{h})
	if err != nil {
		t.Fatal(err)
	}
	p.Attach(ads, 1)
	if cached, _, _ := p.CachedSchemas(); len(cached) != 0 {
		t.Fatalf("adopted self session left %d cached tools advertised", len(cached))
	}
	if _, err := p.Acquire(context.Background(), "global"); err == nil ||
		!strings.Contains(err.Error(), "self-connection") {
		t.Fatalf("adopted self session was published: err=%v, want a self-connection refusal", err)
	}
}

// A DIFFERENT mcpx daemon is a peer, not a loop (#1877): its instance
// identity differs from this process's, so the guard admits it even
// though it serves the aggregate surface -- mcpx as another mcpx's
// upstream is the topology the modern-client e2e test exercises.
func TestPoolAdmitsDistinctMcpxPeerByInstance(t *testing.T) {
	u := newSelfUpstream(t)
	u.instance = "peer-instance"
	pool.OwnInstanceID = "own-instance"
	defer func() { pool.OwnInstanceID = "" }()
	p := upstreamPool(t, u.URL)
	lease, err := p.Acquire(context.Background(), "global")
	if err != nil {
		t.Fatalf("distinct mcpx peer was refused: %v", err)
	}
	lease.Release()
}

// An upstream reporting THIS process's instance identity is the
// daemon connected to itself: refused on identity alone, before its
// tools are ever listed.
func TestPoolRefusesOwnInstanceByIdentity(t *testing.T) {
	u := newSelfUpstream(t)
	u.instance = "own-instance"
	pool.OwnInstanceID = "own-instance"
	defer func() { pool.OwnInstanceID = "" }()
	p := upstreamPool(t, u.URL)
	_, err := p.Acquire(context.Background(), "global")
	if err == nil || !strings.Contains(err.Error(), "self-connection") {
		t.Fatalf("own instance admitted or wrong error: %v", err)
	}
	if n := u.lists(); n != 0 {
		t.Fatalf("identity refusal listed tools %d times; want 0 (identity decides first)", n)
	}
}

func newFakeDaemon(t *testing.T, instance string) *httptest.Server {
	t.Helper()
	s := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		w.Header().Set("Content-Type", "application/json")
		switch r.URL.Path {
		case "/v1/health":
			_ = json.NewEncoder(w).Encode(map[string]any{"ok": true})
		case "/v1/version":
			_ = json.NewEncoder(w).Encode(map[string]any{"version": "dev"})
		case "/v1/recipes":
			_ = json.NewEncoder(w).Encode(map[string]any{"recipes": []any{}})
		case "/v1/status":
			_ = json.NewEncoder(w).Encode(map[string]any{"instance": instance})
		case "/v1/events":
			w.WriteHeader(http.StatusNoContent)
		default:
			_ = json.NewEncoder(w).Encode(map[string]any{})
		}
	}))
	t.Cleanup(s.Close)
	return s
}

func writeShimConfig(t *testing.T, endpoint string) string {
	t.Helper()
	dir := t.TempDir()
	cfgPath := filepath.Join(dir, "mcpx.json")
	content := fmt.Sprintf(`{"mcpServers":{},"daemon":{"endpoint":%q}}`, endpoint)
	if err := os.WriteFile(cfgPath, []byte(content), 0o600); err != nil {
		t.Fatal(err)
	}
	return cfgPath
}

func stdioPool(t *testing.T, cmd string, endpoint string, args ...string) *pool.Pool {
	t.Helper()
	cfg := &config.Config{MCPServers: map[string]*config.Server{
		"shim": {
			Name:    "shim",
			Command: cmd,
			Args:    args,
			Env:     map[string]string{"MCPX_DAEMON_ENDPOINT": endpoint},
		},
	}}
	r, err := cfg.Resolve("shim")
	if err != nil {
		t.Fatal(err)
	}
	p := pool.New(r)
	t.Cleanup(p.Close)
	return p
}

// An mcpx serve stdio shim adopts its backing daemon's instance identity
// (serve.go CmdServe via GET /v1/status), not the shim process's own
// fresh ID (#1877, Codex P1 on PR 2975).
// Regression: verify a stdio shim backed by the same daemon is refused by
// the self-connection guard, while a shim backed by a distinct daemon is
// admitted as a distinct peer.
func TestPoolStdioShimBackingIdentity(t *testing.T) {
	mcpx := testsupport.MCPXBinary(t)
	pool.OwnInstanceID = "daemon-alpha"
	defer func() { pool.OwnInstanceID = "" }()

	// 1. Shim backed by the same daemon ("daemon-alpha"):
	// The stdio shim must adopt "daemon-alpha", matching pool.OwnInstanceID.
	// The pool must refuse it as a self-connection.
	sameDaemon := newFakeDaemon(t, "daemon-alpha")
	sameCfg := writeShimConfig(t, sameDaemon.URL)
	samePool := stdioPool(t, mcpx, sameDaemon.URL, "--config", sameCfg, "serve")
	_, err := samePool.Acquire(context.Background(), "global")
	if err == nil || !strings.Contains(err.Error(), "self-connection") {
		t.Fatalf("stdio shim backed by same daemon was admitted: err=%v, want self-connection refusal", err)
	}

	// 2. Shim backed by a distinct daemon ("daemon-beta"):
	// The stdio shim must adopt "daemon-beta", distinct from pool.OwnInstanceID.
	// The pool must admit it as a distinct peer.
	diffDaemon := newFakeDaemon(t, "daemon-beta")
	diffCfg := writeShimConfig(t, diffDaemon.URL)
	diffPool := stdioPool(t, mcpx, diffDaemon.URL, "--config", diffCfg, "serve")
	lease, err := diffPool.Acquire(context.Background(), "global")
	if err != nil {
		t.Fatalf("stdio shim backed by distinct daemon was refused: %v", err)
	}
	lease.Release()
}
