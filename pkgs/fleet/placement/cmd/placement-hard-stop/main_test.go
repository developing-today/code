package main

import (
	"os"
	"path/filepath"
	"strings"
	"testing"
	"time"

	"github.com/developing-today/code/pkgs/fleet/placement"
)

func TestRunSetsAndClearsHardStop(t *testing.T) {
	path := filepath.Join(t.TempDir(), "hard-stops.json")
	now := time.Date(2026, 11, 27, 10, 0, 0, 0, time.UTC)
	base := []string{"--file", path, "--pool", "pool-a", "--note", "provider usage near limit"}

	if err := run(append([]string{"--action", "set"}, base...), now); err != nil {
		t.Fatalf("set hard-stop: %v", err)
	}
	stopped, err := placement.ReadHardStops(path, "pool-a")
	if err != nil || !stopped {
		t.Fatalf("ReadHardStops after set = (%v, %v), want (true, nil)", stopped, err)
	}

	clearArgs := append([]string{"--action", "clear", "--note", "provider quota recovered"}, base[:4]...)
	if err := run(clearArgs, now.Add(time.Hour)); err != nil {
		t.Fatalf("clear hard-stop: %v", err)
	}
	stopped, err = placement.ReadHardStops(path, "pool-a")
	if err != nil || stopped {
		t.Fatalf("ReadHardStops after clear = (%v, %v), want (false, nil)", stopped, err)
	}
}

func TestRunUsesExplicitIssuedAt(t *testing.T) {
	path := filepath.Join(t.TempDir(), "hard-stops.json")
	args := []string{"--action=set", "--file=" + path, "--pool=pool-a", "--note=steward decision", "--issued-at=2026-11-26T13:20:00-05:00"}
	if err := run(args, time.Time{}); err != nil {
		t.Fatalf("set with explicit issued-at: %v", err)
	}
	contents, err := os.ReadFile(path)
	if err != nil {
		t.Fatal(err)
	}
	if !strings.Contains(string(contents), "2026-11-26T18:20:00Z") {
		t.Fatalf("issuedAt was not normalized to UTC: %s", contents)
	}
}

func TestRunRejectsMalformedCommandBeforeTouchingRegistry(t *testing.T) {
	path := filepath.Join(t.TempDir(), "hard-stops.json")
	for _, tc := range []struct {
		name string
		args []string
	}{
		{name: "unknown flag", args: []string{"--action", "set", "--file", path, "--pool", "pool-a", "--note", "reason", "--typo"}},
		{name: "stray positional", args: []string{"--action", "set", "--file", path, "--pool", "pool-a", "--note", "reason", "extra"}},
		{name: "empty pool", args: []string{"--action", "set", "--file", path, "--pool=", "--note", "reason"}},
		{name: "empty note", args: []string{"--action", "set", "--file", path, "--pool", "pool-a", "--note="}},
		{name: "empty timestamp", args: []string{"--action", "set", "--file", path, "--pool", "pool-a", "--note", "reason", "--issued-at="}},
		{name: "invalid timestamp", args: []string{"--action", "set", "--file", path, "--pool", "pool-a", "--note", "reason", "--issued-at", "yesterday"}},
	} {
		t.Run(tc.name, func(t *testing.T) {
			if err := run(tc.args, time.Now()); err == nil {
				t.Fatal("run succeeded, want usage error")
			}
			if _, err := os.Stat(path); !os.IsNotExist(err) {
				t.Fatalf("invalid command touched registry: stat error = %v", err)
			}
		})
	}
}
