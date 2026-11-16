package daemon

import (
	"context"
	"encoding/json"
	"errors"
	"path/filepath"
	"testing"
	"time"

	"github.com/developing-today/code/pkgs/mcpx/internal/elicit"
	"github.com/developing-today/code/pkgs/mcpx/internal/events"
)

// The ask seam's no-audience refusal (#2438, Phase 3 of #2434): a
// question routed to an audience with no answerer polling is answered
// at once with -32021 naming the capability, instead of waiting out the
// broker TTL for an answer nobody will give. The oracle is the broker's
// own presence record: absence opens nothing, a recorded answerer opens
// the question as before.

// audienceRig is a registry over a broker nobody has answered from yet.
func audienceRig(t *testing.T) (*Registry, *elicit.Broker) {
	t.Helper()
	broker, err := elicit.Open(filepath.Join(t.TempDir(), "elicit.db"))
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { _ = broker.Close() })
	r := &Registry{asks: newAskTable()}
	r.InstallHooks(events.New(0), broker, nil)
	return r, broker
}

// requiredCapabilities reads the data member of a -32021 as a client
// would: the map of capabilities it has to declare.
func requiredCapabilities(t *testing.T, err error) map[string]any {
	t.Helper()
	var ce interface {
		RPCCode() int
		RPCData() json.RawMessage
	}
	if !errors.As(err, &ce) {
		t.Fatalf("the refusal is not a coded error: %v", err)
	}
	if ce.RPCCode() != -32021 {
		t.Fatalf("code %d, want -32021 (-32021 MissingRequiredClientCapability)", ce.RPCCode())
	}
	var body struct {
		Required map[string]any `json:"requiredCapabilities"`
	}
	if err := json.Unmarshal(ce.RPCData(), &body); err != nil {
		t.Fatalf("the -32021 data is not requiredCapabilities: %v (%s)", err, ce.RPCData())
	}
	return body.Required
}

func TestElicitWithoutAnswererIsMissingCapability(t *testing.T) {
	r, broker := audienceRig(t)
	_, err := r.elicitViaBroker(context.Background(), "srv", "key",
		json.RawMessage(`{"mode":"form","message":"who?","requestedSchema":{"type":"object"}}`))
	required := requiredCapabilities(t, err)
	if _, ok := required["elicitation"]; !ok {
		t.Fatalf("requiredCapabilities %v do not name elicitation", required)
	}
	// Nobody was asked, so nothing may be left pending for an answerer
	// that does not exist.
	if ps, perr := broker.Pending(elicit.Filter{}); perr != nil || len(ps) != 0 {
		t.Fatalf("pending after a no-audience refusal: %d (err %v)", len(ps), perr)
	}
}

func TestSampleWithoutAnswererIsMissingCapability(t *testing.T) {
	r, broker := audienceRig(t)
	_, err := r.sampleViaBroker(context.Background(), "srv", "key",
		json.RawMessage(`{"messages":[{"role":"user","content":{"type":"text","text":"hi"}}],"maxTokens":7}`))
	required := requiredCapabilities(t, err)
	if _, ok := required["sampling"]; !ok {
		t.Fatalf("requiredCapabilities %v do not name sampling", required)
	}
	if ps, perr := broker.Pending(elicit.Filter{}); perr != nil || len(ps) != 0 {
		t.Fatalf("pending after a no-audience refusal: %d (err %v)", len(ps), perr)
	}
}

func TestAPresentAnswererKeepsTheQuestionOpen(t *testing.T) {
	r, broker := audienceRig(t)
	if err := broker.NoteAnswerer(""); err != nil {
		t.Fatal(err)
	}
	done := make(chan error, 1)
	go func() {
		_, err := r.elicitViaBroker(context.Background(), "srv", "key",
			json.RawMessage(`{"mode":"form","message":"who?","requestedSchema":{"type":"object"}}`))
		done <- err
	}()
	// The question reaches the broker, which is what "an answerer is
	// present" buys: the ordinary await-and-answer path.
	var q elicit.Request
	for i := 0; i < 200 && q.ID == ""; i++ {
		ps, err := broker.Pending(elicit.Filter{})
		if err != nil {
			t.Fatal(err)
		}
		if len(ps) > 0 {
			q = ps[0]
		}
		time.Sleep(2 * time.Millisecond)
	}
	if q.ID == "" {
		t.Fatal("no question was opened although an answerer was present")
	}
	if err := broker.Respond(elicit.Answer{ID: q.ID, Action: elicit.Accept, By: "tester", Content: json.RawMessage(`{"who":"tester"}`)}); err != nil {
		t.Fatal(err)
	}
	if err := <-done; err != nil {
		t.Fatalf("the answered question failed: %v", err)
	}
}
