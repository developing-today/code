package main

import (
	"os"
	"path/filepath"
	"strings"
	"testing"
	"time"

	"github.com/developing-today/code/pkgs/fleet/placement"
)

func writeHardStopsFixture(t *testing.T, contents string) string {
	t.Helper()
	path := filepath.Join(t.TempDir(), "hard-stops.json")
	if err := os.WriteFile(path, []byte(contents), 0o600); err != nil {
		t.Fatalf("write hard-stops fixture: %v", err)
	}
	return path
}

// TestApplyHardStopStopsAStoppedPool is the seating-path half of issue
// #2732 proposal step 2: a steward hard-stop entry for the seating's
// pool sets pool_hard_stop in the answers, so evaluation refuses with
// the hard-stop line the weekly audit counts — even when the monthly
// read is far below the 97% threshold.
// Oracle: each writer action is reflected by the existing seating-path reader,
// while a stop already carried in answers remains sticky after a registry clear.
func TestApplyHardStopConsumesWriterLifecycle(t *testing.T) {
	path := filepath.Join(t.TempDir(), "hard-stops.json")
	for _, stopped := range []bool{true, true, false, true, false} {
		if err := placement.WriteHardStop(path, "pool-a", "steward lifecycle action", stopped, time.Now()); err != nil {
			t.Fatal(err)
		}
		answers := placement.Answers{PoolID: "pool-a"}
		if err := applyHardStop(&answers, path); err != nil {
			t.Fatal(err)
		}
		if answers.PoolHardStop != stopped {
			t.Fatalf("seating reader got %v, want %v", answers.PoolHardStop, stopped)
		}
		sticky := placement.Answers{PoolID: "pool-a", PoolHardStop: true}
		if err := applyHardStop(&sticky, path); err != nil {
			t.Fatal(err)
		}
		if !sticky.PoolHardStop {
			t.Fatal("writer cleared a stop already recorded in answers")
		}
	}
}

func TestApplyHardStopStopsAStoppedPool(t *testing.T) {
	path := writeHardStopsFixture(t, `{"version":1,"stops":{"chatgpt-plan":{"stopped":true,"note":"steward hard stop after the 93%->97% jump","issuedAt":"2026-10-29T13:22:00Z"}}}`)
	fresh := time.Date(2026, 11, 23, 15, 50, 0, 0, time.UTC)
	pct := 42.0
	answers := placement.Answers{
		Task:           "queue task test / issue #2732",
		PoolID:         "chatgpt-plan",
		PoolReadAt:     &fresh,
		PoolMonthlyPct: &pct,
	}
	if err := applyHardStop(&answers, path); err != nil {
		t.Fatalf("applyHardStop: %v", err)
	}
	if !answers.PoolHardStop {
		t.Fatal("hard-stop entry did not set pool_hard_stop")
	}
}

// TestApplyHardStopLeavesAnUnstoppedPoolSeating pins the negative: a
// pool with no registry entry seats on its monthly read alone.
func TestApplyHardStopLeavesAnUnstoppedPoolSeating(t *testing.T) {
	path := writeHardStopsFixture(t, `{"version":1,"stops":{"chatgpt-plan":{"stopped":true}}}`)
	fresh := time.Date(2026, 11, 23, 15, 50, 0, 0, time.UTC)
	pct := 42.0
	answers := placement.Answers{
		Task:           "queue task test / issue #2732",
		PoolID:         "opencode-go.popesdrew",
		PoolReadAt:     &fresh,
		PoolMonthlyPct: &pct,
	}
	if err := applyHardStop(&answers, path); err != nil {
		t.Fatalf("applyHardStop: %v", err)
	}
	if answers.PoolHardStop {
		t.Fatal("pool with no hard-stop entry gained pool_hard_stop")
	}
}

// TestApplyHardStopIsSticky pins fail-closed precedence: a hand-supplied
// pool_hard_stop survives a stopped:false entry — the file can add a
// stop, never clear one recorded elsewhere.
func TestApplyHardStopIsSticky(t *testing.T) {
	path := writeHardStopsFixture(t, `{"version":1,"stops":{"chatgpt-plan":{"stopped":false}}}`)
	answers := placement.Answers{PoolID: "chatgpt-plan", PoolHardStop: true}
	if err := applyHardStop(&answers, path); err != nil {
		t.Fatalf("applyHardStop: %v", err)
	}
	if !answers.PoolHardStop {
		t.Fatal("stopped:false entry cleared a recorded hard-stop")
	}
}

// TestApplyHardStopNeedsAPool pins the no-unguarded-seating rule: with
// --hard-stops supplied but no pool identity anywhere, the dispatch
// fails instead of skipping the check.
func TestApplyHardStopNeedsAPool(t *testing.T) {
	path := writeHardStopsFixture(t, `{"version":1,"stops":{}}`)
	if err := applyHardStop(&placement.Answers{}, path); err == nil {
		t.Fatal("expected an error when no pool identity exists to check, got nil")
	}
}

// TestApplyHardStopFailsLoudOnUnreadableFile pins fail-closed I/O: a
// missing registry file is an error, never a silent "not stopped".
func TestApplyHardStopFailsLoudOnUnreadableFile(t *testing.T) {
	answers := placement.Answers{PoolID: "chatgpt-plan"}
	err := applyHardStop(&answers, filepath.Join(t.TempDir(), "absent.json"))
	if err == nil || !strings.Contains(err.Error(), "hard-stops") {
		t.Fatalf("expected a hard-stops read error, got %v", err)
	}
	if answers.PoolHardStop {
		t.Fatal("failed registry read must not set pool_hard_stop")
	}
}
