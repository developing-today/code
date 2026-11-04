package daemon

import (
	"sync"
	"time"
)

// limitCheck is one fixed-window limit an admission must satisfy: at most
// Limit admissions per minute for Key. A Limit of 0 or less disables the
// check. Name and What exist so a refusal can name the limit that ran
// out, in the caller's own terms.
type limitCheck struct {
	Name  string // the setting that carries the limit, e.g. "completion.perMinute"
	What  string // who the limit binds, e.g. "this caller and server"
	Key   string
	Limit int
}

// rateLimiter is a fixed-window, in-memory admission counter over
// several limits at once, in the shape design #2386 asked for: keys and
// limits in, verdict out, nothing completion-shaped about it, so the
// tasks-rate-limit question (#1554) can reuse it later. It follows the
// fixed-window precedent of tasks.Window (#2333) with one difference
// that completion needs: admission is atomic across all the checks --
// count-on-admission means a request refused by ANY window consumes
// quota in NONE of them, so the checks are all read first and only then
// all counted.
//
// The zero value is ready to use.
type rateLimiter struct {
	mu        sync.Mutex
	buckets   map[string]*rateBucket
	lastSweep time.Time
	now       func() time.Time // tests substitute a fake clock
}

type rateBucket struct {
	start time.Time
	n     int
}

// admit counts one admission against every enabled check and returns
// nil, or returns the first check that refused -- having counted
// nothing.
func (l *rateLimiter) admit(checks []limitCheck) *limitCheck {
	now := time.Now()
	if l.now != nil {
		now = l.now()
	}
	l.mu.Lock()
	defer l.mu.Unlock()
	if l.buckets == nil {
		l.buckets = map[string]*rateBucket{}
	}
	// Phase 1: every enabled check must have room.
	for i := range checks {
		c := &checks[i]
		if c.Limit <= 0 {
			continue
		}
		if b, ok := l.buckets[c.Key]; ok && now.Sub(b.start) < time.Minute && b.n >= c.Limit {
			refused := *c
			return &refused
		}
	}
	// Phase 2: count the admission in every enabled check.
	for i := range checks {
		c := &checks[i]
		if c.Limit <= 0 {
			continue
		}
		b, ok := l.buckets[c.Key]
		if !ok || now.Sub(b.start) >= time.Minute {
			// Sweep at most once per minute, as tasks.Window does, so a
			// long-lived daemon cannot accumulate buckets past their
			// windows.
			if now.Sub(l.lastSweep) >= time.Minute {
				for k, v := range l.buckets {
					if now.Sub(v.start) >= time.Minute {
						delete(l.buckets, k)
					}
				}
				l.lastSweep = now
			}
			l.buckets[c.Key] = &rateBucket{start: now, n: 1}
			continue
		}
		b.n++
	}
	return nil
}
