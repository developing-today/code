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
// hand-supplied guard inputs in the answers file, and a successful live
// read clears any pool_read_unobtainable B4 fallback recorded there:
// once an authoritative read exists, the fallback no longer applies and
// the guard judges the fresh read. A missing, stale, or failed read is
// a fresh read is retried once; if it still cannot be obtained, the tool
// emits a B4 record from the latest valid last-known measurement. That
// fallback is explicitly marked in the record and never presented as fresh.
// If no last-known monthly measurement exists, the tool exits non-zero.
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
	if *policyPath == "" || *answersPath == "" {
		fmt.Fprintln(os.Stderr, "usage: placement-record --policy <placement-policy.yaml> --answers <answers.json> [--quota-current <quota-current.json> --pool <serviceId>]")
		os.Exit(2)
	}
	supplied := map[string]bool{}
	flag.Visit(func(f *flag.Flag) { supplied[f.Name] = true })
	liveRead, err := liveReadSelection(supplied["quota-current"], supplied["pool"], *quotaCurrentPath, *poolID)
	if err != nil {
		fmt.Fprintln(os.Stderr, "placement-record:", err)
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
	if liveRead {
		pct, readAt, b4Fallback, err := readQuotaCurrentWithB4Fallback(
			*quotaCurrentPath,
			*poolID,
			now,
			time.Duration(pol.QuotaReadMaxAgeMinutes())*time.Minute,
		)
		if err != nil {
			fmt.Fprintln(os.Stderr, "placement-record:", err)
			os.Exit(1)
		}
		if b4Fallback {
			applyB4Fallback(&answers, pct, readAt, *poolID)
			fmt.Fprintf(os.Stderr, "placement-record: B4 last-known quota relied on for pool %q (monthly %.1f%% measured %s); retry fresh quota before the next dispatch\n", *poolID, pct, readAt.UTC().Format(time.RFC3339))
		} else {
			applyLiveRead(&answers, pct, readAt, *poolID)
		}
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

// readQuotaCurrentWithB4Fallback retries the fresh read once, then returns
// the last valid monthly measurement as B4 reliance. A fresh read still
// drives the monthly guard; B4 is explicitly distinguishable in the emitted
// record and is used only after both fresh reads fail.
func readQuotaCurrentWithB4Fallback(path, pool string, now time.Time, maxAge time.Duration) (float64, time.Time, bool, error) {
	pct, readAt, firstErr := placement.ReadQuotaCurrent(path, pool, now, maxAge)
	if firstErr == nil {
		return pct, readAt, false, nil
	}
	time.Sleep(250 * time.Millisecond)
	retryAt := now.Add(250 * time.Millisecond)
	pct, readAt, retryErr := placement.ReadQuotaCurrent(path, pool, retryAt, maxAge)
	if retryErr == nil {
		return pct, readAt, false, nil
	}
	pct, readAt, fallbackErr := placement.ReadLastKnownQuotaCurrent(path, pool, retryAt)
	if fallbackErr != nil {
		return 0, time.Time{}, false, fmt.Errorf("fresh quota read failed twice (%v; retry: %v); no usable B4 last-known state: %w", firstErr, retryErr, fallbackErr)
	}
	return pct, readAt, true, nil
}

// liveReadSelection validates the --quota-current/--pool pair and reports
// whether a live quota read was requested. The two options must be
// supplied together, and a present-but-valueless option is a usage
// error, never the default (operator argument contract, #3352): an
// explicitly empty --quota-current "" --pool "" — the shape a dispatcher
// produces when its path variables are unset — must not silently
// degrade to the hand-supplied answers path when the caller asked for
// live quota enforcement.
func liveReadSelection(quotaCurrentSupplied, poolSupplied bool, quotaCurrent, pool string) (bool, error) {
	if quotaCurrentSupplied && quotaCurrent == "" {
		return false, fmt.Errorf("--quota-current was supplied with an empty value")
	}
	if poolSupplied && pool == "" {
		return false, fmt.Errorf("--pool was supplied with an empty value")
	}
	if (quotaCurrent == "") != (pool == "") {
		return false, fmt.Errorf("--quota-current and --pool must be supplied together")
	}
	return quotaCurrent != "", nil
}

// applyLiveRead records a successful live quota read in the answers. The
// live read is authoritative over hand-supplied guard inputs: the
// seating decision is made on the measured monthly %, never on a stale
// answers file. It also clears any pool_read_unobtainable B4 fallback
// the answers file recorded — once a fresh read has been obtained, B4
// reliance no longer applies, and leaving the flag set would make
// buildRecord return on the fallback before the monthly guard ever
// judges the fresh read.
func applyLiveRead(answers *placement.Answers, pct float64, readAt time.Time, poolID string) {
	answers.PoolMonthlyPct = &pct
	answers.PoolReadAt = &readAt
	answers.PoolID = poolID
	answers.PoolReadUnobtainable = false
}

func applyB4Fallback(answers *placement.Answers, pct float64, readAt time.Time, poolID string) {
	answers.PoolMonthlyPct = &pct
	answers.PoolReadAt = &readAt
	answers.PoolID = poolID
	answers.PoolReadUnobtainable = true
}
