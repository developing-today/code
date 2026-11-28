package daemon

import (
	"testing"
	"time"

	"github.com/developing-today/code/pkgs/mcpx/internal/elicit"
)

// An expired confirmation is silence, whatever action the broker filled in
// for it; a person's answer is reported as that answer.
func TestRefusalReasonTellsExpiryFromAnAnswer(t *testing.T) {
	cases := []struct {
		ans  elicit.Answer
		want string
	}{
		{elicit.Answer{Action: elicit.Cancel, By: "expiry"}, "nobody answered within 1s"},
		{elicit.Answer{Action: elicit.Cancel}, "the answer was cancel"},
		{elicit.Answer{Action: elicit.Decline}, "the answer was decline"},
	}
	for _, c := range cases {
		if got := refusalReason(c.ans, time.Second); got != c.want {
			t.Errorf("refusalReason(%+v) = %q, want %q", c.ans, got, c.want)
		}
	}
}
