// Command placement-audit is the weekly audit sample for the
// placement-record PoC (issue #2732, slice 2): it re-checks emitted
// placement records against the policy table, lists the exceptions,
// and — given a dispatcher log — counts the monthly-guard refusals,
// so a steward daily pass can cite refusal counts from data instead
// of reconstructing seatings after a wall.
//
// Usage:
//
//	placement-audit --policy placement-policy.yaml --records records.json [--log dispatcher.log] [--sample N]
//
// The records file is a JSON array of placement.Record objects, or
// JSONL (one record per line). The report is printed as JSON. Exit
// status is 1 when the audit finds any disagreement between the
// records and the table, 0 when the sampled records all agree
// (refusals are the guard working, not disagreements), and 2 on
// usage errors.
package main

import (
	"bytes"
	"encoding/json"
	"flag"
	"fmt"
	"os"
	"strings"

	"github.com/developing-today/code/pkgs/fleet/placement"
)

func main() {
	policyPath := flag.String("policy", "", "path to placement-policy.yaml (required)")
	recordsPath := flag.String("records", "", "path to the placement records JSON/JSONL file (required)")
	logPath := flag.String("log", "", "path to a dispatcher log to count monthly-guard refusals from (optional)")
	sample := flag.Int("sample", 0, "audit at most N records, evenly spaced (0 = all)")
	flag.Parse()
	// Argument contract (#3352): every argument is accounted for before
	// any work; a stray positional is a usage error, not ignored.
	if flag.NArg() > 0 {
		fmt.Fprintf(os.Stderr, "placement-audit: unexpected argument %q\n", flag.Arg(0))
		fmt.Fprintln(os.Stderr, "usage: placement-audit --policy <placement-policy.yaml> --records <records.json> [--log <dispatcher.log>] [--sample N]")
		os.Exit(2)
	}
	if *policyPath == "" || *recordsPath == "" || *sample < 0 {
		fmt.Fprintln(os.Stderr, "usage: placement-audit --policy <placement-policy.yaml> --records <records.json> [--log <dispatcher.log>] [--sample N]")
		os.Exit(2)
	}
	pol, err := placement.Load(*policyPath)
	if err != nil {
		fmt.Fprintln(os.Stderr, "placement-audit:", err)
		os.Exit(1)
	}
	raw, err := os.ReadFile(*recordsPath)
	if err != nil {
		fmt.Fprintln(os.Stderr, "placement-audit: read records:", err)
		os.Exit(1)
	}
	records, err := parseRecords(raw)
	if err != nil {
		fmt.Fprintln(os.Stderr, "placement-audit: parse records:", err)
		os.Exit(1)
	}
	report := placement.AuditRecords(pol, records, *sample)
	out := struct {
		Audit    placement.AuditReport     `json:"audit"`
		Refusals *placement.RefusalSummary `json:"refusals,omitempty"`
	}{Audit: report}
	if *logPath != "" {
		logRaw, err := os.ReadFile(*logPath)
		if err != nil {
			fmt.Fprintln(os.Stderr, "placement-audit: read log:", err)
			os.Exit(1)
		}
		summary := placement.CountRefusals(string(logRaw))
		out.Refusals = &summary
	}
	enc, err := json.MarshalIndent(out, "", "  ")
	if err != nil {
		fmt.Fprintln(os.Stderr, "placement-audit:", err)
		os.Exit(1)
	}
	fmt.Println(string(enc))
	if len(report.Findings) > 0 {
		os.Exit(1)
	}
}

// parseRecords accepts a JSON array of records or JSONL (one record
// object per non-empty line).
func parseRecords(raw []byte) ([]placement.Record, error) {
	trimmed := bytes.TrimSpace(raw)
	if len(trimmed) == 0 {
		return nil, fmt.Errorf("empty records file")
	}
	if trimmed[0] == '[' {
		var records []placement.Record
		if err := json.Unmarshal(trimmed, &records); err != nil {
			return nil, err
		}
		return records, nil
	}
	var records []placement.Record
	for i, line := range strings.Split(string(trimmed), "\n") {
		line = strings.TrimSpace(line)
		if line == "" {
			continue
		}
		var rec placement.Record
		if err := json.Unmarshal([]byte(line), &rec); err != nil {
			return nil, fmt.Errorf("line %d: %w", i+1, err)
		}
		records = append(records, rec)
	}
	return records, nil
}
