package placement

import (
	"os"
	"path/filepath"
	"testing"
	"time"
)

// testPolicy loads the real placement-policy.yaml from the framework doc
// directory: the PoC is evaluated against the policy of record, not a copy.
func testPolicy(t *testing.T) *Policy {
	t.Helper()
	path := filepath.Join("..", "..", "..",
		"doc", "2026-11-03T15-10-56Z_design_task-placement-framework-v1", "placement-policy.yaml")
	if _, err := os.Stat(path); err != nil {
		t.Skipf("policy of record not present at %s: %v", path, err)
	}
	p, err := Load(path)
	if err != nil {
		t.Fatalf("load policy: %v", err)
	}
	return p
}

func TestEvaluateRules(t *testing.T) {
	p := testPolicy(t)
	now := time.Date(2026, 11, 17, 3, 0, 0, 0, time.UTC)
	fresh := now.Add(-10 * time.Minute)

	cases := []struct {
		name       string
		answers    Answers
		wantOption string
		wantRule   string
	}{
		{"Q1 mechanical goes to an Action", Answers{Q1Mechanical: true}, "github-action", "R1"},
		{"Q2 self-contained goes to Jules", Answers{Q2SelfContainedVerifiable: true, PoolReadAt: nil}, "github-action-jules", "R3"},
		{"Q3 recurring goes to scheduled OpenCode", Answers{Q3RecurringStable: true, PoolReadAt: &fresh}, "opencode-scheduled", "R5"},
		{"Q4 judgement goes to Muse", Answers{Q4Judgement: true, PoolReadAt: &fresh}, "Muse", "R6"},
		{"Q5 review depth goes to Aster/Codex", Answers{Q5ReviewDepth: true, PoolReadAt: &fresh}, "aster-codex", "R7"},
		{"Q6 live cheap OpenClaw", Answers{Q6OpenClawLiveCheap: true, PoolReadAt: &fresh}, "openclaw", "R8"},
		{"default is a dispatched OpenCode session", Answers{PoolReadAt: &fresh}, "opencode-session", "R4"},
		{"Q1 beats Q4 (first match wins)", Answers{Q1Mechanical: true, Q4Judgement: true}, "github-action", "R1"},
	}
	for _, tc := range cases {
		t.Run(tc.name, func(t *testing.T) {
			if tc.answers.PoolReadAt == nil && p.PoolDrawn(tc.wantOption) {
				tc.answers.PoolReadAt = &fresh
			}
			rec, err := Evaluate(p, tc.answers, now)
			if err != nil {
				t.Fatalf("Evaluate: %v", err)
			}
			if rec.Option != tc.wantOption || rec.Rule != tc.wantRule {
				t.Fatalf("got %s/%s, want %s/%s", rec.Option, rec.Rule, tc.wantOption, tc.wantRule)
			}
			opt, _ := p.Option(tc.wantOption)
			if rec.Supervision != opt.Supervision {
				t.Fatalf("supervision = %q, want the option's %q", rec.Supervision, opt.Supervision)
			}
		})
	}
}

func TestGuardExcludesOpenClaw(t *testing.T) {
	p := testPolicy(t)
	now := time.Date(2026, 11, 17, 3, 0, 0, 0, time.UTC)
	fresh := now.Add(-5 * time.Minute)
	// Guard triggered (needs the laptop) + Q6 shape: R8's option is
	// guard-excluded, so evaluation falls through to the default rule,
	// and the record carries the exclusions.
	rec, err := Evaluate(p, Answers{
		Q2NeedsLocalOrSecrets: true,
		Q6OpenClawLiveCheap:   true,
		PoolReadAt:            &fresh,
	}, now)
	if err != nil {
		t.Fatalf("Evaluate: %v", err)
	}
	if rec.Option != "opencode-session" || rec.Rule != "R4" {
		t.Fatalf("got %s/%s, want opencode-session/R4", rec.Option, rec.Rule)
	}
	if len(rec.GuardR2) != 2 {
		t.Fatalf("guard exclusions = %v, want 2 entries", rec.GuardR2)
	}
	// And R3 is unreachable while the guard is triggered.
	rec, err = Evaluate(p, Answers{
		Q2NeedsLocalOrSecrets:     true,
		Q2SelfContainedVerifiable: true,
		PoolReadAt:                &fresh,
	}, now)
	if err != nil {
		t.Fatalf("Evaluate: %v", err)
	}
	if rec.Rule == "R3" {
		t.Fatalf("R3 fired while the R2 guard was triggered")
	}
}

func TestPoolReadGate(t *testing.T) {
	p := testPolicy(t)
	now := time.Date(2026, 11, 17, 3, 0, 0, 0, time.UTC)

	// Non-pool option: pool_read_at is "n/a" and no read is required.
	rec, err := Evaluate(p, Answers{Q1Mechanical: true}, now)
	if err != nil {
		t.Fatalf("Evaluate R1: %v", err)
	}
	if rec.PoolReadAt != "n/a" {
		t.Fatalf("pool_read_at = %q, want n/a", rec.PoolReadAt)
	}

	// Pool option without a read: rejected (B5).
	if _, err := Evaluate(p, Answers{Q4Judgement: true}, now); err == nil {
		t.Fatal("expected a B5 rejection for a missing pool read")
	}

	// Pool option with a stale read: rejected (B5), unless B4 is recorded.
	stale := now.Add(-2 * time.Hour)
	if _, err := Evaluate(p, Answers{Q4Judgement: true, PoolReadAt: &stale}, now); err == nil {
		t.Fatal("expected a B5 rejection for a stale pool read")
	}
	rec, err = Evaluate(p, Answers{Q4Judgement: true, PoolReadAt: &stale, PoolReadUnobtainable: true}, now)
	if err != nil {
		t.Fatalf("Evaluate with B4 reliance: %v", err)
	}
	if !rec.B4Relied {
		t.Fatal("record does not carry the B4 reliance")
	}
}

func TestException(t *testing.T) {
	p := testPolicy(t)
	now := time.Date(2026, 11, 17, 3, 0, 0, 0, time.UTC)
	fresh := now.Add(-1 * time.Minute)
	rec, err := Evaluate(p, Answers{
		ExceptionID:     "E1-2026-11-17-4986",
		ExceptionOption: "Muse",
		PoolReadAt:      &fresh,
	}, now)
	if err != nil {
		t.Fatalf("Evaluate exception: %v", err)
	}
	if rec.Rule != "exception" || rec.ExceptionID == "" || rec.Option != "Muse" {
		t.Fatalf("bad exception record: %+v", rec)
	}
	if _, err := Evaluate(p, Answers{ExceptionID: "E1-x"}, now); err == nil {
		t.Fatal("expected rejection: exception without a chosen option")
	}
}
