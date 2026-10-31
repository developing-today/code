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
)

// selfUpstream is an MCP endpoint that identifies as mcpx itself and,
// if its tools are ever listed, returns a catalogue that grows by one
// tool per list -- the unbounded proxy growth #1877 names, in miniature.
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
		u.mu.Lock()
		u.toolLists++
		n := u.toolLists
		u.mu.Unlock()
		tools := make([]map[string]any, 0, n)
		for i := 0; i < n; i++ {
			tools = append(tools, map[string]any{
				"name":        fmt.Sprintf("proxied_%d", i),
				"inputSchema": map[string]any{"type": "object"},
			})
		}
		rpcReply(w, req.ID, map[string]any{"tools": tools})
	default:
		rpcReply(w, req.ID, map[string]any{})
	}
}

// A server entry pointing at mcpx's own MCP endpoint is refused at
// start: the upstream's identity is mcpx, so none of its tools are
// ever read, however often the pool tries to start it (#1877).
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
	if got := u.lists(); got != 0 {
		t.Fatalf("tools/list reached the self upstream %d times; its growing catalogue was read", got)
	}
	if tools, _, err := p.RefreshSchemas(context.Background()); err == nil || len(tools) != 0 {
		t.Fatalf("refresh of a self entry: tools=%v err=%v, want no tools and an error", tools, err)
	}
}
