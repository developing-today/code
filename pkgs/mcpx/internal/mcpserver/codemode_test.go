package mcpserver_test

import (
	"context"
	"encoding/json"
	"errors"
	"net/http"
	"net/http/httptest"
	"strings"
	"testing"

	"github.com/dezren39/mcpx/internal/mcpserver"
)

// rawBackend is fakeBackend plus every upstream's tools: two namespaces,
// one raw name ("greet") offered by both, which is exactly the case the
// namespaced spelling exists for.
type rawBackend struct {
	*fakeBackend
	down error
}

func (b *rawBackend) RawTools(context.Context) ([]mcpserver.NamespacedTool, error) {
	if b.down != nil {
		return nil, b.down
	}
	mk := func(name, desc string) mcpserver.Tool {
		return mcpserver.Tool{Name: name, Description: desc,
			InputSchema: json.RawMessage(`{"type":"object"}`)}
	}
	return []mcpserver.NamespacedTool{
		{Namespace: "up", Tool: mk("greet", "says hello")},
		{Namespace: "db", Tool: mk("greet", "greets from the database")},
		{Namespace: "db", Tool: mk("query", "runs a query")},
	}, nil
}

func rawServer(b *rawBackend) *mcpserver.Server { return mcpserver.New(b, "mcpx", "test") }

// postQuery answers one legacy POST to /mcp at the given query string.
func postQuery(s *mcpserver.Server, query string, body []byte) map[string]any {
	r := httptest.NewRequest(http.MethodPost, "/mcp?"+query, strings.NewReader(string(body)))
	r.Header.Set("Content-Type", "application/json")
	w := httptest.NewRecorder()
	s.ServeHTTP(w, r)
	var m map[string]any
	if err := json.Unmarshal(w.Body.Bytes(), &m); err != nil {
		return map[string]any{"_unparsed": w.Body.String()}
	}
	return m
}

func toolNames(m map[string]any) []string {
	res, _ := m["result"].(map[string]any)
	tools, _ := res["tools"].([]any)
	var names []string
	for _, t := range tools {
		if tm, ok := t.(map[string]any); ok {
			names = append(names, tm["name"].(string))
		}
	}
	return names
}

// PLG-35 (#1940): ?codemode=false on /mcp exposes the raw upstream tools,
// namespaced, instead of the code-mode surface.
func TestCodemodeFalseListsRawToolsInsteadOfTheGatewaySurface(t *testing.T) {
	s := rawServer(&rawBackend{fakeBackend: newBackend()})
	m := postQuery(s, "codemode=false", frame(1, "tools/list", map[string]any{}))
	if errCode(m) != 0 {
		t.Fatalf("tools/list with codemode=false: %v", m)
	}
	got := strings.Join(toolNames(m), ",")
	want := "db_greet,db_query,up_greet"
	if got != want {
		t.Fatalf("raw tools/list = %q, want %q", got, want)
	}
}

// Without the parameter -- and with any other value -- the surface is the
// code-mode surface as it has always been.
func TestCodemodeAbsentOrOtherValuesKeepTheGatewaySurface(t *testing.T) {
	s := rawServer(&rawBackend{fakeBackend: newBackend()})
	for _, q := range []string{"", "codemode=true", "codemode=1", "codemode=FALSE"} {
		m := postQuery(s, q, frame(1, "tools/list", map[string]any{}))
		names := toolNames(m)
		joined := strings.Join(names, ",")
		if !strings.Contains(joined, "mcpx_exec") {
			t.Errorf("query %q: gateway surface lost mcpx_exec: %v", q, names)
		}
		if strings.Contains(joined, "up_greet") {
			t.Errorf("query %q: raw tools leaked into the gateway surface: %v", q, names)
		}
	}
}

// A raw name calls its upstream under the upstream's own name.
func TestCodemodeFalseCallReachesTheUpstream(t *testing.T) {
	b := &rawBackend{fakeBackend: newBackend()}
	s := rawServer(b)
	m := postQuery(s, "codemode=false", frame(1, "tools/call",
		map[string]any{"name": "db_query", "arguments": map[string]any{}}))
	if errCode(m) != 0 {
		t.Fatalf("tools/call db_query with codemode=false: %v", m)
	}
	res, _ := m["result"].(map[string]any)
	content, _ := res["content"].([]any)
	if len(content) == 0 || content[0].(map[string]any)["text"] != "called db.query" {
		t.Fatalf("call result = %v, want the db upstream's answer", m)
	}
}

// The gateway's own tools are not on the raw surface, and cannot be
// called through it either.
func TestCodemodeFalseCannotCallGatewayTools(t *testing.T) {
	s := rawServer(&rawBackend{fakeBackend: newBackend()})
	m := postQuery(s, "codemode=false", frame(1, "tools/call",
		map[string]any{"name": "mcpx_exec", "arguments": map[string]any{"source": "1"}}))
	if errCode(m) != -32602 {
		t.Fatalf("tools/call mcpx_exec with codemode=false: want -32602, got %v", m)
	}
}

// A backend that cannot list every upstream, and an upstream that is not
// answering, are errors -- never a partial or empty list a client would
// read as "these servers have no tools".
func TestCodemodeFalseWithAnUnlistableBackendIsAnError(t *testing.T) {
	s := mcpserver.New(newBackend(), "mcpx", "test") // fakeBackend: no RawTools
	m := postQuery(s, "codemode=false", frame(1, "tools/list", map[string]any{}))
	if errCode(m) != -32603 {
		t.Fatalf("tools/list, backend cannot list raw tools: want -32603, got %v", m)
	}

	s = rawServer(&rawBackend{fakeBackend: newBackend(), down: errors.New("connection refused")})
	m = postQuery(s, "codemode=false", frame(1, "tools/list", map[string]any{}))
	if errCode(m) != -32603 {
		t.Fatalf("tools/list, upstream down: want -32603, got %v", m)
	}
}
