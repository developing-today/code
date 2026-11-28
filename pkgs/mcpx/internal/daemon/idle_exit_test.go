package daemon

import (
	"testing"
	"time"
)

// TestShouldIdleExitUsesTheSuppliedClock exercises the idle-exit boundary
// with a fake timestamp so the test never waits out the idle duration.
func TestShouldIdleExitUsesTheSuppliedClock(t *testing.T) {
	s := &Server{reg: &Registry{}, idleExit: 4 * time.Hour}
	last := time.Date(2026, 11, 3, 12, 0, 0, 0, time.UTC)
	s.lastReq.Store(last.UnixNano())

	if s.shouldIdleExit(last.Add(4*time.Hour - time.Nanosecond)) {
		t.Fatal("idle exit fired before the idle duration elapsed")
	}
	if !s.shouldIdleExit(last.Add(4 * time.Hour)) {
		t.Fatal("idle exit did not fire at the idle duration with no live instances")
	}
}

func TestShouldIdleExitDisabledOrCommitted(t *testing.T) {
	s := &Server{reg: &Registry{}, idleExit: 4 * time.Hour}
	last := time.Date(2026, 11, 3, 12, 0, 0, 0, time.UTC)
	s.lastReq.Store(last.UnixNano())
	farFuture := last.Add(1000 * time.Hour)

	s.idleExit = 0
	if s.shouldIdleExit(farFuture) {
		t.Fatal("idle exit fired with idleExit disabled")
	}

	s.idleExit = 4 * time.Hour
	s.committed.Store(true)
	if s.shouldIdleExit(farFuture) {
		t.Fatal("idle exit fired after the daemon committed to stay")
	}
}
