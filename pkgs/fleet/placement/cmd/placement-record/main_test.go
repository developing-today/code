package main

import (
	"testing"
	"time"

	"github.com/developing-today/code/pkgs/fleet/placement"
)

// TestLiveReadSelection pins the --quota-current/--pool contract of
// issue #2732's wiring slice: the pair is supplied together or not at
// all, and a present-but-valueless option is a usage error, never the
// default (operator argument contract, #3352). The failure shape that
// matters is a dispatcher expanding unset variables —
// --quota-current "$QUOTA_PATH" --pool "$POOL_ID" with both empty —
// which must fail loudly instead of silently recording a seating from
// hand-supplied answers the caller never meant to trust.
func TestLiveReadSelection(t *testing.T) {
	cases := []struct {
		name          string
		quotaSupplied bool
		poolSupplied  bool
		quotaCurrent  string
		pool          string
		wantLiveRead  bool
		wantErr       bool
	}{
		{"neither supplied", false, false, "", "", false, false},
		{"both supplied with values", true, true, "/tmp/quota-current.json", "chatgpt-plan", true, false},
		{"both supplied empty", true, true, "", "", false, true},
		{"quota-current supplied empty, pool valued", true, true, "", "chatgpt-plan", false, true},
		{"quota-current valued, pool supplied empty", true, true, "/tmp/quota-current.json", "", false, true},
		{"quota-current only", true, false, "/tmp/quota-current.json", "", false, true},
		{"pool only", false, true, "", "chatgpt-plan", false, true},
	}
	for _, c := range cases {
		t.Run(c.name, func(t *testing.T) {
			liveRead, err := liveReadSelection(c.quotaSupplied, c.poolSupplied, c.quotaCurrent, c.pool)
			if c.wantErr && err == nil {
				t.Fatalf("expected a usage error, got liveRead=%v", liveRead)
			}
			if !c.wantErr && err != nil {
				t.Fatalf("unexpected error: %v", err)
			}
			if liveRead != c.wantLiveRead {
				t.Fatalf("liveRead = %v, want %v", liveRead, c.wantLiveRead)
			}
		})
	}
}

// TestApplyLiveReadClearsB4Fallback pins the B4 interaction of issue
// #2732's wiring slice: an answers file can carry a
// pool_read_unobtainable B4 reliance from a previous fallback. A
// successful live read replaces the guard inputs, and it must also
// clear that flag — buildRecord returns on a B4 reliance before the
// monthly guard runs, so leaving it set would seat a pool the fresh
// read says is at 98.4% without the guard ever judging it.
func TestApplyLiveReadClearsB4Fallback(t *testing.T) {
	staleAt := time.Date(2026, 11, 23, 10, 0, 0, 0, time.UTC)
	stalePct := 12.0
	answers := placement.Answers{
		Task:                 "queue task test / issue #2732",
		PoolReadUnobtainable: true,
		PoolReadAt:           &staleAt,
		PoolMonthlyPct:       &stalePct,
		PoolID:               "chatgpt-plan",
	}
	readAt := time.Date(2026, 11, 23, 15, 50, 0, 0, time.UTC)
	applyLiveRead(&answers, 98.4, readAt, "chatgpt-plan")
	if answers.PoolReadUnobtainable {
		t.Fatal("PoolReadUnobtainable still set after a successful live read; the B4 fallback would bypass the monthly guard")
	}
	if answers.PoolMonthlyPct == nil || *answers.PoolMonthlyPct != 98.4 {
		t.Fatalf("PoolMonthlyPct = %v, want the live read's 98.4", answers.PoolMonthlyPct)
	}
	if answers.PoolReadAt == nil || !answers.PoolReadAt.Equal(readAt) {
		t.Fatalf("PoolReadAt = %v, want the live read's %s", answers.PoolReadAt, readAt)
	}
	if answers.PoolID != "chatgpt-plan" {
		t.Fatalf("PoolID = %q, want chatgpt-plan", answers.PoolID)
	}
}
