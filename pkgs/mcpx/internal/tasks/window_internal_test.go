package tasks

import (
	"testing"
	"time"
)

// Expired requestor buckets are evicted when a new bucket is created, so
// distinct requestors on a long-lived daemon cannot grow the window map
// past their windows.
func TestWindowEvictsExpiredBuckets(t *testing.T) {
	w := NewWindow(10)
	start := time.Now()
	if !w.Allow("alice", start) || !w.Allow("bob", start) {
		t.Fatal("first calls are admitted")
	}
	if got := len(w.counts); got != 2 {
		t.Fatalf("two buckets are tracked, got %d", got)
	}
	later := start.Add(2 * time.Minute)
	if !w.Allow("carol", later) {
		t.Fatal("a new key after the window is admitted")
	}
	if got := len(w.counts); got != 1 {
		t.Errorf("expired buckets are evicted when a new one is created, got %d tracked", got)
	}
}
