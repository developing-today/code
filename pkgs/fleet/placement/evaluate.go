package placement

import (
	"fmt"
	"sort"
	"time"
)

// Answers describes one dispatch in the framework's own terms: the
// decision-procedure questions Q1–Q6, the R2 guard trigger, and the
// bookkeeping fields a placement record needs. All fields are inputs a
// dispatcher already knows at handoff time.
type Answers struct {
	// Task identifies the dispatch (e.g. "queue task 14492 / issue #4986").
	Task string `json:"task"`

	// Q1: purely mechanical, no judgement, scriptable end-to-end.
	Q1Mechanical bool `json:"q1_mechanical"`
	// Q2 guard (R2): needs fleet secrets, the laptop, local state, or
	// touches held / user-owned / escalated items.
	Q2NeedsLocalOrSecrets bool `json:"q2_needs_local_or_secrets"`
	// Q2 match (R3), evaluated only when the guard does not trigger:
	// single-repo, fully specified, mechanically verifiable, cheap to reject.
	Q2SelfContainedVerifiable bool `json:"q2_self_contained_verifiable"`
	// Q3: recurring, stable brief, failure visible next round.
	Q3RecurringStable bool `json:"q3_recurring_stable"`
	// Q4: judgement, escalation, or a decision someone acts on.
	Q4Judgement bool `json:"q4_judgement"`
	// Q5: second-opinion / review depth is the point of the task.
	Q5ReviewDepth bool `json:"q5_review_depth"`
	// Q6: the OpenClaw comms path is live for this channel AND failure is
	// cheap and visible.
	Q6OpenClawLiveCheap bool `json:"q6_openclaw_live_cheap"`

	// Exception, when set, bypasses the table: the record's rule is
	// "exception" and ExceptionID identifies the E1 exception record.
	ExceptionID string `json:"exception_id,omitempty"`
	// ExceptionOption is the option actually chosen under the exception.
	ExceptionOption string `json:"exception_option,omitempty"`

	// PoolReadAt is the timestamp of the quota read the placement relies
	// on (B5: at most QuotaReadMaxAgeMinutes old at evaluation), or empty
	// for non-pool options, where the record carries "n/a".
	PoolReadAt *time.Time `json:"pool_read_at,omitempty"`
	// PoolReadUnobtainable records a B4 reliance: no fresh read could be
	// obtained, so dispatch proceeds on the last known state and
	// PoolReadAt carries that last read's timestamp.
	// PoolReadUnobtainable records a B4 reliance: no fresh read could be
	// obtained, so dispatch proceeds on the last known state and
	// PoolReadAt carries that last read's timestamp.
	PoolReadUnobtainable bool `json:"pool_read_unobtainable,omitempty"`
	// PoolID names the cost pool the chosen option draws on (e.g.
	// "chatgpt-plan"), for guard refusals and dispatcher-log lines.
	PoolID string `json:"pool_id,omitempty"`
	// PoolMonthlyPct is the pool's current monthly utilization percent from
	// the quota hub (quota-current.json, ≤60 min old per B5). Nil means the
	// read carried no monthly figure, and the monthly guard cannot apply.
	PoolMonthlyPct *float64 `json:"pool_monthly_pct,omitempty"`
	// PoolPacePph is the pool's trailing burn pace in percentage points per
	// hour toward a wall. Nil means unknown.
	PoolPacePph *float64 `json:"pool_pace_pph,omitempty"`
	// PoolHardStop records a steward hard-stop note for the pool in a
	// machine-readable field the seating path checks (issue #2732 step 2).
	PoolHardStop bool `json:"pool_hard_stop,omitempty"`
}

