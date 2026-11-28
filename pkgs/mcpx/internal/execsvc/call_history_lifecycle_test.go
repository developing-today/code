package execsvc

import (
	"testing"
	"time"
)

// A daemon run's history starts expiring when its stream unregisters, even
// though daemon runs never ask for History themselves.
func TestCallHistoryExpiresAfterDaemonRunUnregisters(t *testing.T) {
	r := NewCallRegistry()
	unregister := r.Register("run", func(Frame) {})
	r.ReportCall("run", "srv", "tool", nil, time.Millisecond, nil, nil)
	unregister()

	r.mu.Lock()
	h := r.history["run"]
	if h == nil || h.active {
		r.mu.Unlock()
		t.Fatalf("history after unregister = %+v, want an inactive entry", h)
	}
	h.updated = time.Now().Add(-callHistoryTTL - time.Second)
	r.pruneHistoryLocked(time.Now())
	_, kept := r.history["run"]
	r.mu.Unlock()
	if kept {
		t.Fatal("a finished run's history outlived its TTL")
	}
}

// Unregistering an old run must not end a newer run that reused its id.
func TestUnregisterLeavesAReplacementHistoryActive(t *testing.T) {
	r := NewCallRegistry()
	unregister := r.Register("run", func(Frame) {})
	r.BeginHistory("run")
	unregister()
	r.mu.Lock()
	defer r.mu.Unlock()
	if h := r.history["run"]; h == nil || !h.active {
		t.Fatalf("replacement history = %+v, want it still active", h)
	}
}

// A local run that never takes its snapshot is still bounded.
func TestAbandonedActiveHistoryIsPruned(t *testing.T) {
	r := NewCallRegistry()
	r.BeginHistory("local")
	r.mu.Lock()
	defer r.mu.Unlock()
	r.history["local"].updated = time.Now().Add(-callHistoryIdleTTL - time.Second)
	r.pruneHistoryLocked(time.Now())
	if _, kept := r.history["local"]; kept {
		t.Fatal("an abandoned active history was never pruned")
	}
}
