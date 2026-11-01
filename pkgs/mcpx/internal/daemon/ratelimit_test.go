package daemon

import (
	"testing"
	"time"
)

// #2387: the completion limiter's unit contract -- fixed windows,
// count-on-admission, per-key independence, 0 disables. The clock is
// fake so rollover is exact.

type fakeClock struct{ t time.Time }

func (c *fakeClock) now() time.Time          { return c.t }
func (c *fakeClock) advance(d time.Duration) { c.t = c.t.Add(d) }

func callerCheck(key string, limit int) limitCheck {
	return limitCheck{Name: "completion.perMinute", What: "this caller and server", Key: key, Limit: limit}
}

func TestRateLimiterAdmitsUpToLimitThenRefuses(t *testing.T) {
	l := &rateLimiter{}
	for i := 0; i < 3; i++ {
		if r := l.admit([]limitCheck{callerCheck("k", 3)}); r != nil {
			t.Fatalf("admission %d refused by %s, want admitted", i+1, r.Name)
		}
	}
	if r := l.admit([]limitCheck{callerCheck("k", 3)}); r == nil || r.Name != "completion.perMinute" {
		t.Fatalf("4th admission: refused = %v, want completion.perMinute", r)
	}
}

func TestRateLimiterWindowRollover(t *testing.T) {
	clock := &fakeClock{t: time.Unix(1_700_000_000, 0)}
	l := &rateLimiter{now: clock.now}
	if r := l.admit([]limitCheck{callerCheck("k", 1)}); r != nil {
		t.Fatal("first admission refused")
	}
	if r := l.admit([]limitCheck{callerCheck("k", 1)}); r == nil {
		t.Fatal("second admission in the same window admitted, want refused")
	}
	clock.advance(59 * time.Second)
	if r := l.admit([]limitCheck{callerCheck("k", 1)}); r == nil {
		t.Fatal("admission at +59s admitted, want still refused")
	}
	clock.advance(2 * time.Second)
	if r := l.admit([]limitCheck{callerCheck("k", 1)}); r != nil {
		t.Fatal("admission after the window rolled over refused, want admitted")
	}
}

func TestRateLimiterCountOnAdmission(t *testing.T) {
	clock := &fakeClock{t: time.Unix(1_700_000_000, 0)}
	l := &rateLimiter{now: clock.now}
	// Two windows over one request: caller limit 2, server limit 1.
	checks := func() []limitCheck {
		return []limitCheck{
			callerCheck("caller", 2),
			{Name: "completion.serverPerMinute", What: "this server", Key: "server", Limit: 1},
		}
	}
	if r := l.admit(checks()); r != nil {
		t.Fatalf("first request refused by %s", r.Name)
	}
	// The server window is now full; the caller window has one left.
	// Refusals must not consume the caller window's remaining quota...
	for i := 0; i < 5; i++ {
		if r := l.admit(checks()); r == nil || r.Name != "completion.serverPerMinute" {
			t.Fatalf("refusal %d: got %v, want serverPerMinute refusal", i, r)
		}
	}
	// ...which the next window proves: after rollover the caller still
	// has its full budget of 2 against a fresh server window.
	clock.advance(61 * time.Second)
	if r := l.admit(checks()); r != nil {
		t.Fatalf("after rollover refused by %s", r.Name)
	}
}

func TestRateLimiterPerKeyIndependence(t *testing.T) {
	l := &rateLimiter{}
	if r := l.admit([]limitCheck{callerCheck("a", 1)}); r != nil {
		t.Fatal("a's first admission refused")
	}
	if r := l.admit([]limitCheck{callerCheck("a", 1)}); r == nil {
		t.Fatal("a's second admission admitted, want refused")
	}
	if r := l.admit([]limitCheck{callerCheck("b", 1)}); r != nil {
		t.Fatal("b's first admission refused: keys must be independent")
	}
}

func TestRateLimiterZeroDisables(t *testing.T) {
	l := &rateLimiter{}
	for i := 0; i < 100; i++ {
		if r := l.admit([]limitCheck{callerCheck("k", 0)}); r != nil {
			t.Fatal("limit 0 must disable the window")
		}
	}
}
