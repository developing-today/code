package mcpserver_test

import (
	"encoding/json"
	"testing"

	"github.com/developing-today/code/pkgs/mcpx/internal/mcpserver"
)

// #9828: PR #4194's declaredValue fix covered the elicitation root and the
// sampling sub-capabilities, but two presence-only root gates remained --
// CanSample read the sampling root with Declared, and the roots/list
// branch of Question.Sendable read the roots root the same way. A client
// that serializes an unset capability as JSON null ({"sampling":null},
// {"roots":null}) therefore counted as having declared it. Oracle: the
// #2474 rule -- a null declaration behaves exactly as no declaration,
// while {} and any non-null value still count as declared.

func TestNullSamplingRootDeclaresNothing(t *testing.T) {
	nullPeer := peer("2025-11-25", `{"sampling":null}`)
	if nullPeer.CanSample() {
		t.Error("CanSample must be false for {\"sampling\":null}")
	}
	if nullPeer.AnswersInline() {
		t.Error("AnswersInline must be false when the only inline capability is {\"sampling\":null}")
	}
	plain := mcpserver.Question{ID: "p", Method: "sampling/createMessage",
		Params: json.RawMessage(`{"messages":[{"role":"user","content":{"type":"text","text":"hi"}}],"maxTokens":5}`)}
	if plain.Sendable(nullPeer) {
		t.Error("a plain sampling question must not be offered on a null sampling declaration")
	}

	bare := peer("2025-11-25", `{"sampling":{}}`)
	if !bare.CanSample() {
		t.Error("CanSample must stay true for {\"sampling\":{}}")
	}
	if !plain.Sendable(bare) {
		t.Error("a plain sampling question must still reach a bare sampling host")
	}
}

func TestNullRootsRootDeclaresNothing(t *testing.T) {
	q := mcpserver.Question{ID: "r", Method: "roots/list"}

	if q.Sendable(peer("2025-11-25", `{"roots":null}`)) {
		t.Error("roots/list must not be offered on a null roots declaration")
	}
	if !q.Sendable(peer("2025-11-25", `{"roots":{}}`)) {
		t.Error("roots/list must reach a client that declared {\"roots\":{}}")
	}
	if !q.Sendable(peer("2025-11-25", `{"roots":{"listChanged":true}}`)) {
		t.Error("roots/list must reach a client that declared roots with listChanged")
	}
}
