package mcpserver_test

import (
	"encoding/json"
	"testing"

	"github.com/dezren39/mcpx/internal/mcpserver"
)

// Phase 1 of #1753 (#1755): the sampling relay checks the sub-capabilities
// a host actually declared. A tools-carrying question is refused whole for
// a host without sampling.tools -- never stripped and degraded -- and
// includeContext is dropped for a host without sampling.context.

func TestSamplingToolsQuestionsGoOnlyToAClientThatDeclaredSamplingTools(t *testing.T) {
	bare := peer("2025-11-25", `{"sampling":{}}`)
	full := peer("2025-11-25", `{"sampling":{"tools":{},"context":{}}}`)
	malformed := mcpserver.Peer{Version: "2025-11-25", Caps: map[string]json.RawMessage{
		"sampling": json.RawMessage(`{"tools":`)}}

	plain := mcpserver.Question{ID: "p", Method: "sampling/createMessage",
		Params: json.RawMessage(`{"messages":[{"role":"user","content":{"type":"text","text":"hi"}}],"maxTokens":5}`)}
	withTools := mcpserver.Question{ID: "t", Method: "sampling/createMessage",
		Params: json.RawMessage(`{"messages":[],"maxTokens":5,"tools":[{"name":"get","inputSchema":{"type":"object"}}]}`)}
	withChoice := mcpserver.Question{ID: "c", Method: "sampling/createMessage",
		Params: json.RawMessage(`{"messages":[],"maxTokens":5,"toolChoice":{"mode":"auto"}}`)}

	if !plain.Sendable(bare) {
		t.Error("a plain sampling question must reach a bare sampling host")
	}
	if withTools.Sendable(bare) || withChoice.Sendable(bare) {
		t.Error("a tools-carrying question must not be offered to a host without sampling.tools")
	}
	if !withTools.Sendable(full) || !withChoice.Sendable(full) {
		t.Error("a tools-carrying question must reach a host that declared sampling.tools")
	}
	if !malformed.CanSample() {
		t.Error("CanSample keeps its parent meaning: key presence, parse or not")
	}
	if malformed.CanSampleTools() || malformed.CanSampleContext() {
		t.Error("a malformed sampling declaration must behave as undeclared")
	}
	if withTools.Sendable(malformed) {
		t.Error("a tools-carrying question must not be offered on a malformed declaration")
	}
}

func TestSamplingIncludeContextReachesOnlyAClientThatDeclaredSamplingContext(t *testing.T) {
	bare := peer("2025-11-25", `{"sampling":{}}`)
	full := peer("2025-11-25", `{"sampling":{"tools":{},"context":{}}}`)
	q := mcpserver.Question{ID: "s", Method: "sampling/createMessage",
		Params: json.RawMessage(`{"messages":[{"role":"user","content":{"type":"text","text":"hi"}}],"maxTokens":5,"includeContext":"allServers"}`)}

	out, err := q.ParamsFor(bare)
	if err != nil {
		t.Fatal(err)
	}
	var got map[string]any
	if err := json.Unmarshal(out, &got); err != nil {
		t.Fatal(err)
	}
	if _, ok := got["includeContext"]; ok {
		t.Errorf("includeContext relayed to a host without sampling.context: %s", out)
	}
	if got["maxTokens"] != float64(5) {
		t.Errorf("the rest of the question must pass unchanged: %s", out)
	}

	out, err = q.ParamsFor(full)
	if err != nil {
		t.Fatal(err)
	}
	got = nil
	if err := json.Unmarshal(out, &got); err != nil {
		t.Fatal(err)
	}
	if got["includeContext"] != "allServers" {
		t.Errorf("includeContext must pass verbatim to a host that declared sampling.context: %s", out)
	}
}
