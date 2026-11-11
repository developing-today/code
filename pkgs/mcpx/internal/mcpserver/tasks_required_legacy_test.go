package mcpserver_test

import (
	"testing"
)

// #1974 headline 13 / section C item C10: on the legacy (2025-11-25) path
// a non-task-augmented call to a tool whose execution.taskSupport is
// "required" is refused -32601 — the tool-level MUST ("Servers MUST
// return a -32601 (Method not found) error if a client does not attempt
// to do so") — not the protocol-errors list's -32600, which covers
// request types with no tool-level rule. Oracle: the 2025-11-25 tasks
// page's tool-level negotiation rule.
func TestLegacyNonAugmentedCallToRequiredToolIsRefused(t *testing.T) {
	s := rawTaskServer()
	m := postQuery(s, "codemode=false", frame(1, "tools/call",
		map[string]any{"name": "db_job", "arguments": map[string]any{}}))
	if errCode(m) != -32601 {
		t.Fatalf("non-augmented legacy call to a required tool: want -32601, got %v", m)
	}

	// An optional tool called the plain way still runs the plain way:
	// the refusal is only for "required".
	m = postQuery(s, "codemode=false", frame(2, "tools/call",
		map[string]any{"name": "db_nap", "arguments": map[string]any{}}))
	if errCode(m) != 0 {
		t.Fatalf("non-augmented legacy call to an optional tool should run: got %v", m)
	}
	if _, isTask := m["result"].(map[string]any)["task"]; isTask {
		t.Fatalf("a plain call must not silently become a task: %v", m)
	}
}
