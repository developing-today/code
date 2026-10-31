package elicit

// Pins #1691's knock-on: an expiry that loses the race hands over the answer
// that landed instead of failing the asker. Internal so the expiry attempt
// itself is exercised deterministically — driving it through Await would need
// the answer to land inside the ticker's microsecond check-then-claim window.

import (
	"path/filepath"
	"testing"
)

func TestExpireHandsOverAnAnswerThatLandedFirst(t *testing.T) {
	dir := t.TempDir()
	path := filepath.Join(dir, "e.db")

	a, err := Open(path)
	if err != nil {
		t.Fatal(err)
	}
	defer a.Close()
	b, err := Open(path)
	if err != nil {
		t.Fatal(err)
	}
	defer b.Close()

	r, err := a.OpenRequest(Request{Message: "which one?"})
	if err != nil {
		t.Fatal(err)
	}

	// A real answer claims the question first, from the other handle.
	want := Answer{ID: r.ID, Action: Decline, By: "cli"}
	if err := b.Respond(want); err != nil {
		t.Fatal(err)
	}

	// The expiry loses the race: the asker still gets the landed answer.
	got, err := a.expire(r.ID)
	if err != nil {
		t.Fatalf("expire lost the race and failed: %v", err)
	}
	if got.Action != Decline || got.By != "cli" {
		t.Errorf("expire returned %+v, want the landed decline-by-cli", got)
	}
}

func TestExpireCancelsAnUnclaimedQuestion(t *testing.T) {
	a, err := Open(filepath.Join(t.TempDir(), "e.db"))
	if err != nil {
		t.Fatal(err)
	}
	defer a.Close()

	r, err := a.OpenRequest(Request{Message: "which one?"})
	if err != nil {
		t.Fatal(err)
	}

	got, err := a.expire(r.ID)
	if err != nil {
		t.Fatal(err)
	}
	if got.Action != Cancel || got.By != "expiry" {
		t.Errorf("expire returned %+v, want the expiry cancel", got)
	}
}