// Record is the placement record declared by placement-policy.yaml's
// placement_record schema, plus PoC bookkeeping (policy version, the guard
// exclusions that applied, and when the record was evaluated).
type Record struct {
	Option        string    `json:"option"`
	Rule          string    `json:"rule"`
	ExceptionID   string    `json:"exception_id,omitempty"`
	PoolReadAt    string    `json:"pool_read_at"`
	Supervision   string    `json:"supervision"`
	Task          string    `json:"task,omitempty"`
	GuardR2       []string  `json:"guard_r2_excluded_options,omitempty"`
	B4Relied      bool      `json:"b4_unknown_read_relied_on,omitempty"`
	PolicyVersion string    `json:"policy_version"`
	EvaluatedAt   time.Time `json:"evaluated_at"`
}

// Monthly pace-guard thresholds (issue #2732): a seating attempt against
// a pool at or above MonthlyGuardPct, burning faster than PaceGuardPph
// toward a wall, or under a steward hard-stop is refused and rerouted.
const (
	MonthlyGuardPct = 97.0
	PaceGuardPph    = 1.0
)

// Evaluate runs the policy's decision procedure over the answers and
// returns the placement record. Evaluation order is the policy's own:
// Q1 first; the R2 guard at Q2 (it excludes options rather than selecting
// one); then Q3–Q6 in order, first match wins; the policy's default rule
// last. A rule whose option the guard excluded is skipped.
func Evaluate(p *Policy, a Answers, now time.Time) (Record, error) {
	if a.ExceptionID != "" {
		return exceptionRecord(p, a, now)
	}

	var excluded []string
	if a.Q2NeedsLocalOrSecrets {
		excluded = append(excluded, p.Guard.Effect.ExcludedOptions...)
	}
	isExcluded := func(optionID string) bool {
		for _, e := range excluded {
			if e == optionID {
				return true
			}
		}
		return false
	}

	// Candidate rules in the questions' order. R3 is reachable only when
	// the guard did not trigger (Q2's on_no_and shape).
	type candidate struct {
		ruleID  string
		matched bool
	}
	candidates := []candidate{
		{"R1", a.Q1Mechanical},
		{"R3", !a.Q2NeedsLocalOrSecrets && a.Q2SelfContainedVerifiable},
		{"R5", a.Q3RecurringStable},
		{"R6", a.Q4Judgement},
		{"R7", a.Q5ReviewDepth},
		{"R8", a.Q6OpenClawLiveCheap},
	}
	for _, c := range candidates {
		if !c.matched {
			continue
		}
		rule, ok := p.Rule(c.ruleID)
		if !ok {
			return Record{}, fmt.Errorf("policy: missing rule %s", c.ruleID)
		}
		if isExcluded(rule.Option) {
			continue
		}
		return buildRecord(p, a, rule.ID, rule.Option, excluded, now)
	}

	// Default rule (the policy's `default: true` rule, R4 in v1).
	var defaults []Rule
	for _, r := range p.Rules {
		if r.Default {
			defaults = append(defaults, r)
		}
	}
	if len(defaults) != 1 {
		return Record{}, fmt.Errorf("policy: expected exactly one default rule, found %d", len(defaults))
	}
	d := defaults[0]
	if isExcluded(d.Option) {
		return Record{}, fmt.Errorf("default rule %s option %q is guard-excluded; record an exception instead", d.ID, d.Option)
	}
	return buildRecord(p, a, d.ID, d.Option, excluded, now)
}

func exceptionRecord(p *Policy, a Answers, now time.Time) (Record, error) {
	if a.ExceptionOption == "" {
		return Record{}, fmt.Errorf("exception %q names no chosen option", a.ExceptionID)
	}
	return buildRecord(p, a, "exception", a.ExceptionOption, nil, now)
}

