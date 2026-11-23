package placement

import (
	"strings"
	"testing"
	"time"
)

// TestMonthlyPaceGuard is the first slice of issue #2732: a seating attempt
// against a pool at/above the 97% monthly guard, with a trailing pace above
// 1pp/h toward a wall, or under a steward hard-stop, is refused (with the
// pool, the % read, and the guard source in the refusal) instead of seating
// silently. Unknown state never refuses: a B4 reliance and a non-pool
// option proceed, per burn rules B4/B5.
func TestMonthlyPaceGuard(t *testing.T) {
	p := testPolicy(t)
	now := time.Date(2026, 11, 22, 3, 0, 0, 0, time.UTC)
	fresh := now.Add(-10 * time.Minute)

	pct := func(f float64) *float64 { return &f }
	pace := func(f float64) *float64 { return &f }

	judgement := func(pool string, monthly *float64, pph *float64, hardStop bool) Answers {
		return Answers{
			Task:           "queue task test / issue #2732",
			Q4Judgement:    true,
			PoolReadAt:     &fresh,
			PoolID:         pool,
			PoolMonthlyPct: monthly,
			PoolPacePph:    pph,
			PoolHardStop:   hardStop,
		}
	}

	t.Run("monthly at 97 refuses", func(t *testing.T) {
		_, err := Evaluate(p, judgement("chatgpt-plan", pct(97.0), pace(0.1), false), now)
		if err == nil {
			t.Fatal("expected a monthly-guard refusal at 97%")
		}
		msg := err.Error()
		for _, want := range []string{"chatgpt-plan", "97", "monthly"} {
			if !strings.Contains(msg, want) {
				t.Fatalf("refusal %q does not cite %q (pool, %%, guard source)", msg, want)
			}
		}
	})

	t.Run("monthly below 97 seats", func(t *testing.T) {
		rec, err := Evaluate(p, judgement("chatgpt-plan", pct(96.9), pace(0.1), false), now)
		if err != nil {
			t.Fatalf("Evaluate below the guard: %v", err)
		}
		if rec.Option != "Muse" || rec.Rule != "R6" {
			t.Fatalf("got %s/%s, want Muse/R6", rec.Option, rec.Rule)
		}
	})

	t.Run("pace above 1pp/h refuses", func(t *testing.T) {
		_, err := Evaluate(p, judgement("meta-pool", pct(90.0), pace(1.2), false), now)
		if err == nil {
			t.Fatal("expected a pace-guard refusal above 1pp/h")
		}
		msg := err.Error()
		for _, want := range []string{"meta-pool", "pace"} {
			if !strings.Contains(msg, want) {
				t.Fatalf("refusal %q does not cite %q", msg, want)
			}
		}
	})

	t.Run("pace at 1pp/h seats", func(t *testing.T) {
		if _, err := Evaluate(p, judgement("meta-pool", pct(90.0), pace(1.0), false), now); err != nil {
			t.Fatalf("Evaluate at exactly 1pp/h: %v", err)
		}
	})

	t.Run("steward hard-stop refuses at any pct", func(t *testing.T) {
		_, err := Evaluate(p, judgement("plan-routes", pct(42.0), pace(0.0), true), now)
		if err == nil {
			t.Fatal("expected a hard-stop refusal")
		}
		if msg := err.Error(); !strings.Contains(msg, "hard-stop") {
			t.Fatalf("refusal %q does not cite its guard source", msg)
		}
	})

	t.Run("B4 reliance proceeds despite the guard", func(t *testing.T) {
		stale := now.Add(-2 * time.Hour)
		rec, err := Evaluate(p, Answers{
			Task:                 "queue task test / issue #2732",
			Q4Judgement:          true,
			PoolReadAt:           &stale,
			PoolReadUnobtainable: true,
			PoolID:               "chatgpt-plan",
			PoolMonthlyPct:       pct(99.5),
			PoolHardStop:         true,
		}, now)
		if err != nil {
			t.Fatalf("B4 reliance must never refuse (B4 governs, not the guard): %v", err)
		}
		if !rec.B4Relied {
			t.Fatal("record does not carry the B4 reliance")
		}
	})

	t.Run("non-pool option ignores guard inputs", func(t *testing.T) {
		rec, err := Evaluate(p, Answers{
			Q1Mechanical:   true,
			PoolID:         "chatgpt-plan",
			PoolMonthlyPct: pct(99.5),
			PoolHardStop:   true,
		}, now)
		if err != nil {
			t.Fatalf("non-pool placement must ignore the monthly guard: %v", err)
		}
		if rec.PoolReadAt != "n/a" {
			t.Fatalf("pool_read_at = %q, want n/a", rec.PoolReadAt)
		}
	})

	t.Run("no monthly data seats on a fresh read", func(t *testing.T) {
		if _, err := Evaluate(p, Answers{
			Q4Judgement: true,
			PoolReadAt:  &fresh,
		}, now); err != nil {
			t.Fatalf("a fresh read with no monthly data must seat: %v", err)
		}
	})

	// Regression (Codex P2 on PR #12895): the README documents that an E1
	// exception bypasses the guard — the exception record itself is the
	// required log line — but buildRecord applied the guard to exception
	// records too, so a recorded exception choosing a pool-drawn option at
	// or above the guard was rejected instead of recorded.
	t.Run("E1 exception bypasses the guard", func(t *testing.T) {
		rec, err := Evaluate(p, Answers{
			Task:            "queue task test / issue #2732",
			ExceptionID:     "E1-2026-11-22-2732",
			ExceptionOption: "Muse",
			PoolReadAt:      &fresh,
			PoolID:          "chatgpt-plan",
			PoolMonthlyPct:  pct(97.5),
			PoolPacePph:     pace(1.2),
			PoolHardStop:    true,
		}, now)
		if err != nil {
			t.Fatalf("a recorded E1 exception must bypass the monthly guard: %v", err)
		}
		if rec.Rule != "exception" || rec.ExceptionID != "E1-2026-11-22-2732" || rec.Option != "Muse" {
			t.Fatalf("bad exception record: %+v", rec)
		}
	})

	// The bypass is the guard only: the quota-read checks (B5 freshness)
	// still bind an exception seating exactly as they bind a ruled one.
	t.Run("E1 exception still bound by the B5 freshness gate", func(t *testing.T) {
		stale := now.Add(-2 * time.Hour)
		_, err := Evaluate(p, Answers{
			Task:            "queue task test / issue #2732",
			ExceptionID:     "E1-2026-11-22-2732",
			ExceptionOption: "Muse",
			PoolReadAt:      &stale,
			PoolID:          "chatgpt-plan",
			PoolMonthlyPct:  pct(97.5),
		}, now)
		if err == nil {
			t.Fatal("expected the B5 freshness gate to refuse a stale read even under an exception")
		}
		if msg := err.Error(); !strings.Contains(msg, "B5 gate") {
			t.Fatalf("refusal %q does not cite the B5 gate", msg)
		}
	})
}
