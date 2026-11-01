package cli

import (
	"context"
	"encoding/json"
	"io"
	"net/http"
	"net/http/httptest"
	"strings"
	"sync"
	"testing"

	"github.com/developing-today/code/pkgs/mcpx/internal/daemon"
	"github.com/developing-today/code/pkgs/mcpx/internal/mcpserver"
)

// rawAskBackend is an mcpserver.Backend that can list every upstream's
// tools, for the raw surface: one db upstream offering query. Its Call is
// the ordinary path -- a raw call the asker declines lands there, and the
// test can tell the two paths apart by which one answered.
type rawAskBackend struct {
	mu    sync.Mutex
	calls []string
}

func (b *rawAskBackend) Namespaces(context.Context) (string, error) { return "db", nil }
func (b *rawAskBackend) Catalog(context.Context, int, string) (string, error) {
	return "", nil
}
func (b *rawAskBackend) Types(context.Context, []string) (string, error) { return "", nil }
func (b *rawAskBackend) Search(context.Context, string, int) (string, error) {
	return "", nil
}
func (b *rawAskBackend) Call(_ context.Context, ns, tool string, _ json.RawMessage) (string, error) {
	b.mu.Lock()
	defer b.mu.Unlock()
	b.calls = append(b.calls, ns+"."+tool)
	return "called " + ns + "." + tool, nil
}
func (b *rawAskBackend) Exec(context.Context, string, int) (string, error) { return "", nil }
func (b *rawAskBackend) Log(context.Context, string, string, string, int) (string, error) {
	return "", nil
}
func (b *rawAskBackend) Stats(context.Context, string) (string, error) { return "", nil }
func (b *rawAskBackend) Status(context.Context) (string, error)        { return "{}", nil }
func (b *rawAskBackend) RegistrySearch(context.Context, string, int) (string, error) {
	return "", nil
}
func (b *rawAskBackend) Resources(context.Context) ([]mcpserver.ResourceRef, error) {
	return nil, nil
}
func (b *rawAskBackend) Prompts(context.Context) ([]mcpserver.PromptRef, error) {
	return nil, nil
}
func (b *rawAskBackend) ReadResource(context.Context, string) ([]mcpserver.ResourceContents, error) {
	return nil, nil
}
func (b *rawAskBackend) GetPrompt(context.Context, string, map[string]string) (string, error) {
	return "", nil
}
func (b *rawAskBackend) Complete(context.Context, json.RawMessage) ([]string, error) {
	return nil, nil
}
func (b *rawAskBackend) ResourceTemplates(context.Context) ([]mcpserver.ResourceRef, error) {
	return nil, nil
}
func (b *rawAskBackend) RawTools(context.Context) ([]mcpserver.NamespacedTool, error) {
	return []mcpserver.NamespacedTool{{Namespace: "db", Tool: mcpserver.Tool{
		Name: "query", InputSchema: json.RawMessage(`{"type":"object"}`)}}}, nil
}

// A raw tool call (codemode=false) from a client that can answer questions
// goes through the ask bridge as the upstream call it is -- server db,
// tool query -- so a question the upstream raises reaches the client with
// the call attributed. Before, daemonAsker.Begin recognised only
// pass-through names and gateway tools, answered ErrNotInterruptible for
// the namespaced raw name, and the call ran unattributed.
func TestDaemonAskerBeginsRawToolCalls(t *testing.T) {
	asked := make(chan map[string]any, 1)
	daemonSrv := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		switch {
		case r.URL.Path == "/v1/namespaces":
			_, _ = w.Write([]byte(`[{"namespace":"db","server":"db"}]`))
		case r.URL.Path == "/v1/ask" && r.Method == http.MethodPost:
			raw, _ := io.ReadAll(r.Body)
			var body map[string]any
			if err := json.Unmarshal(raw, &body); err != nil {
				t.Errorf("ask body: %v", err)
			}
			asked <- body
			_, _ = w.Write([]byte(`{"callId":"call-1"}`))
		case strings.HasPrefix(r.URL.Path, "/v1/ask/"):
			_, _ = w.Write([]byte(`{"done":true,"result":{"kind":"tools/call","server":"db",` +
				`"result":{"content":[{"type":"text","text":"asked db.query"}]}}}`))
		default:
			http.Error(w, `{"error":"unexpected `+r.Method+` `+r.URL.Path+`"}`, http.StatusNotFound)
		}
	}))
	defer daemonSrv.Close()

	app := &App{client: NewClientAt(daemon.Paths{}, "", daemonSrv.URL), connected: true}
	backend := &rawAskBackend{}
	srv := mcpserver.New(backend, "mcpx", "test")
	srv.Ask = daemonAsker{app: app}

	params := map[string]any{
		"name": "db_query", "arguments": map[string]any{"q": "select 1"},
		"_meta": map[string]any{
			mcpserver.MetaProtocolVersion:    "2026-07-28",
			mcpserver.MetaClientCapabilities: json.RawMessage(`{"elicitation":{"form":{}}}`),
		},
	}
	body, _ := json.Marshal(map[string]any{
		"jsonrpc": "2.0", "id": 1, "method": "tools/call", "params": params})
	req := httptest.NewRequest(http.MethodPost, "/mcp?codemode=false", strings.NewReader(string(body)))
	req.Header.Set("Content-Type", "application/json")
	req.Header.Set("MCP-Protocol-Version", "2026-07-28")
	req.Header.Set("Mcp-Method", "tools/call")
	req.Header.Set("Mcp-Name", "db_query")
	w := httptest.NewRecorder()
	srv.ServeHTTP(w, req)

	var body2 map[string]any
	select {
	case body2 = <-asked:
	default:
		t.Fatalf("the daemon was never asked to start the call; response: %s", w.Body.String())
	}
	if body2["server"] != "db" || body2["tool"] != "query" {
		t.Fatalf("ask body server/tool = %v/%v, want db/query: %v", body2["server"], body2["tool"], body2)
	}
	args, _ := json.Marshal(body2["args"])
	if !strings.Contains(string(args), "select 1") {
		t.Fatalf("ask body args = %s, want the call's arguments", args)
	}
	backend.mu.Lock()
	calls := append([]string(nil), backend.calls...)
	backend.mu.Unlock()
	if len(calls) != 0 {
		t.Fatalf("the call also ran the ordinary way: %v", calls)
	}
	if !strings.Contains(w.Body.String(), "asked db.query") {
		t.Fatalf("response should carry the asked call's result: %s", w.Body.String())
	}
}

