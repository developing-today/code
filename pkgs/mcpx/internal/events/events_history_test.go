package events_test

import (
	"testing"

	"github.com/dezren39/mcpx/internal/events"
)

// The retained history is what "reconnect from zero" promises. The existing
// regression test replays 300 events, more than the subscriber buffer but
// well short of the history the daemon actually keeps; a full history is the
// case where the buffer-sizing fix has to hold for every retained event, and
// where "no gap" has to mean all 1024, not "all that fitted".
func TestAFullRetainedHistoryReplaysAtSinceZero(t *testing.T) {
	const keep = 1024
	b := events.New(keep)
	for i := 0; i < keep; i++ {
		b.Publish(events.Event{Kind: events.CallFinished})
	}

	s, gap := b.SubscribeFrom(events.Filter{}, 0, true)
	defer s.Close()
	if gap {
		t.Fatalf("replaying the full retained history from zero reported a gap")
	}
	if got := s.Dropped.Load(); got != 0 {
		t.Fatalf("full-history replay dropped %d events", got)
	}

	for want := uint64(1); want <= keep; want++ {
		e := recv(t, s)
		if e.Seq != want {
			t.Fatalf("event %d of the replay has seq %d", want, e.Seq)
		}
	}
	none(t, s)
}
