package placement

import (
	"encoding/json"
	"fmt"
	"os"
	"path/filepath"
	"sync"
	"testing"
	"testing/quick"
	"time"
)

func TestWriteHardStopCreatesAndUpdatesPoolEntry(t *testing.T) {
	path := filepath.Join(t.TempDir(), "hard-stops.json")
	issuedAt := time.Date(2026, 11, 27, 10, 0, 0, 0, time.UTC)

	if err := WriteHardStop(path, "pool-a", "monthly use is near the wall", true, issuedAt); err != nil {
		t.Fatalf("write initial stop: %v", err)
	}
	stopped, err := ReadHardStops(path, "pool-a")
	if err != nil || !stopped {
		t.Fatalf("ReadHardStops after set = (%v, %v), want (true, nil)", stopped, err)
	}

	updatedAt := issuedAt.Add(time.Hour)
	if err := WriteHardStop(path, "pool-a", "steward confirmed stop", true, updatedAt); err != nil {
		t.Fatalf("update stop: %v", err)
	}
	var doc hardStopsDoc
	contents, err := os.ReadFile(path)
	if err != nil {
		t.Fatalf("read registry: %v", err)
	}
	if err := json.Unmarshal(contents, &doc); err != nil {
		t.Fatalf("parse registry: %v", err)
	}
	entry := doc.Stops["pool-a"]
	if doc.Version != 1 || !entry.Stopped || entry.Note != "steward confirmed stop" || entry.IssuedAt != updatedAt.Format(time.RFC3339) {
		t.Fatalf("updated entry = %#v (version %d), want latest auditable stop", entry, doc.Version)
	}
}

func TestWriteHardStopClearKeepsAuditablePoolEntry(t *testing.T) {
	path := filepath.Join(t.TempDir(), "hard-stops.json")
	issuedAt := time.Date(2026, 11, 27, 10, 0, 0, 0, time.UTC)
	if err := WriteHardStop(path, "pool-a", "steward stop", true, issuedAt); err != nil {
		t.Fatalf("set stop: %v", err)
	}
	clearedAt := issuedAt.Add(time.Hour)
	if err := WriteHardStop(path, "pool-a", "provider quota recovered", false, clearedAt); err != nil {
		t.Fatalf("clear stop: %v", err)
	}
	stopped, err := ReadHardStops(path, "pool-a")
	if err != nil || stopped {
		t.Fatalf("ReadHardStops after clear = (%v, %v), want (false, nil)", stopped, err)
	}
	contents, err := os.ReadFile(path)
	if err != nil {
		t.Fatalf("read registry: %v", err)
	}
	var doc hardStopsDoc
	if err := json.Unmarshal(contents, &doc); err != nil {
		t.Fatalf("parse registry: %v", err)
	}
	entry, exists := doc.Stops["pool-a"]
	if !exists || entry.Stopped || entry.Note != "provider quota recovered" || entry.IssuedAt != clearedAt.Format(time.RFC3339) {
		t.Fatalf("cleared entry = %#v (exists %v), want retained audited false entry", entry, exists)
	}
}

func TestWriteHardStopPreservesOtherPoolsAndRejectsInvalidState(t *testing.T) {
	path := filepath.Join(t.TempDir(), "hard-stops.json")
	issuedAt := time.Date(2026, 11, 27, 10, 0, 0, 0, time.UTC)
	if err := WriteHardStop(path, "pool-a", "first stop", true, issuedAt); err != nil {
		t.Fatalf("write first pool: %v", err)
	}
	if err := WriteHardStop(path, "pool-b", "second stop", true, issuedAt); err != nil {
		t.Fatalf("write second pool: %v", err)
	}
	for _, tc := range []struct {
		pool string
		note string
		at   time.Time
	}{
		{pool: " ", note: "reason", at: issuedAt},
		{pool: "pool-c", note: "  ", at: issuedAt},
		{pool: "pool-c", note: "reason"},
	} {
		if err := WriteHardStop(path, tc.pool, tc.note, true, tc.at); err == nil {
			t.Errorf("WriteHardStop(%q, %q, %v) succeeded, want validation error", tc.pool, tc.note, tc.at)
		}
	}
	for _, pool := range []string{"pool-a", "pool-b"} {
		stopped, err := ReadHardStops(path, pool)
		if err != nil || !stopped {
			t.Errorf("existing %s entry changed after invalid writes: (%v, %v)", pool, stopped, err)
		}
	}
}

// Oracle: invalid/unknown registry data must remain byte-for-byte unchanged.
func TestWriteHardStopRejectsMalformedExistingRegistryWithoutReplacingIt(t *testing.T) {
	for _, before := range []string{
		`{"version":2,"stops":{}}`, `{}`, `null`, `{"stops":{}}`,
		`{"version":1}`, `{"version":1,"stops":null}`,
		`{"version":1,"stops":{},"futureField":true}`,
		`{"version":1,"stops":{}} {}`, `{`,
	} {
		t.Run(before, func(t *testing.T) {
			path := filepath.Join(t.TempDir(), "hard-stops.json")
			if err := os.WriteFile(path, []byte(before), 0o600); err != nil {
				t.Fatal(err)
			}
			if err := WriteHardStop(path, "pool-a", "reason", true, time.Now()); err == nil {
				t.Fatal("WriteHardStop accepted invalid registry")
			}
			after, err := os.ReadFile(path)
			if err != nil {
				t.Fatal(err)
			}
			if string(after) != before {
				t.Fatalf("invalid registry was modified: %s", after)
			}
		})
	}
}

// Oracle: a simple last-write-per-pool map models arbitrary set/clear sequences.
func TestWriteHardStopMatchesLifecycleModel(t *testing.T) {
	property := func(actions []uint8) bool {
		path := filepath.Join(t.TempDir(), "hard-stops.json")
		model := make(map[string]bool)
		at := time.Date(2026, 11, 27, 10, 0, 0, 0, time.UTC)
		for _, action := range actions {
			pool := fmt.Sprintf("pool-%d", action%4)
			stopped := action&4 != 0
			if err := WriteHardStop(path, pool, "model action", stopped, at); err != nil {
				return false
			}
			model[pool] = stopped
			for key, want := range model {
				got, err := ReadHardStops(path, key)
				if err != nil || got != want {
					return false
				}
			}
		}
		return true
	}
	if err := quick.Check(property, &quick.Config{MaxCount: 30}); err != nil {
		t.Fatal(err)
	}
}

// Oracle: independent concurrent pool updates must all survive the registry commit.
func TestWriteHardStopSerializesConcurrentWriters(t *testing.T) {
	path := filepath.Join(t.TempDir(), "hard-stops.json")
	var wg sync.WaitGroup
	for i := 0; i < 24; i++ {
		wg.Add(1)
		go func(i int) {
			defer wg.Done()
			if err := WriteHardStop(path, fmt.Sprintf("pool-%d", i), "concurrent stop", true, time.Now()); err != nil {
				t.Error(err)
			}
		}(i)
	}
	wg.Wait()
	for i := 0; i < 24; i++ {
		pool := fmt.Sprintf("pool-%d", i)
		got, err := ReadHardStops(path, pool)
		if err != nil || !got {
			t.Errorf("lost stop for %s: (%v, %v)", pool, got, err)
		}
	}
}
