package elicit_test

// Pins #3129: the deadline is enforced on the answer path, not only on
// the wait/list paths. An overdue question must not accept an answer,
// and Get must not present it as live pending -- both settle it as
// expired (expiry-as-cancel, decision 0006's lazy expire-on-read).

import (
	"strings"
	"testing"
	"time"

	"github.com/developing-today/code/pkgs/mcpx/internal/elicit"
)

func TestRespondRefusesAnOverdueQuestionAndSettlesItExpired(t *testing.T) {
	b := open(t)
	r := ask(t, b, elicit.Request{ExpiresAt: time.Now().Add(-time.Second)})

	err := b.Respond(elicit.Answer{ID: r.ID, Action: elicit.Decline, By: "cli"})
	if err == nil || !strings.Contains(err.Error(), "expired") {
		t.Fatalf("a late answer must be refused as expired, got %v", err)
	}
	a, ok, err := b.Lookup(r.ID)
	if err != nil || !ok {
		t.Fatalf("lookup: ok=%v err=%v", ok, err)
	}
	if a.Action != elicit.Cancel || a.By != "expiry" {
		t.Errorf("the settled outcome should be expiry-as-cancel, got %+v", a)
	}
	got, ok, err := b.Get(r.ID)
	if err != nil || !ok {
		t.Fatalf("get: ok=%v err=%v", ok, err)
	}
	if got.State != elicit.Expired {
		t.Errorf("state = %q, want %q", got.State, elicit.Expired)
	}
}

func TestGetExpiresAnOverdueQuestionInsteadOfPresentingItPending(t *testing.T) {
	b := open(t)
	r := ask(t, b, elicit.Request{ExpiresAt: time.Now().Add(-time.Second)})

	got, ok, err := b.Get(r.ID)
	if err != nil || !ok {
		t.Fatalf("get: ok=%v err=%v", ok, err)
	}
	if got.State != elicit.Expired {
		t.Errorf("an overdue item read must show expired, got %q", got.State)
	}
	pending, err := b.Pending(elicit.Filter{})
	if err != nil {
		t.Fatal(err)
	}
	if len(pending) != 0 {
		t.Errorf("nothing may remain pending: %+v", pending)
	}
}

func TestRespondStillAcceptsALiveQuestion(t *testing.T) {
	// Control: the deadline check changes nothing inside the lifetime.
	b := open(t)
	r := ask(t, b, elicit.Request{ExpiresAt: time.Now().Add(time.Hour)})

	if err := b.Respond(elicit.Answer{ID: r.ID, Action: elicit.Decline, By: "cli"}); err != nil {
		t.Fatal(err)
	}
	a, ok, err := b.Lookup(r.ID)
	if err != nil || !ok || a.Action != elicit.Decline {
		t.Errorf("live answer should stand: ok=%v err=%v %+v", ok, err, a)
	}
}
