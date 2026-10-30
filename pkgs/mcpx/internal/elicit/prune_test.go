package elicit_test

import (
	"encoding/json"
	"path/filepath"
	"testing"
	"time"

	"github.com/dezren39/mcpx/internal/defaults"
	"github.com/dezren39/mcpx/internal/elicit"
)

// Retention (#1677): opening the broker prunes terminal rows past the
// bounds in defaults. These tests seed rows through the public API
// with backdated timestamps, close, and reopen -- the prune happens on
// the second open.

func seedAnswered(t *testing.T, b *elicit.Broker, id string, mode elicit.Mode, at time.Time) {
	t.Helper()
	_, err := b.OpenRequest(elicit.Request{
		ID: id, Mode: mode, Message: "q",
		Created: at.Add(-time.Minute), ExpiresAt: at.Add(time.Hour),
	})
	if err != nil {
		t.Fatal(err)
	}
	content := `{"v":1}`
	if mode == elicit.Sample {
		// A sampling answer must be a CreateMessageResult (role, model,
		// content) since the sampling validation landed on main.
		content = `{"role":"assistant","model":"m","content":{"type":"text","text":"hi"},"stopReason":"endTurn"}`
	}
	if err := b.Respond(elicit.Answer{
		ID: id, Action: elicit.Accept, Content: json.RawMessage(content), At: at,
	}); err != nil {
		t.Fatal(err)
	}
}

func TestOpenPrunesTerminalRowsPastTheAgeBound(t *testing.T) {
	path := filepath.Join(t.TempDir(), "elicit.db")
	b, err := elicit.Open(path)
	if err != nil {
		t.Fatal(err)
	}
	old := time.Now().Add(-40 * 24 * time.Hour) // beyond the 720h default
	seedAnswered(t, b, "elc-aged", elicit.Form, old)
	seedAnswered(t, b, "elc-fresh", elicit.Form, time.Now())
	// A pending question is never pruned, however old it was asked:
	// retention settles terminal rows only.
	if _, err := b.OpenRequest(elicit.Request{
		ID: "elc-pending", Message: "q",
		Created: old, ExpiresAt: time.Now().Add(time.Hour),
	}); err != nil {
		t.Fatal(err)
	}
	if err := b.Close(); err != nil {
		t.Fatal(err)
	}

	b2, err := elicit.Open(path)
	if err != nil {
		t.Fatal(err)
	}
	defer b2.Close()
	if _, ok, err := b2.Get("elc-aged"); err != nil || ok {
		t.Errorf("aged terminal row survived the prune (ok=%v, err=%v)", ok, err)
	}
	if _, ok, err := b2.Lookup("elc-aged"); err != nil || ok {
		t.Errorf("aged answer row survived the prune (ok=%v, err=%v)", ok, err)
	}
	if _, ok, err := b2.Get("elc-fresh"); err != nil || !ok {
		t.Errorf("fresh terminal row was pruned (ok=%v, err=%v)", ok, err)
	}
	if _, ok, err := b2.Lookup("elc-fresh"); err != nil || !ok {
		t.Errorf("fresh answer row was pruned (ok=%v, err=%v)", ok, err)
	}
	if r, ok, err := b2.Get("elc-pending"); err != nil || !ok || r.State != elicit.Pending {
		t.Errorf("pending row was pruned or settled (ok=%v, state=%v, err=%v)", ok, r.State, err)
	}
}

func TestOpenPrunesTerminalSamplingRows(t *testing.T) {
	// Sampling rows live in the same tables and obey the same bound:
	// a sampling request's messages are not retained between requests
	// once the row is terminal and aged (the conformance gap
	// sampling-messages-not-retained-between-requests).
	path := filepath.Join(t.TempDir(), "elicit.db")
	b, err := elicit.Open(path)
	if err != nil {
		t.Fatal(err)
	}
	old := time.Now().Add(-40 * 24 * time.Hour)
	seedAnswered(t, b, "elc-sample-aged", elicit.Sample, old)
	seedAnswered(t, b, "elc-sample-fresh", elicit.Sample, time.Now())
	if err := b.Close(); err != nil {
		t.Fatal(err)
	}

	b2, err := elicit.Open(path)
	if err != nil {
		t.Fatal(err)
	}
	defer b2.Close()
	if _, ok, err := b2.Get("elc-sample-aged"); err != nil || ok {
		t.Errorf("aged sampling row survived the prune (ok=%v, err=%v)", ok, err)
	}
	if _, ok, err := b2.Get("elc-sample-fresh"); err != nil || !ok {
		t.Errorf("fresh sampling row was pruned (ok=%v, err=%v)", ok, err)
	}
}

func TestOpenPrunesTerminalRowsPastTheCountBound(t *testing.T) {
	defer func(prevAge time.Duration, prevRows int) {
		defaults.ElicitRetentionMaxAge, defaults.ElicitRetentionMaxTerminalRows = prevAge, prevRows
	}(defaults.ElicitRetentionMaxAge, defaults.ElicitRetentionMaxTerminalRows)
	defaults.ElicitRetentionMaxAge = 100 * 365 * 24 * time.Hour // age bound inert here
	defaults.ElicitRetentionMaxTerminalRows = 3

	path := filepath.Join(t.TempDir(), "elicit.db")
	b, err := elicit.Open(path)
	if err != nil {
		t.Fatal(err)
	}
	base := time.Now().Add(-time.Hour)
	ids := []string{"elc-c1", "elc-c2", "elc-c3", "elc-c4", "elc-c5"}
	for i, id := range ids {
		seedAnswered(t, b, id, elicit.Form, base.Add(time.Duration(i)*time.Minute))
	}
	if err := b.Close(); err != nil {
		t.Fatal(err)
	}

	b2, err := elicit.Open(path)
	if err != nil {
		t.Fatal(err)
	}
	defer b2.Close()
	// Oldest-first out: the two oldest are gone, the newest three stay.
	for _, id := range ids[:2] {
		if _, ok, err := b2.Get(id); err != nil || ok {
			t.Errorf("%s survived the count prune (ok=%v, err=%v)", id, ok, err)
		}
		if _, ok, err := b2.Lookup(id); err != nil || ok {
			t.Errorf("%s's answer survived the count prune (ok=%v, err=%v)", id, ok, err)
		}
	}
	for _, id := range ids[2:] {
		if _, ok, err := b2.Get(id); err != nil || !ok {
			t.Errorf("%s was pruned inside the count bound (ok=%v, err=%v)", id, ok, err)
		}
	}
}
