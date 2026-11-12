package api

import (
	"sort"
	"time"
)

// The decision report (#1972 step 2): the permissive stage already
// records every decision (Authorize), and this is the operator's read
// of that record — the dry-run the issue's Approach C is built on.
// Before anyone enables a check, they can read from their own daemon
// exactly what the check would have decided about: counts by operation
// (with the permission tier a future policy would require) and by
// principal, over the bounded window the record keeps. Nothing here
// refuses anything, and the record never carries a credential: a
// Decision holds who asked (surface + descriptive ID), which operation,
// which tier it declared, and the answer — there is no field a secret
// could be written into.

// DecisionOperationCount is the report's per-operation row: how many
// decisions named this operation, and the permission tier a future
// policy would require for it (the would-require vocabulary).
type DecisionOperationCount struct {
	Op         string     `json:"op"`
	Permission Permission `json:"permission"`
	Count      int        `json:"count"`
	Allowed    int        `json:"allowed"`
}

// DecisionPrincipalCount is the report's per-principal row: how many
// decisions this caller (surface + descriptive ID) made.
type DecisionPrincipalCount struct {
	Surface string `json:"surface"`
	ID      string `json:"id"`
	Count   int    `json:"count"`
}

// DecisionReport aggregates the recorded decisions over the window the
// record currently holds (the most recent maxRecordedDecisions).
type DecisionReport struct {
	// Stage is the authorisation stage the decisions were taken under.
	// In the permissive stage every decision is allowed; enforcement is
	// a later step of #1972 and will say so here when it exists.
	Stage       string                   `json:"stage"`
	Total       int                      `json:"total"`
	Allowed     int                      `json:"allowed"`
	Since       *time.Time               `json:"since,omitempty"`
	Until       *time.Time               `json:"until,omitempty"`
	ByOperation []DecisionOperationCount `json:"by_operation"`
	ByPrincipal []DecisionPrincipalCount `json:"by_principal"`
}

// ReportDecisions aggregates the recorded decisions into counts by
// operation and by principal. Rows are ordered by count descending,
// then by name, so two reads of an unchanged record render identically.
func ReportDecisions() DecisionReport {
	ds := Decisions()
	report := DecisionReport{
		Stage:       "permissive",
		Total:       len(ds),
		ByOperation: []DecisionOperationCount{},
		ByPrincipal: []DecisionPrincipalCount{},
	}
	if len(ds) == 0 {
		return report
	}
	since := ds[0].At
	until := ds[len(ds)-1].At
	report.Since = &since
	report.Until = &until

	type opKey struct {
		op         string
		permission Permission
	}
	ops := map[opKey]*DecisionOperationCount{}
	principals := map[Principal]*DecisionPrincipalCount{}
	for _, d := range ds {
		if d.Allowed {
			report.Allowed++
		}
		ok := opKey{d.Op, d.Permission}
		row, seen := ops[ok]
		if !seen {
			row = &DecisionOperationCount{Op: d.Op, Permission: d.Permission}
			ops[ok] = row
		}
		row.Count++
		if d.Allowed {
			row.Allowed++
		}
		prow, seen := principals[d.Principal]
		if !seen {
			prow = &DecisionPrincipalCount{Surface: d.Principal.Surface, ID: d.Principal.ID}
			principals[d.Principal] = prow
		}
		prow.Count++
	}
	for _, row := range ops {
		report.ByOperation = append(report.ByOperation, *row)
	}
	sort.Slice(report.ByOperation, func(i, j int) bool {
		a, b := report.ByOperation[i], report.ByOperation[j]
		if a.Count != b.Count {
			return a.Count > b.Count
		}
		if a.Op != b.Op {
			return a.Op < b.Op
		}
		return a.Permission < b.Permission
	})
	for _, row := range principals {
		report.ByPrincipal = append(report.ByPrincipal, *row)
	}
	sort.Slice(report.ByPrincipal, func(i, j int) bool {
		a, b := report.ByPrincipal[i], report.ByPrincipal[j]
		if a.Count != b.Count {
			return a.Count > b.Count
		}
		if a.Surface != b.Surface {
			return a.Surface < b.Surface
		}
		return a.ID < b.ID
	})
	return report
}

// ClearDecisions empties the decision record: the operator's reset for
// the dry-run window, so a report read after a clear covers only what
// happened after it. It changes no behaviour — the funnel keeps
// recording, and keeps answering yes.
func ClearDecisions() {
	decisionsMu.Lock()
	decisions = nil
	decisionsMu.Unlock()
}
