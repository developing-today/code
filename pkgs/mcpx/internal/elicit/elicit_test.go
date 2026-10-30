package elicit_test

import (
	"context"
	"encoding/json"
	"path/filepath"
	"strings"
	"testing"
	"time"

	"github.com/dezren39/mcpx/internal/elicit"
)

func open(t *testing.T) *elicit.Broker {
	t.Helper()
	b, err := elicit.Open(filepath.Join(t.TempDir(), "elicit.db"))
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { b.Close() })
	return b
}

func ask(t *testing.T, b *elicit.Broker, r elicit.Request) elicit.Request {
	t.Helper()
	if r.Message == "" {
		r.Message = "which one?"
	}
	out, err := b.OpenRequest(r)
	if err != nil {
		t.Fatal(err)
	}
	return out
}

func TestAnAnswerCanComeFromAnotherProcess(t *testing.T) {
	// The whole point: the asker and the answerer need not be the same
	// process, so CI can raise a question a person answers later.
	b := open(t)
	r := ask(t, b, elicit.Request{Session: "s1"})

	done := make(chan elicit.Answer, 1)
	go func() {
		ctx, cancel := context.WithTimeout(context.Background(), 10*time.Second)
		defer cancel()
		a, err := b.Await(ctx, r.ID)
		if err != nil {
			t.Error(err)
		}
		done <- a
	}()

	time.Sleep(50 * time.Millisecond)
	if err := b.Respond(elicit.Answer{
		ID: r.ID, Action: elicit.Accept,
		Content: json.RawMessage(`{"repo":"me/thing"}`), By: "cli",
	}); err != nil {
		t.Fatal(err)
	}

	select {
	case a := <-done:
		if a.Action != elicit.Accept || !strings.Contains(string(a.Content), "me/thing") {
			t.Errorf("got %+v", a)
		}
	case <-time.After(5 * time.Second):
		t.Fatal("the waiter was never woken")
	}
}

func TestAnExpiredQuestionBecomesCancelNotDecline(t *testing.T) {
	// Expiry means dismissed without choosing. Telling a server the user
	// declined would say something different and untrue.
	b := open(t)
	r := ask(t, b, elicit.Request{ExpiresAt: time.Now().Add(-time.Second)})

	ctx, cancel := context.WithTimeout(context.Background(), 5*time.Second)
	defer cancel()
	a, err := b.Await(ctx, r.ID)
	if err != nil {
		t.Fatal(err)
	}
	if a.Action != elicit.Cancel {
		t.Errorf("expiry should cancel, got %q", a.Action)
	}
	if a.By != "expiry" {
		t.Errorf("it should say who answered: %q", a.By)
	}
}

func TestAnsweringAfterTheProcessDiedStillWorks(t *testing.T) {
	// The reattach case: nothing is in memory, and a fresh handle on the
	// same file finds the question and can answer it.
	dir := t.TempDir()
	path := filepath.Join(dir, "e.db")

	first, err := elicit.Open(path)
	if err != nil {
		t.Fatal(err)
	}
	r := ask(t, first, elicit.Request{Session: "s1"})
	first.Close() // the asking process goes away

	second, err := elicit.Open(path)
	if err != nil {
		t.Fatal(err)
	}
	defer second.Close()

	pending, err := second.Pending(elicit.Filter{Session: "s1"})
	if err != nil {
		t.Fatal(err)
	}
	if len(pending) != 1 || pending[0].ID != r.ID {
		t.Fatalf("the question should have survived: %+v", pending)
	}
	if err := second.Respond(elicit.Answer{ID: r.ID, Action: elicit.Decline}); err != nil {
		t.Fatal(err)
	}
	a, ok, _ := second.Lookup(r.ID)
	if !ok || a.Action != elicit.Decline {
		t.Errorf("got %+v", a)
	}
}

