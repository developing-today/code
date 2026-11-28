package elicit_test

import (
	"testing"

	"github.com/developing-today/code/pkgs/mcpx/internal/elicit"
)

// Decision 0007 (docs/decisions/0007-elicitation-identity.md) refuses
// identity: the session string a caller sends is a correlation label,
// and answering authority is daemon access. These two tests pin that
// for the conformance rows elicitation-server-no-trust-client-user-id
// and elicitation-state-not-session-id-alone.

// TestAClientSuppliedSessionGrantsNoAuthority: a session or
// user-id-shaped label a client sends changes nothing about who can
// see a question or have an answer accepted. A question raised under a
// forged "admin" label and one raised under no label at all are listed,
// fetched and answered exactly alike, by an answerer that names no
// session, because the broker has nothing to check one against.
func TestAClientSuppliedSessionGrantsNoAuthority(t *testing.T) {
	b := open(t)
	labelled := ask(t, b, elicit.Request{Session: "user:admin", Audience: elicit.ToHuman})
	bare := ask(t, b, elicit.Request{Audience: elicit.ToHuman})

	// Visibility: the unfiltered listing shows both, whatever the label.
	all, err := b.Pending(elicit.Filter{})
	if err != nil {
		t.Fatal(err)
	}
	seen := map[string]bool{}
	for _, r := range all {
		seen[r.ID] = true
	}
	if !seen[labelled.ID] || !seen[bare.ID] {
		t.Fatalf("unfiltered listing hid a question by its label: %v", seen)
	}
	// Fetching by id needs no label either.
	for _, id := range []string{labelled.ID, bare.ID} {
		if _, ok, err := b.Get(id); err != nil || !ok {
			t.Fatalf("Get(%s) = %v, %v; want found", id, ok, err)
		}
	}

	// Answer acceptance: an answer carries no session at all, and one
	// from an answerer whose label matches nothing is accepted for both.
	for _, id := range []string{labelled.ID, bare.ID} {
		if err := b.Respond(elicit.Answer{ID: id, Action: elicit.Decline, By: "someone-else"}); err != nil {
			t.Fatalf("Respond(%s) refused an answer for want of a matching label: %v", id, err)
		}
		a, ok, err := b.Lookup(id)
		if err != nil || !ok {
			t.Fatalf("Lookup(%s) = %v, %v", id, ok, err)
		}
		if a.Action != elicit.Decline || a.By != "someone-else" {
			t.Fatalf("Lookup(%s) = %+v; want the decline as given", id, a)
		}
	}
}

// TestQuestionsSharingASessionStayAddressedById: state is keyed by the
// elicitation id, never by the session alone. Two questions raised
// under the same session string are answered independently: answering
// one settles that one only, and each keeps its own answer.
func TestQuestionsSharingASessionStayAddressedById(t *testing.T) {
	b := open(t)
	first := ask(t, b, elicit.Request{Session: "shared", Message: "first?"})
	second := ask(t, b, elicit.Request{Session: "shared", Message: "second?"})
	if first.ID == second.ID {
		t.Fatalf("two questions in one session share an id %s", first.ID)
	}

	if err := b.Respond(elicit.Answer{ID: first.ID, Action: elicit.Decline, By: "one"}); err != nil {
		t.Fatal(err)
	}

	pending, err := b.Pending(elicit.Filter{Session: "shared"})
	if err != nil {
		t.Fatal(err)
	}
	if len(pending) != 1 || pending[0].ID != second.ID {
		t.Fatalf("after answering %s, pending in the session = %+v; want only %s",
			first.ID, pending, second.ID)
	}
	if _, ok, err := b.Lookup(second.ID); err != nil || ok {
		t.Fatalf("Lookup(%s) = %v, %v; the second question took the first's answer", second.ID, ok, err)
	}

	if err := b.Respond(elicit.Answer{ID: second.ID, Action: elicit.Cancel, By: "two"}); err != nil {
		t.Fatal(err)
	}
	for id, want := range map[string]elicit.Action{first.ID: elicit.Decline, second.ID: elicit.Cancel} {
		a, ok, err := b.Lookup(id)
		if err != nil || !ok || a.Action != want {
			t.Fatalf("Lookup(%s) = %+v, %v, %v; want %s", id, a, ok, err, want)
		}
	}
}
