package mcpserver_test

import (
	"encoding/json"

	"github.com/developing-today/code/pkgs/mcpx/internal/mcpserver"
	"net/http/httptest"
	"strings"
	"testing"
)

// postSession posts one frame on an initialized legacy session at the
// given query string and decodes the single response frame.
func postSession(t *testing.T, s *mcpserver.Server, sid, query string, body []byte) map[string]any {
	t.Helper()
	r := httptest.NewRequest("POST", "/mcp?"+query, strings.NewReader(string(body)))
	r.Header.Set("Content-Type", "application/json")
	r.Header.Set("Mcp-Session-Id", sid)
	w := httptest.NewRecorder()
	s.ServeHTTP(w, r)
	var m map[string]any
	if err := json.Unmarshal(w.Body.Bytes(), &m); err != nil {
		t.Fatalf("response does not parse: %s", w.Body.String())
	}
	return m
}

// #1974 headline 13 / section C item C10: on the legacy (2025-11-25) path
// a non-task-augmented call to a tool whose execution.taskSupport is
// "required" is refused -32601 — the tool-level MUST ("Servers MUST
// return a -32601 (Method not found) error if a client does not attempt
// to do so") — not the protocol-errors list's -32600, which covers
// request types with no tool-level rule. Oracle: the 2025-11-25 tasks
// page's tool-level negotiation rule.
func TestLegacyNonAugmentedCallToRequiredToolIsRefused(t *testing.T) {
	s := rawTaskServer()
	sid := legacySession(t, s, "2025-11-25")
	m := postSession(t, s, sid, "codemode=false", frame(1, "tools/call",
		map[string]any{"name": "db_job", "arguments": map[string]any{}}))
	if errCode(m) != -32601 {
		t.Fatalf("non-augmented legacy call to a required tool: want -32601, got %v", m)
	}

	// An optional tool called the plain way still runs the plain way:
	// the refusal is only for "required".
	m = postSession(t, s, sid, "codemode=false", frame(2, "tools/call",
		map[string]any{"name": "db_nap", "arguments": map[string]any{}}))
	if errCode(m) != 0 {
		t.Fatalf("non-augmented legacy call to an optional tool should run: got %v", m)
	}
	if _, isTask := m["result"].(map[string]any)["task"]; isTask {
		t.Fatalf("a plain call must not silently become a task: %v", m)
	}
}

// The refusal binds only a peer whose revision defines tasks. A
// 2025-06-18 client has no task augmentation and never saw the
// requirement (`execution` is stripped from its listings), so its
// plain call to the same tool runs as it always did (PR #8652 review).
func TestPreTaskEraCallToRequiredToolStillRuns(t *testing.T) {
	s := rawTaskServer()
	sid := legacySession(t, s, "2025-06-18")
	m := postSession(t, s, sid, "codemode=false", frame(1, "tools/call",
		map[string]any{"name": "db_job", "arguments": map[string]any{}}))
	if errCode(m) != 0 {
		t.Fatalf("pre-task-era call to a required tool should run: got %v", m)
	}
}
