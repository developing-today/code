package elicit

import (
	"os"
	"time"

	"github.com/developing-today/code/pkgs/mcpx/internal/defaults"
)

// Answerer presence: the broker's truthful no-audience signal (#2438,
// Phase 3 of #2434, step 1).
//
// Route says who SHOULD answer a question; it cannot say whether anyone
// who could answer is actually there. Answerers are out-of-process --
// the mcpx elicit CLI, the opencode plugin, a dashboard -- and poll the
// broker for work. Those answerer-facing surfaces heartbeat here
// (NoteAnswerer) so the broker can answer "does any audience exist for
// this question" from recorded presence instead of guessing. Presence
// is stored in the broker's own database, so a heartbeat from any
// process is visible to every other process holding the same store.
//
// Presence is evidence, not a promise: an answerer seen inside the
// window may still decline or vanish, which the ordinary Await expiry
// already handles. Absence inside the window is the signal the ask
// seam needs: nobody is polling for this audience, so a question
// routed there cannot be answered and waiting out the TTL is not an
// answer.

// DefaultAudienceWindow is how recent an answerer heartbeat must be
// for the audience to count as present. The value lives in
// defaults.json (elicit.audienceWindow, 2m): answerer surfaces poll on
// a one-second cadence (mcpx elicit watch) or per user refresh, so two
// minutes is generous without letting a long-gone answerer count.
var DefaultAudienceWindow = defaults.ElicitAudienceWindow

// NoteAnswerer records that an answerer able to serve aud was seen
// now. An empty audience means an unfiltered answerer -- one polling
// for every audience -- and counts for both. Internal broker consumers
// (the daemon's own bookkeeping reads) must NOT call this: presence is
// only truthful when it comes from a surface an answerer drives.
func (b *Broker) NoteAnswerer(aud Audience) error {
	now := time.Now().UnixMilli()
	upsert := `INSERT INTO elicitation_answerers (audience, last_seen_ms, pid)
		VALUES (?, ?, ?)
		ON CONFLICT(audience) DO UPDATE SET last_seen_ms = excluded.last_seen_ms, pid = excluded.pid`
	if aud == "" {
		for _, a := range []Audience{ToAgent, ToHuman} {
			if _, err := b.db.Exec(upsert, string(a), now, os.Getpid()); err != nil {
				return err
			}
		}
		return nil
	}
	_, err := b.db.Exec(upsert, string(aud), now, os.Getpid())
	return err
}

// AnswererSeen returns when an answerer for aud was last recorded, and
// whether that is inside within of now.
func (b *Broker) AnswererSeen(aud Audience, within time.Duration) (time.Time, bool, error) {
	var ms int64
	err := b.db.QueryRow(
		`SELECT last_seen_ms FROM elicitation_answerers WHERE audience = ?`,
		string(aud)).Scan(&ms)
	if err != nil {
		// No row is absence, not an error.
		return time.Time{}, false, nil
	}
	at := time.UnixMilli(ms)
	return at, time.Since(at) <= within, nil
}

// HasAudience reports whether the audience a question routes to has an
// answerer present inside within. The question's recorded audience
// wins; an unrouted question is routed on the spot, the same rule
// Pending applies on read. A store error is reported as presence --
// the signal must never invent an absence it did not observe.
func (b *Broker) HasAudience(r Request, within time.Duration) bool {
	aud := r.Audience
	if aud == "" {
		aud, _ = Route(r)
	}
	_, present, err := b.AnswererSeen(aud, within)
	if err != nil {
		return true
	}
	return present
}
