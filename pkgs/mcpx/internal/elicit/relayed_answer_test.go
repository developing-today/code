package elicit_test

import (
	"encoding/json"
	"strings"
	"testing"

	"github.com/developing-today/code/pkgs/mcpx/internal/elicit"
)

// #8922: an answer relayed from the downstream MCP client (marked
// Relayed by the daemon ask-answers route, its only source) is judged
// by the upstream server, the way a direct connection behaves. The
// #1954 requestedSchema gate cannot apply to it: mcpx cannot correct
// a relayed answer, and refusing it left the question pending so the
// relay loop re-pushed the same question until its rounds ran out.
// The gate still applies to answers entered on mcpx's own surfaces.
func TestRespondRelayedAnswerSkipsSchemaGate(t *testing.T) {
	b := open(t)
	r := ask(t, b, elicit.Request{Mode: elicit.Form, Schema: json.RawMessage(formSchema)})

	content := json.RawMessage(`{"count": 1}`) // missing the required repo field

	// The same content from mcpx's own surface is still refused.
	if err := b.Respond(elicit.Answer{ID: r.ID, Action: elicit.Accept,
		Content: content, By: "cli"}); err == nil {
		t.Fatal("a cli answer that misses a required field was accepted")
	}

	// Relayed, it is recorded as given; the upstream judges it.
	if err := b.Respond(elicit.Answer{ID: r.ID, Action: elicit.Accept,
		Content: content, By: "mcp-client", Relayed: true}); err != nil {
		t.Fatalf("relayed answer refused: %v", err)
	}
	a, ok, err := b.Lookup(r.ID)
	if err != nil || !ok || a.Action != elicit.Accept {
		t.Fatalf("relayed answer not recorded: %+v ok=%v err=%v", a, ok, err)
	}
	if !strings.EqualFold(string(a.Content), string(content)) {
		t.Fatalf("relayed content changed in transit: %s", a.Content)
	}
}

// The plumbing refusals are not the schema gate: a relayed accept with
// no content at all is still refused, relayed or not.
func TestRespondRelayedAnswerStillNeedsContent(t *testing.T) {
	b := open(t)
	r := ask(t, b, elicit.Request{Mode: elicit.Form, Schema: json.RawMessage(formSchema)})
	if err := b.Respond(elicit.Answer{ID: r.ID, Action: elicit.Accept,
		By: "mcp-client", Relayed: true}); err == nil {
		t.Fatal("a relayed accept with no content was accepted")
	}
}
