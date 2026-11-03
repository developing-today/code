package daemon

import (
	"context"
	"encoding/json"
	"fmt"
	"net/http"
	"net/http/httptest"
	"strings"
	"sync"
	"testing"

	"github.com/developing-today/code/pkgs/mcpx/internal/config"
	"github.com/developing-today/code/pkgs/mcpx/internal/testsupport"
)

// growingSelf is an HTTP MCP endpoint identifying as mcpx whose
// tools/list grows by one tool per call -- what an admitted
// self-connection would feed the catalogue on every reload (#1877).
type growingSelf struct {
	*httptest.Server
	mu    sync.Mutex
	lists int
}

func newGrowingSelf(t *testing.T) *growingSelf {
	t.Helper()
	g := &growingSelf{}
	g.Server = httptest.NewServer(http.HandlerFunc(g.serve))
	t.Cleanup(g.Close)
	return g
}

func (g *growingSelf) serve(w http.ResponseWriter, r *http.Request) {
	var req struct {
		ID     json.RawMessage `json:"id"`
		Method string          `json:"method"`
	}
	_ = json.NewDecoder(r.Body).Decode(&req)
	reply := func(result any) {
		w.Header().Set("Content-Type", "application/json")
		_ = json.NewEncoder(w).Encode(map[string]any{"jsonrpc": "2.0", "id": req.ID, "result": result})
	}
	switch req.Method {
	case "server/discover":
		w.Header().Set("Content-Type", "application/json")
		_ = json.NewEncoder(w).Encode(map[string]any{"jsonrpc": "2.0", "id": req.ID,
			"error": map[string]any{"code": -32601, "message": "method not found"}})
	case "initialize":
		reply(map[string]any{
			"protocolVersion": "2025-03-26",
			"capabilities":    map[string]any{"tools": map[string]any{}},
			"serverInfo":      map[string]any{"name": "mcpx", "version": "dev"},
		})
	case "notifications/initialized":
		w.WriteHeader(http.StatusAccepted)
	case "tools/list":
		g.mu.Lock()
		g.lists++
		n := g.lists
		g.mu.Unlock()
		tools := []map[string]any{{"name": "mcpx_namespaces",
			"inputSchema": map[string]any{"type": "object"}}}
		for i := 0; i < n; i++ {
			tools = append(tools, map[string]any{"name": fmt.Sprintf("proxied_%d", i),
				"inputSchema": map[string]any{"type": "object"}})
		}
		reply(map[string]any{"tools": tools})
	default:
		reply(map[string]any{})
	}
}

// Repeated reloads with a self entry present must not grow the
// catalogue: the self pool is refused by identity, so the only tools
// are the honest server's, identical after every reload (#1877).
func TestReloadWithSelfEntryDoesNotGrowTools(t *testing.T) {
	self := newGrowingSelf(t)
	fake := testsupport.FakeMCPBinary(t)
	dir := t.TempDir()
	cfgOf := func() *config.Config {
		return &config.Config{MCPServers: map[string]*config.Server{
			"self": {Name: "self", URL: self.URL},
			"fake": {Name: "fake", Command: fake},
		}}
	}
	r, err := NewRegistry(cfgOf(), Paths{State: dir, Cache: dir}, nil)
	if err != nil {
		t.Fatal(err)
	}
	defer r.Close()

	var want int
	for reload := 0; reload < 3; reload++ {
		errs := r.Warm(context.Background(), true)
		werr, ok := errs["self"]
		if !ok || !strings.Contains(werr.Error(), "self-connection") {
			t.Fatalf("reload %d: warm errors = %v, want a self-connection refusal for self", reload, errs)
		}
		tools := r.Tools(nil)
		for _, tl := range tools {
			if tl.Namespace == "self" {
				t.Fatalf("reload %d: self entry contributed tool %s", reload, tl.Function)
			}
		}
		if reload == 0 {
			want = len(tools)
			if want == 0 {
				t.Fatal("honest server contributed no tools; test would pass vacuously")
			}
		} else if len(tools) != want {
			t.Fatalf("reload %d: catalogue grew %d -> %d tools with a self entry present", reload, want, len(tools))
		}
		if _, _, err := r.Reload(cfgOf()); err != nil {
			t.Fatal(err)
		}
	}
	self.mu.Lock()
	defer self.mu.Unlock()
	// At most one identity-check list per warm cycle (3), and at
	// least one: the guard reads the surface to convict it, and never
	// a catalogue beyond that (later cycles may be served the refusal
	// from the pool's restart cooldown without redialling). While the
	// fake's own list grew on every call it did get, the registry
	// catalogue above never did.
	if self.lists < 1 || self.lists > 3 {
		t.Fatalf("self upstream was listed %d times, want between 1 and the 3 identity checks", self.lists)
	}
}
