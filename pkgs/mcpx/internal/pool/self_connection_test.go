package pool_test

import (
	"context"
	"encoding/json"
	"fmt"
	"net/http"
	"net/http/httptest"
	"strings"
	"sync"
	"testing"
	"time"

	"github.com/dezren39/mcpx/internal/mcpclient"
)

// selfUpstream is a legacy MCP endpoint that identifies as mcpx
// itself and serves mcpx's aggregate surface; its catalogue grows by
// one proxied tool per list -- the unbounded growth #1877 names, in
// miniature.
type selfUpstream struct {
	*httptest.Server
	mu        sync.Mutex
	toolLists int
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
			"serverInfo":      map[string]any{"name": "mcpx", "version": "dev"},
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
