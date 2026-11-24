package placement

import (
	"regexp"
	"strings"
	"time"
)

// Finding kinds reported by AuditRecords. A record with no findings
// agrees with the policy table; every finding names the way it does not.
const (
	FindingUnknownOption    = "unknown-option"
	FindingUnknownRule      = "unknown-rule"
	FindingRuleOption       = "rule-option-mismatch"
	FindingSupervision      = "supervision-mismatch"
	FindingMissingException = "missing-exception-id"
	FindingPoolReadAt       = "pool-read-at"
	FindingPolicyVersion    = "policy-version-mismatch"
)

// Finding is one disagreement between an emitted placement record and
// the policy table it claims to follow.
type Finding struct {
	Index  int    `json:"index"`
	Task   string `json:"task,omitempty"`
	Kind   string `json:"kind"`
	Detail string `json:"detail"`
}

// ExceptionEntry lists one exception placement in the audited sample:
// exception records are listed whether or not they agree, because the
// weekly audit's job includes showing every bypass of the table.
type ExceptionEntry struct {
	Index       int    `json:"index"`
	Task        string `json:"task,omitempty"`
	ExceptionID string `json:"exception_id"`
	Option      string `json:"option"`
}

// AuditReport is the weekly audit sample's output (issue #2732,
// acceptance criterion 2): the sampled records re-checked against the
// policy table, the exceptions listed, and the counts a steward daily
// pass cites instead of reconstructing seatings after the fact.
type AuditReport struct {
	PolicyVersion   string           `json:"policy_version"`
	RecordsExamined int              `json:"records_examined"`
	RecordsSampled  int              `json:"records_sampled"`
	Agreements      int              `json:"agreements"`
	Findings        []Finding        `json:"findings"`
	Exceptions      []ExceptionEntry `json:"exceptions"`
}

// AuditRecords re-checks emitted placement records against the policy
// table: the named rule must select the recorded option, supervision
// must be the option's own, exception records must carry their
// exception id, pool_read_at must be a timestamp exactly for
// pool-drawn options and "n/a" otherwise, and the record's policy
// version must be the audited policy's. It does not re-run the
// decision procedure — the records do not carry the answers — it
// verifies the records against the table's own declarations.
//
// sample <= 0 audits every record; a positive sample audits that many
// records, chosen evenly across the input so the sample is
// deterministic and reproducible.
func AuditRecords(p *Policy, records []Record, sample int) AuditReport {
	report := AuditReport{
		PolicyVersion:   p.Version.String(),
		RecordsExamined: len(records),
		Findings:        []Finding{},
		Exceptions:      []ExceptionEntry{},
	}
	for _, idx := range sampleIndices(len(records), sample) {
		report.RecordsSampled++
		if auditRecord(p, records[idx], idx, &report) {
			report.Agreements++
		}
	}
	return report
}

// sampleIndices returns the indices to audit: all of them, or count
// evenly spaced indices when count is positive and smaller than n.
func sampleIndices(n, count int) []int {
	if count <= 0 || count >= n {
		idx := make([]int, n)
		for i := range idx {
			idx[i] = i
		}
		return idx
	}
	idx := make([]int, count)
	for i := 0; i < count; i++ {
		idx[i] = i * n / count
	}
	return idx
}

// auditRecord checks one record, appending its findings and exception
// entry to the report. It returns true when the record agrees fully.
func auditRecord(p *Policy, rec Record, idx int, report *AuditReport) bool {
	agree := true
	add := func(kind, detail string) {
		agree = false
		report.Findings = append(report.Findings, Finding{
			Index: idx, Task: rec.Task, Kind: kind, Detail: detail,
		})
	}

	if rec.PolicyVersion != p.Version.String() {
		add(FindingPolicyVersion, "record carries policy version "+rec.PolicyVersion+", audit policy is "+p.Version.String())
	}

	opt, knownOption := p.Option(rec.Option)
	if !knownOption {
		add(FindingUnknownOption, "option "+rec.Option+" is not in the policy")
	} else {
		if rec.Supervision != opt.Supervision {
			add(FindingSupervision, "record supervision differs from option "+rec.Option+"'s own supervision in the policy")
		}
		if p.PoolDrawn(rec.Option) {
			if rec.PoolReadAt == "n/a" {
				add(FindingPoolReadAt, "pool-drawn option "+rec.Option+" carries pool_read_at \"n/a\"")
			} else if _, err := time.Parse(time.RFC3339, rec.PoolReadAt); err != nil {
				add(FindingPoolReadAt, "pool_read_at "+rec.PoolReadAt+" is not an RFC3339 timestamp")
			}
		} else if rec.PoolReadAt != "n/a" {
			add(FindingPoolReadAt, "non-pool option "+rec.Option+" carries pool_read_at "+rec.PoolReadAt+", want \"n/a\"")
		}
	}

	if rec.Rule == "exception" {
		report.Exceptions = append(report.Exceptions, ExceptionEntry{
			Index: idx, Task: rec.Task, ExceptionID: rec.ExceptionID, Option: rec.Option,
		})
		if rec.ExceptionID == "" {
			add(FindingMissingException, "exception record names no exception_id")
		}
	} else if rule, ok := p.Rule(rec.Rule); !ok {
		add(FindingUnknownRule, "rule "+rec.Rule+" is not in the policy")
	} else if knownOption && rule.Option != rec.Option {
		add(FindingRuleOption, "rule "+rec.Rule+" selects option "+rule.Option+" in the table, record says "+rec.Option)
	}
	return agree
}

// RefusalEntry is one monthly-guard refusal line from a dispatcher
// log, reduced to the pool and the guard source that refused it.
type RefusalEntry struct {
	Pool   string `json:"pool"`
	Source string `json:"source"`
}

// RefusalSummary counts monthly-guard refusals in a dispatcher log:
// the total, the per-pool and per-source breakdowns, and the entries
// themselves. Refusals are the guard working, not findings — the
// steward cites these counts as compliance evidence (issue #2732,
// acceptance criterion 2).
type RefusalSummary struct {
	Total    int            `json:"total"`
	ByPool   map[string]int `json:"by_pool"`
	BySource map[string]int `json:"by_source"`
	Entries  []RefusalEntry `json:"entries"`
}

var (
	refusalPoolRe   = regexp.MustCompile(`on pool "([^"]+)" refused`)
	refusalSourceRe = regexp.MustCompile(`\[source: ([a-z-]+)\]`)
)

// CountRefusals scans dispatcher-log text for monthly-guard refusal
// lines — the refusal shape Evaluate produces (and placement-record
// prints to stderr) — and summarizes them. Lines that are not
// refusals are ignored; a refusal-shaped line whose pool or source
// cannot be parsed still counts, under "unknown-pool" / "unknown",
// so the total never undercounts.
func CountRefusals(logText string) RefusalSummary {
	summary := RefusalSummary{
		ByPool:   map[string]int{},
		BySource: map[string]int{},
		Entries:  []RefusalEntry{},
	}
	for _, line := range strings.Split(logText, "\n") {
		if !strings.Contains(line, "monthly-guard refusal:") {
			continue
		}
		entry := RefusalEntry{Pool: "unknown-pool", Source: "unknown"}
		if m := refusalPoolRe.FindStringSubmatch(line); m != nil {
			entry.Pool = m[1]
		}
		if m := refusalSourceRe.FindStringSubmatch(line); m != nil {
			entry.Source = m[1]
		}
		summary.Total++
		summary.ByPool[entry.Pool]++
		summary.BySource[entry.Source]++
		summary.Entries = append(summary.Entries, entry)
	}
	return summary
}
