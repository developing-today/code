package mcpserver_test

import (
	"encoding/json"
	"net/http"
	"net/http/httptest"
	"strings"
	"testing"
	"time"

	"github.com/developing-today/code/pkgs/mcpx/internal/mcpserver"
)

// Codex review on PR #9038 (issue #2435), finding 1: relay provenance
// must survive task execution. A peer error that reaches the client
// through a task -- eagerly, through tasks/result, or through an
// asked task -- is still a peer error: code, message and data exactly
// as the upstream sent them, and never stamped with mcpx's
// local-origin marker. Oracle: issue #2435's acceptance ("no relayed
// peer error carries the marker").

func relayedDataUnchanged(t *testing.T, m map[string]any) {
	t.Helper()
	e, _ := m["error"].(map[string]any)
	if e == nil {
		t.Fatalf("want an error response, got %v", m)
	}
	data, _ := e["data"].(map[string]any)
	if len(data) != 1 || data["peer"] != "data" {
		t.Fatalf("relayed error's data was altered or marked local: %v", e)
	}
}

func TestARelayedPeerErrorThroughAnEagerTaskKeepsItsData(t *testing.T) {
	up := &mcpserver.UpstreamError{Code: -32603, Message: "as designed",
		Data: json.RawMessage(`{"peer":"data"}`)}
	u := &taskUpstream{fakeBackend: newBackend(), callErr: up,
		tools: map[string][]mcpserver.Tool{"tasks": {supportTool("relay_job", "optional")}}}
	s := taskServer(u, "tasks")
	s.Timing.TaskEager = 5 * time.Second

	m := handle(t, s, "tools/call", callTool(extCaps, "relay_job"))
	if errCode(m) != -32603 {
		t.Fatalf("eager task: want the upstream's -32603, got %v", m)
	}
	relayedDataUnchanged(t, m)
}

func TestARelayedPeerErrorThroughTasksResultKeepsItsData(t *testing.T) {
	up := &mcpserver.UpstreamError{Code: -32603, Message: "as designed",
		Data: json.RawMessage(`{"peer":"data"}`)}
	u := &taskUpstream{fakeBackend: newBackend(), callErr: up, delay: 200 * time.Millisecond,
		tools: map[string][]mcpserver.Tool{"tasks": {supportTool("relay_job", "optional")}}}
	s := taskServer(u, "tasks")
	s.Timing.TaskEager = 50 * time.Millisecond

	r := resultOf(t, handle(t, s, "tools/call", callTool(extCaps, "relay_job")))
	id, _ := r["taskId"].(string)
	if id == "" {
		t.Fatalf("want a task, got %v", r)
	}
	waitTask(t, s, id, terminal)

	m := handle(t, s, "tasks/result", map[string]any{"taskId": id})
	if errCode(m) != -32603 {
		t.Fatalf("tasks/result: want the upstream's -32603, got %v", m)
	}
	relayedDataUnchanged(t, m)
}

func TestARelayedPeerErrorThroughAnAskedTaskKeepsItsData(t *testing.T) {
	a := &timedAsker{workAfter: 300 * time.Millisecond,
		upstream: &mcpserver.UpstreamError{Code: -32603, Message: "as designed",
			Data: json.RawMessage(`{"peer":"data"}`)}}
	s := askingServer(a, "protocol_error_job")
	r := resultOf(t, handle(t, s, "tools/call", callTool(extAskCaps, "protocol_error_job")))
	id, _ := r["taskId"].(string)
	if id == "" {
		t.Fatalf("want a task, got %v", r)
	}
	waitTask(t, s, id, terminal)

	m := handle(t, s, "tasks/result", map[string]any{"taskId": id})
	if errCode(m) != -32603 {
		t.Fatalf("asked task tasks/result: want the upstream's -32603, got %v", m)
	}
	relayedDataUnchanged(t, m)
}

// Codex review on PR #9038 (issue #2435), finding 2: the transport's
// own pre-dispatch error responses bypass HandleOn, so the marker
// must be stamped where they are minted. This table is the class
// guard: every pre-dispatch error family the HTTP transport mints
// carries the local-origin marker. Oracle: issue #2435's acceptance
// ("every error mcpx mints itself on the MCP surface").

func serveRaw(s *mcpserver.Server, method string, headers map[string]string, body string) *httptest.ResponseRecorder {
	r := httptest.NewRequest(method, "/mcp", strings.NewReader(body))
	for k, v := range headers {
		r.Header.Set(k, v)
	}
	w := httptest.NewRecorder()
	s.ServeHTTP(w, r)
	return w
}

// firstErrorData decodes the first error's data from a response body
// that is either one response object or a batch reply array.
func firstErrorData(t *testing.T, body []byte) map[string]any {
	t.Helper()
	var raw json.RawMessage = body
	var arr []json.RawMessage
	if err := json.Unmarshal(body, &arr); err == nil && len(arr) > 0 {
		raw = arr[0]
	}
	var resp struct {
		Error *struct {
			Data map[string]any `json:"data"`
		} `json:"error"`
	}
	if err := json.Unmarshal(raw, &resp); err != nil || resp.Error == nil {
		t.Fatalf("response carries no error: %s", body)
	}
	return resp.Error.Data
}

func TestTransportLocalErrorsCarryTheLocalOriginMarker(t *testing.T) {
	plain := func() *mcpserver.Server { return mcpserver.New(newBackend(), "mcpx", "test") }
	origined := func() *mcpserver.Server {
		s := plain()
		s.Origins = mcpserver.OriginPolicy{Hosts: []string{"localhost", "127.0.0.1", "::1"},
			Origins: []string{"https://app.example.com"}}
		return s
	}
	oneCall := `[{"jsonrpc":"2.0","id":1,"method":"tools/list","params":{}}]`
	cases := []struct {
		name    string
		server  func() *mcpserver.Server
		method  string
		headers map[string]string
		body    string
	}{
		{"rejected Origin", origined, http.MethodPost,
			map[string]string{"Origin": "https://evil.example"},
			string(frame(1, "tools/list", nil))},
		{"empty batch", plain, http.MethodPost, nil, `[]`},
		{"modern batch refused", plain, http.MethodPost,
			map[string]string{"MCP-Protocol-Version": "2026-07-28"}, oneCall},
		{"batch refused for 2025-06-18", plain, http.MethodPost,
			map[string]string{"MCP-Protocol-Version": "2025-06-18"}, oneCall},
		{"batch with an unknown session", plain, http.MethodPost,
			map[string]string{"Mcp-Session-Id": "no-such"}, oneCall},
		{"batch element framing", plain, http.MethodPost, nil, `[123]`},
		{"batch element is initialize", plain, http.MethodPost, nil,
			`[{"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}]`},
		{"DELETE without a session", plain, http.MethodDelete, nil, ""},
		{"DELETE with an unknown session", plain, http.MethodDelete,
			map[string]string{"Mcp-Session-Id": "no-such"}, ""},
		{"GET without a session", plain, http.MethodGet, nil, ""},
		{"GET with an unknown session", plain, http.MethodGet,
			map[string]string{"Mcp-Session-Id": "no-such"}, ""},
	}
	for _, tc := range cases {
		t.Run(tc.name, func(t *testing.T) {
			w := serveRaw(tc.server(), tc.method, tc.headers, tc.body)
			data := firstErrorData(t, w.Body.Bytes())
			mcpx, ok := data["mcpx"].(map[string]any)
			if !ok || mcpx["origin"] != "local" {
				t.Fatalf("%s: transport-local error lacks the origin marker: status %d, data = %v (%s)",
					tc.name, w.Code, data, w.Body.String())
			}
		})
	}
}