func buildRecord(p *Policy, a Answers, ruleID, optionID string, excluded []string, now time.Time) (Record, error) {
	opt, ok := p.Option(optionID)
	if !ok {
		return Record{}, fmt.Errorf("unknown option %q", optionID)
	}
	rec := Record{
		Option:        optionID,
		Rule:          ruleID,
		Supervision:   opt.Supervision,
		Task:          a.Task,
		PolicyVersion: p.Version.String(),
		EvaluatedAt:   now.UTC(),
	}
	if ruleID == "exception" {
		rec.ExceptionID = a.ExceptionID
	}
	if len(excluded) > 0 {
		rec.GuardR2 = append([]string(nil), excluded...)
		sort.Strings(rec.GuardR2)
	}

	// pool_read_at: a timestamp for pool-drawn options (B5 measurement
	// gate), "n/a" otherwise — exactly the policy's field note.
	if !p.PoolDrawn(optionID) {
		rec.PoolReadAt = "n/a"
		return rec, nil
	}
	if a.PoolReadAt == nil {
		return Record{}, fmt.Errorf("option %q draws on a pool but no pool_read_at was supplied (B5 measurement gate)", optionID)
	}
	rec.PoolReadAt = a.PoolReadAt.UTC().Format(time.RFC3339)
	if a.PoolReadUnobtainable {
		rec.B4Relied = true
		return rec, nil
	}
	maxAge := time.Duration(p.QuotaReadMaxAgeMinutes()) * time.Minute
	if age := now.Sub(*a.PoolReadAt); age > maxAge {
		return Record{}, fmt.Errorf("pool_read_at is %s old, beyond the B5 gate of %s; take a fresh read or record a B4 reliance", age.Round(time.Second), maxAge)
	} else if age < -5*time.Minute {
		return Record{}, fmt.Errorf("pool_read_at is %s in the future", (-age).Round(time.Second))
	}
	// Monthly pace guard (#2732 step 1): on a fresh read for a pool-drawn
	// seating, refuse at/above the monthly threshold, above the pace
	// threshold, or under a steward hard-stop. The refusal names the pool,
	// the % read, and the guard source so the dispatcher log can score
	// compliance from data. Unknown state never refuses: a B4 reliance
	// returns above, and absent monthly data seats on the fresh read.
	// A recorded E1 exception bypasses the guard (README: the exception
	// record itself is the required log line) — but the quota-read checks
	// above (B5 freshness, B4) apply to exceptions exactly as to rules.
	if ruleID != "exception" {
		if reason := monthlyGuardRefusal(a); reason != "" {
			return Record{}, fmt.Errorf("monthly-guard refusal: option %q on pool %q refused (%s)", optionID, guardPoolID(p, a, optionID), reason)
		}
	}
	return rec, nil
}

// monthlyGuardRefusal reports why a seating must be refused under issue
// #2732, or "" when it may proceed. Guard sources: "monthly-guard",
// "pace-guard", "hard-stop".
func monthlyGuardRefusal(a Answers) string {
	monthly := -1.0
	if a.PoolMonthlyPct != nil {
		monthly = *a.PoolMonthlyPct
	}
	pace := 0.0
	if a.PoolPacePph != nil {
		pace = *a.PoolPacePph
	}
	if a.PoolHardStop {
		return fmt.Sprintf("steward hard-stop in effect (monthly %.1f%%) [source: hard-stop]", monthly)
	}
	if a.PoolMonthlyPct != nil && monthly >= MonthlyGuardPct {
		return fmt.Sprintf("monthly %.1f%% at/above %.0f%% threshold [source: monthly-guard]", monthly, MonthlyGuardPct)
	}
	if a.PoolPacePph != nil && pace > PaceGuardPph {
		return fmt.Sprintf("trailing pace %.1fpp/h above %.0fpp/h toward a wall (monthly %.1f%%) [source: pace-guard]", pace, PaceGuardPph, monthly)
	}
	return ""
}

// guardPoolID names the pool for a refusal line: the explicit PoolID when
// supplied, else the option's cost-pool label from the policy.
func guardPoolID(p *Policy, a Answers, optionID string) string {
	if a.PoolID != "" {
		return a.PoolID
	}
	if opt, ok := p.Option(optionID); ok {
		return opt.CostPool
	}
	return "unknown-pool"
}
