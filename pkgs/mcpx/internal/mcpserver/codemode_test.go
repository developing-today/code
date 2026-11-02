package mcpserver_test

import (
	"context"
	"encoding/json"
	"errors"
	"net/http"
	"net/http/httptest"
	"strings"
	"testing"
	"time"

	"github.com/developing-today/code/pkgs/mcpx/internal/mcpserver"
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

// postQueryModern is postQuery for a 2026-07-28 request, which the HTTP
// transport requires the protocol headers of (modernHeaders).
func postQueryModern(s *mcpserver.Server, query, method, name string, body []byte) map[string]any {
	r := httptest.NewRequest(http.MethodPost, "/mcp?"+query, strings.NewReader(string(body)))
	r.Header.Set("Content-Type", "application/json")
	for k, v := range modernHeaders(method, name) {
		r.Header.Set(k, v)
	}
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

// A raw call that becomes a task still runs on the raw surface: the task
// store runs the body on a fresh background context, and the raw-surface
// selection the request carried has to be carried into it explicitly, or
// the inner call is judged against the gateway surface and a validated
// raw name is refused with -32602.
func TestCodemodeFalseTaskCallKeepsTheRawSurface(t *testing.T) {
	b := &rawBackend{fakeBackend: newBackend()}
	s := rawServer(b)
	m := postQueryModern(s, "codemode=false", "tools/call", "db_query", frame(1, "tools/call",
		modernWith(extCaps, map[string]any{
			"name": "db_query", "arguments": map[string]any{}})))
	if errCode(m) != 0 {
		t.Fatalf("task-run raw tools/call db_query: %v", m)
	}
	res, _ := m["result"].(map[string]any)
	content, _ := res["content"].([]any)
	if len(content) == 0 || content[0].(map[string]any)["text"] != "called db.query" {
		t.Fatalf("task result = %v, want the db upstream's answer", m)
	}
}

// rawCaptureAsker records what RawToolOf reports for the name a call
// carries, then declines: the call runs the ordinary way.
type rawCaptureAsker struct {
	ns, tool string
	known    bool
	began    int
}

func (a *rawCaptureAsker) Begin(ctx context.Context, _ string, params json.RawMessage) (string, error) {
	a.began++
	var p struct {
		Name string `json:"name"`
	}
	_ = json.Unmarshal(params, &p)
	a.ns, a.tool, a.known = mcpserver.RawToolOf(ctx, p.Name)
	return "", mcpserver.ErrNotInterruptible
}

func (a *rawCaptureAsker) Poll(context.Context, string, time.Duration) (mcpserver.Outcome, error) {
	return mcpserver.Outcome{Done: true, Text: "unused"}, nil
}

func (a *rawCaptureAsker) Reply(context.Context, string, map[string]json.RawMessage) error {
	return nil
}

func (a *rawCaptureAsker) Abandon(string) {}

// The Asker bridge resolves a raw name from the request's own raw lookup:
// db_query is the db upstream's query tool. A call that did not ask for
// the raw surface reports known=false, so the bridge cannot mistake a
// gateway or pass-through name for a raw one.
func TestRawToolOfResolvesRawNamesForTheAsker(t *testing.T) {
	asker := &rawCaptureAsker{}
	b := &rawBackend{fakeBackend: newBackend()}
	s := rawServer(b)
	s.Ask = asker

	// Control: an ordinary mcpx_call, no raw surface in sight.
	s.Handle(context.Background(), mcpserver.Request(1, "tools/call", modernCall(1, nil)))
	if asker.began != 1 || asker.known {
		t.Fatalf("mcpx_call: began=%d known=%v, want a begun call that is not raw",
			asker.began, asker.known)
	}

	m := postQueryModern(s, "codemode=false", "tools/call", "db_query", frame(2, "tools/call",
		modernWith(`{"elicitation":{"form":{}}}`, map[string]any{
			"name": "db_query", "arguments": map[string]any{}})))
	if asker.began != 2 || !asker.known || asker.ns != "db" || asker.tool != "query" {
		t.Fatalf("raw call: began=%d known=%v ns=%q tool=%q, want db/query known",
			asker.began, asker.known, asker.ns, asker.tool)
	}
	if errCode(m) != 0 {
		t.Fatalf("raw tools/call db_query after the asker declined: %v", m)
	}
}

// A raw-surface request advertises tools.listChanged (capabilities already
// applied the raw condition), so a subscriptions/listen on that request
// must agree to toolsListChanged and the notifier must be asked for tool
// changes -- the pass-through case is
// TestPassThroughToolsListChangedReachesListenStreams.
func TestRawToolsListChangedReachesListenStreams(t *testing.T) {
	n := &fakeNotifier{got: make(chan mcpserver.ListenFilter, 4), fire: make(chan [2]any, 4)}
	s := rawServer(&rawBackend{fakeBackend: newBackend()})
	s.Notify = n
	s.SetPush(func(string, any) {})

	m := postQuery(s, "codemode=false", frame(1, "initialize",
		map[string]any{"protocolVersion": "2025-11-25"}))
	b, _ := json.Marshal(m)
	if !strings.Contains(string(b), `"listChanged":true`) {
		t.Fatalf("a raw request should be promised tools.listChanged: %s", b)
	}

	rec := &recorder{}
	c := s.ConnWithSend("sess-raw-tools", rec.send)
	s.HandleOn(mcpserver.WithRawForTest(context.Background()), c,
		mcpserver.Request(7, "subscriptions/listen",
			modernParams(map[string]any{"notifications": map[string]any{"toolsListChanged": true}})))
	ack := rec.wait(t, 1)[0]
	agreed, _ := ack["params"].(map[string]any)["notifications"].(map[string]any)
	if agreed["toolsListChanged"] != true {
		t.Fatalf("toolsListChanged not agreed on a raw listen: %v", ack)
	}
	select {
	case lf := <-n.got:
		if !lf.ToolsListChanged {
			t.Fatalf("the notifier was not asked for tool changes: %+v", lf)
		}
	case <-time.After(2 * time.Second):
		t.Fatal("no notifier started")
	}
	n.fire <- [2]any{"notifications/tools/list_changed", map[string]any{}}
	if f := rec.wait(t, 2)[1]; f["method"] != "notifications/tools/list_changed" {
		t.Fatalf("%v", f)
	}
}

// The legacy delivery path for the same promise: a GET stream opened with
// ?codemode=false starts the legacy notifier with ToolsListChanged set,
// where a plain GET stream (TestLegacyGETStream) starts it without.
func TestRawToolsListChangedReachesTheLegacyGETStream(t *testing.T) {
	n := &fakeNotifier{got: make(chan mcpserver.ListenFilter, 4), fire: make(chan [2]any, 4)}
	s := rawServer(&rawBackend{fakeBackend: newBackend()})
	s.Notify = n
	s.Timing.SSEKeepAlive = 20 * time.Millisecond
	ts := httptest.NewServer(s)
	defer ts.Close()

	id := legacySession(t, s, "2025-11-25")
	ctx, cancel := context.WithCancel(context.Background())
	defer cancel()
	resp, _ := openGET(t, ts.URL+"?codemode=false", id, ctx)
	defer resp.Body.Close()
	if resp.StatusCode != http.StatusOK {
		t.Fatalf("GET stream status %d", resp.StatusCode)
	}
	lf := listened(t, n)
	if !lf.ToolsListChanged {
		t.Fatalf("a raw GET stream was not promised tool changes: %+v", lf)
	}
}

// rawTaskBackend is the raw surface with execution.taskSupport
// declarations on it: db_job requires a task, db_nap allows one, db_query
// declares nothing.
type rawTaskBackend struct {
	*rawBackend
}

func (b *rawTaskBackend) RawTools(context.Context) ([]mcpserver.NamespacedTool, error) {
	mk := func(name, support string) mcpserver.Tool {
		t := mcpserver.Tool{Name: name, InputSchema: json.RawMessage(`{"type":"object"}`)}
		if support != "" {
			t.Execution = json.RawMessage(`{"taskSupport":"` + support + `"}`)
		}
		return t
	}
	return []mcpserver.NamespacedTool{
		{Namespace: "db", Tool: mk("job", "required")},
		{Namespace: "db", Tool: mk("nap", "optional")},
		{Namespace: "db", Tool: mk("query", "")},
	}, nil
}

func rawTaskServer() *mcpserver.Server {
	return mcpserver.New(&rawTaskBackend{rawBackend: &rawBackend{fakeBackend: newBackend()}}, "mcpx", "test")
}

// The task-support lookups consult the raw definitions: a modern client
// that did not declare the tasks extension calling a raw tool whose
// taskSupport is "required" gets -32021 naming the extension, exactly as
// a pass-through tool's declaration is honoured. Before, taskSupportOf
// resolved only pass-through tools, so the raw declaration was ignored
// and the call ran in line.
func TestCodemodeFalseRequiredRawToolWithoutTheExtensionIsMissingCapability(t *testing.T) {
	s := rawTaskServer()
	m := postQueryModern(s, "codemode=false", "tools/call", "db_job", frame(1, "tools/call",
		modernWith(`{}`, map[string]any{"name": "db_job", "arguments": map[string]any{}})))
	if errCode(m) != -32021 {
		t.Fatalf("raw required-task tool without the extension: want -32021, got %v", m)
	}
}

// The legacy lookup too: a 2025-11-25 task-augmented call to a raw tool
// declaring taskSupport "optional" becomes a task, while one declaring
// nothing stays forbidden (-32601) -- before, both were refused because
// legacyTaskSupport never saw the raw catalogue.
func TestCodemodeFalseLegacyTaskAugmentedRawCalls(t *testing.T) {
	s := rawTaskServer()
	m := postQuery(s, "codemode=false", frame(1, "tools/call",
		map[string]any{"name": "db_nap", "arguments": map[string]any{},
			"task": map[string]any{"ttl": 60000}}))
	res, _ := m["result"].(map[string]any)
	task, _ := res["task"].(map[string]any)
	if errCode(m) != 0 || task["taskId"] == nil {
		t.Fatalf("task-augmented raw db_nap: want a task handle, got %v", m)
	}

	m = postQuery(s, "codemode=false", frame(2, "tools/call",
		map[string]any{"name": "db_query", "arguments": map[string]any{},
			"task": map[string]any{"ttl": 60000}}))
	if errCode(m) != -32601 {
		t.Fatalf("task-augmented raw db_query (no taskSupport): want -32601, got %v", m)
	}
}

// upstreamAsker finishes every call at once with an upstream protocol
// error, the way the question bridge reports a call whose upstream
// answered with a JSON-RPC error.
type upstreamAsker struct {
	up *mcpserver.UpstreamError
}

func (a *upstreamAsker) Begin(context.Context, string, json.RawMessage) (string, error) {
	return "call-1", nil
}

func (a *upstreamAsker) Poll(context.Context, string, time.Duration) (mcpserver.Outcome, error) {
	return mcpserver.Outcome{Done: true, IsError: true, Text: a.up.Error(), Upstream: a.up}, nil
}

func (a *upstreamAsker) Reply(context.Context, string, map[string]json.RawMessage) error {
	return nil
}

func (a *upstreamAsker) Abandon(string) {}

// A raw call whose upstream answers through the question bridge with a
// JSON-RPC error is relayed as that error, as a pass-through call's is:
// before, passCall recognised only pass-through names, so the error came
// back as a successful tools/call result carrying isError and the client
// lost the code and its data.
func TestCodemodeFalseAskBridgeRelaysUpstreamProtocolErrors(t *testing.T) {
	s := rawServer(&rawBackend{fakeBackend: newBackend()})
	s.Ask = &upstreamAsker{up: &mcpserver.UpstreamError{
		Code: -32021, Message: "Missing required client capability",
		Data: json.RawMessage(`{"requiredCapabilities":{"sampling":{}}}`)}}
	m := postQueryModern(s, "codemode=false", "tools/call", "db_query", frame(1, "tools/call",
		modernWith(`{"elicitation":{"form":{}}}`, map[string]any{
			"name": "db_query", "arguments": map[string]any{}})))
	if errCode(m) != -32021 {
		t.Fatalf("raw call with an upstream protocol error: want -32021 relayed, got %v", m)
	}
}
