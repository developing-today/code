package placement

import (
	"strings"
	"testing"
	"time"
)

// TestAuditRecordsAgree is the audit half of issue #2732's second
// slice: records the evaluator itself emits agree with the policy
// table, and the exception among them is listed.
func TestAuditRecordsAgree(t *testing.T) {
	p := testPolicy(t)
	now := time.Date(2026, 11, 23, 3, 0, 0, 0, time.UTC)
	fresh := now.Add(-10 * time.Minute)

	var records []Record
	for _, a := range []Answers{
		{Task: "mechanical", Q1Mechanical: true},
		{Task: "judgement", Q4Judgement: true, PoolReadAt: &fresh},
		{Task: "exception", ExceptionID: "E1-2026-11-23-1", ExceptionOption: "Muse", PoolReadAt: &fresh},
	} {
		rec, err := Evaluate(p, a, now)
		if err != nil {
			t.Fatalf("Evaluate %q: %v", a.Task, err)
		}
		records = append(records, rec)
	}

	report := AuditRecords(p, records, 0)
	if report.RecordsExamined != 3 || report.RecordsSampled != 3 {
		t.Fatalf("examined/sampled = %d/%d, want 3/3", report.RecordsExamined, report.RecordsSampled)
	}
	if len(report.Findings) != 0 {
		t.Fatalf("evaluator-emitted records produced findings: %+v", report.Findings)
	}
	if report.Agreements != 3 {
		t.Fatalf("agreements = %d, want 3", report.Agreements)
	}
	if len(report.Exceptions) != 1 || report.Exceptions[0].ExceptionID != "E1-2026-11-23-1" {
		t.Fatalf("exceptions = %+v, want the one E1 record listed", report.Exceptions)
	}
}

// TestAuditRecordsFindDisagreements tampers with emitted records the
// ways a hand-edited or stale record can drift from the table, and
// requires the audit to name each drift by kind.
func TestAuditRecordsFindDisagreements(t *testing.T) {
	p := testPolicy(t)
	now := time.Date(2026, 11, 23, 3, 0, 0, 0, time.UTC)
	fresh := now.Add(-10 * time.Minute)

	base, err := Evaluate(p, Answers{Task: "judgement", Q4Judgement: true, PoolReadAt: &fresh}, now)
	if err != nil {
		t.Fatalf("Evaluate: %v", err)
	}
	nonPool, err := Evaluate(p, Answers{Task: "mechanical", Q1Mechanical: true}, now)
	if err != nil {
		t.Fatalf("Evaluate: %v", err)
	}

	wrongOption := base
	wrongOption.Option = "openclaw" // R6 selects Muse in the table.
	wrongSupervision := base
	wrongSupervision.Supervision = "none"
	wrongReadAt := base
	wrongReadAt.PoolReadAt = "n/a" // Muse is pool-drawn.
	nonPoolReadAt := nonPool
	nonPoolReadAt.PoolReadAt = fresh.Format(time.RFC3339) // github-action is not.
	wrongVersion := base
	wrongVersion.PolicyVersion = "0.9"
	badException := Record{
		Option: "Muse", Rule: "exception", PoolReadAt: base.PoolReadAt,
		Supervision: base.Supervision, PolicyVersion: p.Version.String(),
	}

	records := []Record{wrongOption, wrongSupervision, wrongReadAt, nonPoolReadAt, wrongVersion, badException}
	report := AuditRecords(p, records, 0)
	if report.Agreements != 0 {
		t.Fatalf("agreements = %d, want 0 for six tampered records", report.Agreements)
	}
	kinds := map[string]int{}
	for _, f := range report.Findings {
		kinds[f.Kind]++
	}
	for _, want := range []string{
		FindingRuleOption, FindingSupervision, FindingPoolReadAt, FindingPolicyVersion, FindingMissingException,
	} {
		if kinds[want] == 0 {
			t.Fatalf("no %s finding in %+v", want, report.Findings)
		}
	}
	if kinds[FindingPoolReadAt] != 2 {
		t.Fatalf("pool-read-at findings = %d, want 2 (pool-drawn n/a + non-pool timestamp)", kinds[FindingPoolReadAt])
	}
	if len(report.Exceptions) != 1 {
		t.Fatalf("exceptions = %+v, want the tampered exception still listed", report.Exceptions)
	}
}

// TestAuditSampling pins the deterministic sample: evenly spaced
// across the input, reproducible between runs.
func TestAuditSampling(t *testing.T) {
	p := testPolicy(t)
	records := make([]Record, 10)
	for i := range records {
		records[i] = Record{
			Option: "github-action", Rule: "R1", PoolReadAt: "n/a",
			PolicyVersion: p.Version.String(),
		}
		if opt, ok := p.Option("github-action"); ok {
			records[i].Supervision = opt.Supervision
		}
	}
	report := AuditRecords(p, records, 3)
	if report.RecordsExamined != 10 || report.RecordsSampled != 3 {
		t.Fatalf("examined/sampled = %d/%d, want 10/3", report.RecordsExamined, report.RecordsSampled)
	}
	if report.Agreements != 3 {
		t.Fatalf("agreements = %d, want 3", report.Agreements)
	}
	again := AuditRecords(p, records, 3)
	if again.RecordsSampled != report.RecordsSampled || len(again.Findings) != len(report.Findings) {
		t.Fatal("sampling is not deterministic between runs")
	}
}

// TestCountRefusals feeds dispatcher-log text in the shape the
// evaluator's refusals take (placement-record prints them to stderr)
// and requires the counts the steward cites: total, per pool, per
// guard source. Non-refusal lines are ignored.
func TestCountRefusals(t *testing.T) {
	logText := strings.Join([]string{
		`placement-record: monthly-guard refusal: option "Muse" on pool "chatgpt-plan" refused (monthly 97.0% at/above 97% threshold [source: monthly-guard])`,
		`some other dispatcher line`,
		`placement-record: monthly-guard refusal: option "Muse" on pool "chatgpt-plan" refused (steward hard-stop in effect (monthly 98.2%) [source: hard-stop])`,
		`placement-record: monthly-guard refusal: option "opencode-session" on pool "meta-pool" refused (trailing pace 1.4pp/h above 1pp/h toward a wall (monthly 90.0%) [source: pace-guard])`,
		``,
	}, "\n")
	summary := CountRefusals(logText)
	if summary.Total != 3 {
		t.Fatalf("total = %d, want 3", summary.Total)
	}
	if summary.ByPool["chatgpt-plan"] != 2 || summary.ByPool["meta-pool"] != 1 {
		t.Fatalf("by_pool = %v", summary.ByPool)
	}
	for _, source := range []string{"monthly-guard", "hard-stop", "pace-guard"} {
		if summary.BySource[source] != 1 {
			t.Fatalf("by_source[%s] = %d, want 1 (all: %v)", source, summary.BySource[source], summary.BySource)
		}
	}
	if len(summary.Entries) != 3 {
		t.Fatalf("entries = %d, want 3", len(summary.Entries))
	}
}
