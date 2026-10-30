package cache

import (
	"os"
	"path/filepath"
	"testing"
)

// TestRemoveAllRechecksLeaseAfterSelection is the deterministic
// scan-lease-evict case (#2566 finding 3): victims are selected from a
// Scan snapshot while the entry is unleased, the entry is then leased,
// and removal must skip it because the lease state is re-checked at
// removal time.
func TestRemoveAllRechecksLeaseAfterSelection(t *testing.T) {
	c := Open(t.TempDir())
	p, _, err := c.Put("deps", KeyString("victim"), Limits{}, func(dir string) error {
		return os.WriteFile(filepath.Join(dir, "payload"), []byte("data"), 0o600)
	})
	if err != nil {
		t.Fatal(err)
	}
	entries, err := c.List("deps", 0)
	if err != nil {
		t.Fatal(err)
	}
	victims := SelectVictims(entries, Limits{MaxBytes: 1}, "")
	if len(victims) != 1 || victims[0].Path != p {
		t.Fatalf("victims = %v, want exactly the scanned entry", victims)
	}
	// The lease lands after the victim scan — the TOCTOU window.
	lease, err := Acquire(p)
	if err != nil {
		t.Fatal(err)
	}
	if removed := removeAll(victims, 0); len(removed) != 0 {
		t.Fatalf("removeAll removed %d entries leased after the scan", len(removed))
	}
	if _, err := os.Stat(p); err != nil {
		t.Fatalf("entry leased after the scan was deleted: %v", err)
	}
	lease.Release()
	if removed := removeAll(victims, 0); len(removed) != 1 {
		t.Fatalf("after release, removeAll removed %d entries, want 1", len(removed))
	}
}