func TestAnsweringTwiceIsRefusedAndTheFirstStands(t *testing.T) {
	b := open(t)
	r := ask(t, b, elicit.Request{})
	if err := b.Respond(elicit.Answer{ID: r.ID, Action: elicit.Decline}); err != nil {
		t.Fatal(err)
	}
	err := b.Respond(elicit.Answer{ID: r.ID, Action: elicit.Accept,
		Content: json.RawMessage(`{"a":1}`)})
	if err == nil {
		t.Fatal("a second answer should be refused")
	}
	a, _, _ := b.Lookup(r.ID)
	if a.Action != elicit.Decline {
		t.Errorf("the first answer should stand: %q", a.Action)
	}
}

func TestAcceptingAFormWithoutContentIsRefused(t *testing.T) {
	// An accepted form elicitation whose content is missing is a protocol
	// violation the server cannot recover from.
	b := open(t)
	r := ask(t, b, elicit.Request{Mode: elicit.Form})
	if err := b.Respond(elicit.Answer{ID: r.ID, Action: elicit.Accept}); err == nil {
		t.Fatal("expected a refusal")
	}
}

func TestPendingExpiresOverdueQuestionsRatherThanListingThem(t *testing.T) {
	// Expiring on read means no sweeper to go wrong, and nothing is ever
	// reported as pending when it is not.
	b := open(t)
	ask(t, b, elicit.Request{Session: "s1", ExpiresAt: time.Now().Add(-time.Second)})
	fresh := ask(t, b, elicit.Request{Session: "s1", ExpiresAt: time.Now().Add(time.Hour)})

	pending, err := b.Pending(elicit.Filter{Session: "s1"})
	if err != nil {
		t.Fatal(err)
	}
	if len(pending) != 1 || pending[0].ID != fresh.ID {
		t.Errorf("only the live question should be pending: %+v", pending)
	}
}

func TestRoutingSendsCredentialsToAHumanAndTheRestToTheAgent(t *testing.T) {
	// The default is the agent: it asked for something, the question is part
	// of that request, and it has the context. A human is pulled in only for
	// what an agent cannot know or should not hold.
	cases := []struct {
		name string
		req  elicit.Request
		want elicit.Audience
	}{
		{"plain choice",
			elicit.Request{Schema: json.RawMessage(`{"properties":{"repo":{"type":"string"}}}`)},
			elicit.ToAgent},
		{"a token field",
			elicit.Request{Schema: json.RawMessage(`{"properties":{"api_token":{"type":"string"}}}`)},
			elicit.ToHuman},
		{"a password format",
			elicit.Request{Schema: json.RawMessage(`{"properties":{"pw":{"type":"string","format":"password"}}}`)},
			elicit.ToHuman},
		{"a bare confirmation",
			elicit.Request{Schema: json.RawMessage(`{"properties":{"confirm":{"type":"boolean"}}}`)},
			elicit.ToHuman},
		{"url mode", elicit.Request{Mode: elicit.URL, URL: "https://x"}, elicit.ToHuman},
	}
	for _, c := range cases {
		got, why := elicit.Route(c.req)
		if got != c.want {
			t.Errorf("%s: routed to %q, want %q (%s)", c.name, got, c.want, why)
		}
		if why == "" {
			t.Errorf("%s: a routing decision nobody can inspect is one nobody can correct", c.name)
		}
	}
}

func TestTTLRemainingNeverGoesNegative(t *testing.T) {
	r := elicit.Request{ExpiresAt: time.Now().Add(-time.Hour)}
	if r.TTLRemaining() != 0 {
		t.Errorf("got %v", r.TTLRemaining())
	}
}

