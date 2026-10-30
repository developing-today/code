package elicit_test

import (
	"encoding/json"
	"strings"
	"testing"

	"github.com/dezren39/mcpx/internal/elicit"
)

// #1954: an accepted form answer is validated against the question's
// requestedSchema at the broker, before the waiter relays it upstream.
const formSchema = `{
	"type": "object",
	"properties": {
		"repo": {"type": "string"},
		"count": {"type": "integer"}
	},
	"required": ["repo"],
	"additionalProperties": false
}`

func TestRespondValidatesAcceptedFormContentAgainstSchema(t *testing.T) {
	b := open(t)
	r := ask(t, b, elicit.Request{Mode: elicit.Form, Schema: json.RawMessage(formSchema)})

	bad := map[string]string{
		"wrong type":       `{"repo": 42}`,
		"missing required": `{"count": 1}`,
		"extra property":   `{"repo": "me/thing", "surprise": true}`,
		"not an object":    `["repo"]`,
	}
	for name, content := range bad {
		err := b.Respond(elicit.Answer{ID: r.ID, Action: elicit.Accept,
			Content: json.RawMessage(content), By: "cli"})
		if err == nil {
			t.Errorf("%s: Respond accepted %s against the schema", name, content)
			continue
		}
		if !strings.Contains(err.Error(), "requested schema") {
			t.Errorf("%s: error %q does not name the schema mismatch", name, err)
		}
		// The refusal leaves the question pending for a corrected answer.
		got, ok, err := b.Get(r.ID)
		if err != nil || !ok {
			t.Fatalf("%s: Get after refusal: ok=%v err=%v", name, ok, err)
		}
		if got.State != elicit.Pending {
			t.Errorf("%s: state after refusal = %s, want pending", name, got.State)
		}
		if _, ok, err := b.Lookup(r.ID); err != nil || ok {
			t.Errorf("%s: a refused answer was recorded (ok=%v, err=%v)", name, ok, err)
		}
	}

	if err := b.Respond(elicit.Answer{ID: r.ID, Action: elicit.Accept,
		Content: json.RawMessage(`{"repo": "me/thing", "count": 2}`), By: "cli"}); err != nil {
		t.Fatalf("valid content refused: %v", err)
	}
	a, ok, err := b.Lookup(r.ID)
	if err != nil || !ok || a.Action != elicit.Accept {
		t.Fatalf("valid answer not recorded: %+v ok=%v err=%v", a, ok, err)
	}
}

// A question with no requestedSchema, and non-accept answers, are
// untouched by the validation.
func TestRespondWithoutSchemaOrAcceptIsUnvalidated(t *testing.T) {
	b := open(t)

	plain := ask(t, b, elicit.Request{Mode: elicit.Form})
	if err := b.Respond(elicit.Answer{ID: plain.ID, Action: elicit.Accept,
		Content: json.RawMessage(`{"anything": [1, 2]}`), By: "cli"}); err != nil {
		t.Errorf("no-schema accept refused: %v", err)
	}

	schemed := ask(t, b, elicit.Request{Mode: elicit.Form, Schema: json.RawMessage(formSchema)})
	if err := b.Respond(elicit.Answer{ID: schemed.ID, Action: elicit.Decline, By: "cli"}); err != nil {
		t.Errorf("decline against a schema refused: %v", err)
	}
}

// A requestedSchema that does not compile is the asker's defect: the
// answer is recorded unvalidated rather than refused.
func TestRespondRecordsAnswerWhenSchemaDoesNotCompile(t *testing.T) {
	b := open(t)
	r := ask(t, b, elicit.Request{Mode: elicit.Form,
		Schema: json.RawMessage(`{"type": "nonsense"}`)})
	if err := b.Respond(elicit.Answer{ID: r.ID, Action: elicit.Accept,
		Content: json.RawMessage(`{"repo": 42}`), By: "cli"}); err != nil {
		t.Errorf("answer wedged on an uncompilable schema: %v", err)
	}
}
