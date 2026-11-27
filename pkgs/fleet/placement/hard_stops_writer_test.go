package placement

import (
	"encoding/json"
	"os"
	"path/filepath"
	"testing"
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

func TestWriteHardStopRejectsMalformedExistingRegistryWithoutReplacingIt(t *testing.T) {
	path := filepath.Join(t.TempDir(), "hard-stops.json")
	before := []byte(`{"version":2,"stops":{}}`)
	if err := os.WriteFile(path, before, 0o600); err != nil {
		t.Fatal(err)
	}
	if err := WriteHardStop(path, "pool-a", "reason", true, time.Now()); err == nil {
		t.Fatal("WriteHardStop accepted unsupported registry version")
	}
	after, err := os.ReadFile(path)
	if err != nil {
		t.Fatal(err)
	}
	if string(after) != string(before) {
		t.Fatalf("invalid registry was modified: %s", after)
	}
}
