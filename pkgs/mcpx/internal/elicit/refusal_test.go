package elicit_test

import (
	"testing"

	"github.com/developing-today/code/pkgs/mcpx/internal/elicit"
)

// #1927 item 6: refusal reason codes are defined once, are pairwise
// distinct, and map the answer shapes a refusal can arrive in.
func TestRefusalCodesAreDistinctAndNonEmpty(t *testing.T) {
	codes := []elicit.RefusalCode{
		elicit.RefusalUnanswered,
		elicit.RefusalDeclined,
		elicit.RefusalCancelled,
		elicit.RefusalDenied,
		elicit.RefusalUnavailable,
		elicit.RefusalAnnotationsUnreadable,
	}
	seen := map[elicit.RefusalCode]bool{}
	for _, c := range codes {
		if c == "" {
			t.Fatal("a refusal code is empty; the empty code means no refusal")
		}
		if seen[c] {
			t.Fatalf("refusal code %q is defined twice", c)
		}
		seen[c] = true
	}
}

func TestRefusalCodeForAnswer(t *testing.T) {
	cases := []struct {
		action elicit.Action
		by     string
		want   elicit.RefusalCode
	}{
		{elicit.Decline, "", elicit.RefusalDeclined},
		{elicit.Cancel, "", elicit.RefusalCancelled},
		// An accept whose content did not grant the confirmation is a
		// denial, not a decline: somebody answered, and the answer was no.
		{elicit.Accept, "", elicit.RefusalDenied},
		// A cancel recorded by the broker's expiry path is not a
		// cancellation: the question went unanswered, which is the
		// unanswered refusal, so callers can tell silence from a
		// person cancelling.
		{elicit.Cancel, "expiry", elicit.RefusalUnanswered},
	}
	for _, tc := range cases {
		if got := elicit.RefusalCodeForAnswer(elicit.Answer{Action: tc.action, By: tc.by}); got != tc.want {
			t.Errorf("RefusalCodeForAnswer(%q by %q) = %q, want %q", tc.action, tc.by, got, tc.want)
		}
	}
}
