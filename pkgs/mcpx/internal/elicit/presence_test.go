package elicit_test

import (
	"path/filepath"
	"testing"
	"time"

	"github.com/developing-today/code/pkgs/mcpx/internal/elicit"
)

func openPresenceBroker(t *testing.T) *elicit.Broker {
	t.Helper()
	b, err := elicit.Open(filepath.Join(t.TempDir(), "elicit.db"))
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { _ = b.Close() })
	return b
}

// No heartbeat recorded: no audience is present. This is the absence
// the -32021 ask-seam rule consumes -- it must be a recorded absence,
// never a guess.
func TestHasAudienceNoHeartbeat(t *testing.T) {
	b := openPresenceBroker(t)
	r := elicit.Request{Mode: elicit.Form}
	if b.HasAudience(r, elicit.DefaultAudienceWindow) {
		t.Fatal("audience present with no heartbeat recorded")
	}
	if _, present, err := b.AnswererSeen(elicit.ToAgent, elicit.DefaultAudienceWindow); err != nil || present {
		t.Fatalf("AnswererSeen = present %v, err %v; want absent, nil", present, err)
	}
}

// A heartbeat for one audience does not invent the other.
func TestNoteAnswererScoped(t *testing.T) {
	b := openPresenceBroker(t)
	if err := b.NoteAnswerer(elicit.ToHuman); err != nil {
		t.Fatal(err)
	}
	human := elicit.Request{Mode: elicit.URL} // routes ToHuman
	if !b.HasAudience(human, elicit.DefaultAudienceWindow) {
		t.Fatal("human audience absent after its heartbeat")
	}
	agent := elicit.Request{Mode: elicit.Sample} // routes ToAgent
	if b.HasAudience(agent, elicit.DefaultAudienceWindow) {
		t.Fatal("agent audience present on a human-only heartbeat")
	}
}

// An unfiltered answerer serves both audiences.
func TestNoteAnswererUnfiltered(t *testing.T) {
	b := openPresenceBroker(t)
	if err := b.NoteAnswerer(""); err != nil {
		t.Fatal(err)
	}
	for _, r := range []elicit.Request{{Mode: elicit.URL}, {Mode: elicit.Sample}} {
		if !b.HasAudience(r, elicit.DefaultAudienceWindow) {
			t.Fatalf("audience for mode %v absent after unfiltered heartbeat", r.Mode)
		}
	}
}

// A heartbeat older than the window is absence again.
func TestAnswererSeenWindow(t *testing.T) {
	b := openPresenceBroker(t)
	if err := b.NoteAnswerer(elicit.ToAgent); err != nil {
		t.Fatal(err)
	}
	if _, present, _ := b.AnswererSeen(elicit.ToAgent, -time.Second); present {
		t.Fatal("stale heartbeat counted as present")
	}
}

// A recorded answer is itself presence for the question's audience.
func TestRespondNotesPresence(t *testing.T) {
	b := openPresenceBroker(t)
	req, err := b.OpenRequest(elicit.Request{
		Mode: elicit.Form, Message: "pick one",
		ExpiresAt: time.Now().Add(time.Minute),
	})
	if err != nil {
		t.Fatal(err)
	}
	if b.HasAudience(req, elicit.DefaultAudienceWindow) {
		t.Fatal("audience present before any answerer acted")
	}
	if err := b.Respond(elicit.Answer{ID: req.ID, Action: elicit.Decline, By: "test"}); err != nil {
		t.Fatal(err)
	}
	if !b.HasAudience(req, elicit.DefaultAudienceWindow) {
		t.Fatal("audience absent after an answer was recorded")
	}
}
