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
	flag.Parse()
	if *policyPath == "" || *answersPath == "" {
		fmt.Fprintln(os.Stderr, "usage: placement-record --policy <placement-policy.yaml> --answers <answers.json>")
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
	rec, err := placement.Evaluate(pol, answers, time.Now())
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
