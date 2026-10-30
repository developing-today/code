package elicit_test

import (
	"encoding/json"
	"testing"

	"github.com/dezren39/mcpx/internal/elicit"
)

func req(t *testing.T, raw string, arrayOK bool) error {
	t.Helper()
	return elicit.ValidateSampleRequest(json.RawMessage(raw), arrayOK)
}

func res(t *testing.T, raw string, arrayOK, tools bool) error {
	t.Helper()
	return elicit.ValidateSampleResult(json.RawMessage(raw), arrayOK, tools)
}

func TestWellFormedSamplingRequestPassesInBothEras(t *testing.T) {
	// A plain text exchange is legal in every revision.
	body := `{"messages":[{"role":"user","content":{"type":"text","text":"hi"}}],"maxTokens":16}`
	if err := req(t, body, false); err != nil {
		t.Fatalf("legacy era rejected a plain request: %v", err)
	}
	if err := req(t, body, true); err != nil {
		t.Fatalf("modern era rejected a plain request: %v", err)
	}
}

func TestArrayContentIsOnlyLegalFrom20251125(t *testing.T) {
	// SMP-05: single block before 2025-11-25, single or array after.
	body := `{"messages":[{"role":"user","content":[{"type":"text","text":"hi"}]}]}`
	if err := req(t, body, false); err == nil {
		t.Fatal("an array was accepted for a revision that allows only a single block")
	}
	if err := req(t, body, true); err != nil {
		t.Fatalf("an array was rejected for a revision that allows one: %v", err)
	}
}

func TestToolResultMessageMustHoldOnlyToolResults(t *testing.T) {
	// SMP-06: "it MUST contain ONLY tool results".
	body := `{"messages":[
		{"role":"assistant","content":[{"type":"tool_use","id":"t1","name":"x","input":{}}]},
		{"role":"user","content":[{"type":"tool_result","toolUseId":"t1"},{"type":"text","text":"and?"}]}
	]}`
	if err := req(t, body, true); err == nil {
		t.Fatal("a message mixing tool results with text was accepted")
	}
}

func TestToolResultIDMustPairWithAnEarlierToolUse(t *testing.T) {
	body := `{"messages":[{"role":"user","content":[{"type":"tool_result","toolUseId":"nope"}]}]}`
	if err := req(t, body, true); err == nil {
		t.Fatal("a tool_result naming no earlier tool_use was accepted")
	}
	paired := `{"messages":[
		{"role":"assistant","content":[{"type":"tool_use","id":"t1","name":"x","input":{}}]},
		{"role":"user","content":[{"type":"tool_result","toolUseId":"t1"}]}
	]}`
	if err := req(t, paired, true); err != nil {
		t.Fatalf("a properly paired tool result was rejected: %v", err)
	}
}

func TestSamplingResultMustCarryRoleAndModel(t *testing.T) {
	if err := res(t, `{"content":{"type":"text","text":"hi"},"model":"m"}`, true, false); err == nil {
		t.Fatal("a result with no role was accepted")
	}
	if err := res(t, `{"role":"assistant","content":{"type":"text","text":"hi"}}`, true, false); err == nil {
		t.Fatal("a result with no model was accepted")
	}
	ok := `{"role":"assistant","model":"m","content":{"type":"text","text":"hi"},"stopReason":"endTurn"}`
	if err := res(t, ok, true, false); err != nil {
		t.Fatalf("a well-formed result was refused: %v", err)
	}
}

func TestToolUseResultNeedsAToolsCarryingRequest(t *testing.T) {
	toolUse := `{"role":"assistant","model":"m","content":[{"type":"tool_use","id":"t1","name":"x","input":{}}],"stopReason":"toolUse"}`
	if err := res(t, toolUse, true, false); err == nil {
		t.Fatal("tool use was accepted against a request that carried no tools")
	}
	if err := res(t, toolUse, true, true); err != nil {
		t.Fatalf("tool use was rejected against a request that carried tools: %v", err)
	}
	stopOnly := `{"role":"assistant","model":"m","content":{"type":"text","text":"hi"},"stopReason":"toolUse"}`
	if err := res(t, stopOnly, true, false); err == nil {
		t.Fatal("stopReason toolUse was accepted against a request that carried no tools")
	}
}

func TestSamplingBlocksMustCarryTheirFields(t *testing.T) {
	// A block that decodes is not necessarily a block. Codex review on
	// PR #2443: {} and {"type":"text","text":42} both passed the result
	// validator, closing the broker question and forwarding an invalid
	// CreateMessageResult upstream.
	if err := res(t, `{"role":"assistant","model":"m","content":{}}`, true, false); err == nil {
		t.Fatal("a result whose content block has no type was accepted")
	}
	if err := res(t, `{"role":"assistant","model":"m","content":{"type":"text","text":42}}`, true, false); err == nil {
		t.Fatal("a result whose text is a number was accepted")
	}
	if err := res(t, `{"role":"assistant","model":"m","content":{"type":"text"}}`, true, false); err == nil {
		t.Fatal("a result whose text block carries no text was accepted")
	}
	if err := req(t, `{"messages":[{"role":"user","content":{"type":"text","text":42}}]}`, true); err == nil {
		t.Fatal("a request whose text is a number was accepted")
	}
	if err := res(t, `{"role":"assistant","model":"m","content":[{"type":"tool_use","id":"t1"}],"stopReason":"toolUse"}`, true, true); err == nil {
		t.Fatal("a tool_use block with no name or input was accepted")
	}
}

func TestModelHintMismatchIsNotAFailure(t *testing.T) {
	// Hints are expressly ignorable, so the answer's model need not be the
	// one the request preferred.
	if err := req(t, `{"messages":[{"role":"user","content":{"type":"text","text":"hi"}}],"modelPreferences":{"hints":[{"name":"preferred"}]}}`, true); err != nil {
		t.Fatalf("a request with a model hint was rejected: %v", err)
	}
	if err := res(t, `{"role":"assistant","model":"something-else","content":{"type":"text","text":"hi"}}`, true, false); err != nil {
		t.Fatalf("an answer from a different model was refused: %v", err)
	}
}

func TestSamplingInvalidParamsCarries32602(t *testing.T) {
	var err error = &elicit.SamplingInvalidParams{Err: jsonError("bad")}
	coded, ok := err.(interface{ RPCCode() int })
	if !ok || coded.RPCCode() != -32602 {
		t.Fatalf("SamplingInvalidParams RPCCode = %v, want -32602", coded)
	}
}

type jsonError string

func (e jsonError) Error() string { return string(e) }