// The same raw call, but the upstream tool's own arguments carry
// fields named namespace and tool with non-string values -- the
// upstream's schema, not mcpx_call's. Decoding mcpx_call's gateway
// fields before resolving the raw name used to fail on exactly this
// and drop the call to the unattributed ordinary path (Codex review
// of PR #2992, round 3).
func TestDaemonAskerBeginsRawToolCallsWithGatewayShapedArguments(t *testing.T) {
	asked := make(chan map[string]any, 1)
	daemonSrv := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		switch {
		case r.URL.Path == "/v1/namespaces":
			_, _ = w.Write([]byte(`[{"namespace":"db","server":"db"}]`))
		case r.URL.Path == "/v1/ask" && r.Method == http.MethodPost:
			raw, _ := io.ReadAll(r.Body)
			var body map[string]any
			if err := json.Unmarshal(raw, &body); err != nil {
				t.Errorf("ask body: %v", err)
			}
			asked <- body
			_, _ = w.Write([]byte(`{"callId":"call-1"}`))
		case strings.HasPrefix(r.URL.Path, "/v1/ask/"):
			_, _ = w.Write([]byte(`{"done":true,"result":{"kind":"tools/call","server":"db",` +
				`"result":{"content":[{"type":"text","text":"asked db.query"}]}}}`))
		default:
			http.Error(w, `{"error":"unexpected `+r.Method+` `+r.URL.Path+`"}`, http.StatusNotFound)
		}
	}))
	defer daemonSrv.Close()

	app := &App{client: NewClientAt(daemon.Paths{}, "", daemonSrv.URL), connected: true}
	backend := &rawAskBackend{}
	srv := mcpserver.New(backend, "mcpx", "test")
	srv.Ask = daemonAsker{app: app}

	params := map[string]any{
		"name": "db_query", "arguments": map[string]any{"namespace": 42, "tool": true, "q": "select 1"},
		"_meta": map[string]any{
			mcpserver.MetaProtocolVersion:    "2026-07-28",
			mcpserver.MetaClientCapabilities: json.RawMessage(`{"elicitation":{"form":{}}}`),
		},
	}
	body, _ := json.Marshal(map[string]any{
		"jsonrpc": "2.0", "id": 1, "method": "tools/call", "params": params})
	req := httptest.NewRequest(http.MethodPost, "/mcp?codemode=false", strings.NewReader(string(body)))
	req.Header.Set("Content-Type", "application/json")
	req.Header.Set("MCP-Protocol-Version", "2026-07-28")
	req.Header.Set("Mcp-Method", "tools/call")
	req.Header.Set("Mcp-Name", "db_query")
	w := httptest.NewRecorder()
	srv.ServeHTTP(w, req)

	var body2 map[string]any
	select {
	case body2 = <-asked:
	default:
		t.Fatalf("the daemon was never asked to start the call; response: %s", w.Body.String())
	}
	if body2["server"] != "db" || body2["tool"] != "query" {
		t.Fatalf("ask body server/tool = %v/%v, want db/query: %v", body2["server"], body2["tool"], body2)
	}
	args, _ := json.Marshal(body2["args"])
	if !strings.Contains(string(args), `"namespace":42`) || !strings.Contains(string(args), "select 1") {
		t.Fatalf("ask body args = %s, want the call's arguments", args)
	}
	backend.mu.Lock()
	calls := append([]string(nil), backend.calls...)
	backend.mu.Unlock()
	if len(calls) != 0 {
		t.Fatalf("the call also ran the ordinary way: %v", calls)
	}
	if !strings.Contains(w.Body.String(), "asked db.query") {
		t.Fatalf("response should carry the asked call's result: %s", w.Body.String())
	}
}