func TestItemReadRenewsButCollectionReadDoesNot(t *testing.T) {
	// The convention (docs/decisions/0006-ttl-touch-on-read.md): an item
	// read is use and renews; a collection read is a listing and does not.
	b := open(t)
	b.Configure(2*time.Second, time.Hour)
	created := time.Now().Add(-1500 * time.Millisecond)
	r := ask(t, b, elicit.Request{Created: created})
	// Deadline at creation: created + ttl.
	if want := created.Add(2 * time.Second); !r.ExpiresAt.Truncate(time.Millisecond).Equal(want.Truncate(time.Millisecond)) {
		t.Fatalf("initial deadline = %v, want %v", r.ExpiresAt, want)
	}

	pending, err := b.Pending(elicit.Filter{})
	if err != nil {
		t.Fatal(err)
	}
	if len(pending) != 1 {
		t.Fatalf("pending = %d, want 1", len(pending))
	}
	if !pending[0].ExpiresAt.Equal(r.ExpiresAt.Truncate(time.Millisecond)) {
		t.Errorf("collection read renewed the deadline: %v -> %v",
			r.ExpiresAt, pending[0].ExpiresAt)
	}
	if !pending[0].LastTouched.Equal(created.Truncate(time.Millisecond)) {
		t.Errorf("collection read touched: lastTouched = %v, want %v",
			pending[0].LastTouched, created)
	}

	got, ok, err := b.Get(r.ID)
	if err != nil || !ok {
		t.Fatalf("get: ok=%v err=%v", ok, err)
	}
	if !got.ExpiresAt.After(r.ExpiresAt) {
		t.Errorf("item read did not renew: deadline still %v", got.ExpiresAt)
	}
	if got.LastTouched.Before(created) || time.Since(got.LastTouched) > time.Minute {
		t.Errorf("lastTouched = %v, want ~now", got.LastTouched)
	}
	if got.TTLRemainingMs <= 0 || got.TTLRemainingMs > 2000 {
		t.Errorf("ttlRemainingMs = %d, want in (0, 2000]", got.TTLRemainingMs)
	}
}

func TestTouchNeverPassesTheMaxAgeCeiling(t *testing.T) {
	// Renewal is bounded: however often a question is read, it dies at
	// created + maxAge.
	b := open(t)
	b.Configure(2*time.Second, 2500*time.Millisecond)
	created := time.Now().Add(-time.Second)
	r := ask(t, b, elicit.Request{Created: created})
	ceiling := created.Add(2500 * time.Millisecond)
	if !r.ExpiresAt.Truncate(time.Millisecond).Equal(created.Add(2 * time.Second).Truncate(time.Millisecond)) {
		t.Fatalf("initial deadline = %v, want created+ttl (below ceiling)", r.ExpiresAt)
	}
	time.Sleep(600 * time.Millisecond)
	got, ok, err := b.Get(r.ID)
	if err != nil || !ok {
		t.Fatalf("get: ok=%v err=%v", ok, err)
	}
	if got.ExpiresAt.After(ceiling) {
		t.Errorf("touch pushed the deadline past the ceiling: %v > %v", got.ExpiresAt, ceiling)
	}
	if got.ExpiresAt.Before(ceiling.Add(-100 * time.Millisecond)) {
		t.Errorf("deadline = %v, want ~ceiling %v", got.ExpiresAt, ceiling)
	}
}

func TestTouchNeverShortensAnExplicitExpiry(t *testing.T) {
	// An explicit lifetime at creation is the exception the convention
	// honours: a read renews the default family, it does not cut down a
	// deadline the creator chose.
	b := open(t)
	b.Configure(2*time.Second, time.Hour)
	explicit := time.Now().Add(time.Hour)
	r := ask(t, b, elicit.Request{ExpiresAt: explicit})
	got, ok, err := b.Get(r.ID)
	if err != nil || !ok {
		t.Fatalf("get: ok=%v err=%v", ok, err)
	}
	if !got.ExpiresAt.Equal(r.ExpiresAt.Truncate(time.Millisecond)) {
		t.Errorf("touch shortened an explicit expiry: %v -> %v", r.ExpiresAt, got.ExpiresAt)
	}
}

func TestAnAbandonedQuestionStillExpires(t *testing.T) {
	// Touch-on-read changes nothing for a question nobody reads: it
	// expires on the idle schedule, exactly as before.
	b := open(t)
	b.Configure(300*time.Millisecond, time.Hour)
	r := ask(t, b, elicit.Request{})
	time.Sleep(450 * time.Millisecond)
	pending, err := b.Pending(elicit.Filter{})
	if err != nil {
		t.Fatal(err)
	}
	if len(pending) != 0 {
		t.Errorf("abandoned question %s still pending after its ttl", r.ID)
	}
}
