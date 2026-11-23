// Command placement-record is the #4986 PoC entry point for ONE dispatch
// path — the queue-worker handoff. A dispatcher (or the worker at claim
// time) describes the dispatch with the framework's decision questions in
// a JSON answers file, and the tool prints the placement record as JSON.
//
// Usage:
//
//	placement-record --policy placement-policy.yaml --answers answers.json
//
// The answers file is a placement.Answers JSON object, e.g.:
//
//	{"task":"queue task 14492 / issue #4986","q4_judgement":true,
//	 "pool_read_at":"2026-11-17T02:55:00Z"}
//
// Live quota read (#2732): instead of hand-supplying the monthly guard
// inputs in the answers file, pass --quota-current <quota-current.json>
// --pool <serviceId> and the tool fills pool_monthly_pct, pool_read_at,
// and pool_id from the gatherer's monthly window for that pool (the live
// read the B5 gate measures against). The live read wins over any
// hand-supplied guard inputs in the answers file. A missing, stale, or
// failed read is a non-zero exit, not a silent unguarded record.
//
// Exit status is non-zero when no valid record can be emitted (for
// example a pool-drawn placement without a fresh-enough quota read, per
// burn rule B5) — a dispatch that cannot produce a record does not
// silently proceed unrecorded.
package main

import (
	"encoding/json"
	"flag"
	"fmt"
	"os"
	"time"

	"github.com/developing-today/code/pkgs/fleet/placement"
)

func main() {
	policyPath := flag.String("policy", "", "path to placement-policy.yaml (required)")
	answersPath := flag.String("answers", "", "path to the answers JSON file (required)")
	quotaCurrentPath := flag.String("quota-current", "", "path to the quota gatherer's quota-current.json; with --pool, fills the monthly guard inputs from its live read (#2732)")
	poolID := flag.String("pool", "", "pool serviceId whose monthly window --quota-current reads (required with --quota-current)")
	flag.Parse()
	// Argument contract (#3352): every argument is accounted for before
	// any work; a stray positional is a usage error, not ignored.
	if flag.NArg() > 0 {
		fmt.Fprintf(os.Stderr, "placement-record: unexpected argument %q\n", flag.Arg(0))
		fmt.Fprintln(os.Stderr, "usage: placement-record --policy <placement-policy.yaml> --answers <answers.json> [--quota-current <quota-current.json> --pool <serviceId>]")
		os.Exit(2)
	}
	if *policyPath == "" || *answersPath == "" || (*quotaCurrentPath == "") != (*poolID == "") {
		fmt.Fprintln(os.Stderr, "usage: placement-record --policy <placement-policy.yaml> --answers <answers.json> [--quota-current <quota-current.json> --pool <serviceId>]")
		os.Exit(2)
	}
	pol, err := placement.Load(*policyPath)
	if err != nil {
		fmt.Fprintln(os.Stderr, "placement-record:", err)
		os.Exit(1)
	}
	raw, err := os.ReadFile(*answersPath)
	if err != nil {
		fmt.Fprintln(os.Stderr, "placement-record: read answers:", err)
		os.Exit(1)
	}
	var answers placement.Answers
	if err := json.Unmarshal(raw, &answers); err != nil {
		fmt.Fprintln(os.Stderr, "placement-record: parse answers:", err)
		os.Exit(1)
	}
	now := time.Now()
	if *quotaCurrentPath != "" {
		pct, readAt, err := placement.ReadQuotaCurrent(*quotaCurrentPath, *poolID, now, time.Duration(pol.QuotaReadMaxAgeMinutes())*time.Minute)
		if err != nil {
			fmt.Fprintln(os.Stderr, "placement-record:", err)
			os.Exit(1)
		}
		// The live read is authoritative over hand-supplied guard
		// inputs: the seating decision is made on the measured
		// monthly %, never on a stale answers file.
		answers.PoolMonthlyPct = &pct
		answers.PoolReadAt = &readAt
		answers.PoolID = *poolID
	}
	rec, err := placement.Evaluate(pol, answers, now)
	if err != nil {
		fmt.Fprintln(os.Stderr, "placement-record:", err)
		os.Exit(1)
	}
	out, err := json.MarshalIndent(rec, "", "  ")
	if err != nil {
		fmt.Fprintln(os.Stderr, "placement-record:", err)
		os.Exit(1)
	}
	fmt.Println(string(out))
}
