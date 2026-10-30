// Package policy is the mcpx write policy gate (#1979, design #2126):
// whether a tool call that may change something outside mcpx is allowed,
// asked about, or refused, decided before the call is made.
//
// The package is deliberately small and pure. It classifies a call from
// the tool's annotations, renders its arguments canonically for display,
// and evaluates a mode against that classification. It knows nothing
// about pools, brokers or settings: the daemon fetches what it needs,
// hands plain values in, and acts on the decision that comes back. That
// is what makes the gate testable at unit speed, and what will let the
// later phases -- an argument-rule hook and a ledger hook, both stubbed
// here -- slot in without moving the decision itself.
package policy

import (
	"encoding/json"
	"fmt"
)

// Mode is one write policy: what to do with a call that may write.
type Mode string

const (
	// ModeAsk asks for confirmation before a write call, showing the
	// exact tool and its canonical arguments.
	ModeAsk Mode = "ask"
	// ModeAllow lets write calls proceed with no confirmation from the
	// policy. The destructive floor may still ask; see Evaluate.
	ModeAllow Mode = "allow"
	// ModeDeny refuses write calls outright.
	ModeDeny Mode = "deny"
)

// Valid reports whether m is one of the three modes.
func (m Mode) Valid() bool {
	switch m {
	case ModeAsk, ModeAllow, ModeDeny:
		return true
	}
	return false
}

// ParseMode parses s as a Mode, reporting whether it was one.
func ParseMode(s string) (Mode, bool) {
	m := Mode(s)
	if m.Valid() {
		return m, true
	}
	return "", false
}

// Classification is what a tool's annotations say about one call to it,
// read with the specification's defaults, exactly as the daemon's
// destructiveHint has always read them: a tool not annotated
// readOnlyHint true may write, and a write whose destructiveHint is
// absent or true may destroy.
type Classification struct {
	// ReadOnly is set when readOnlyHint is true.
	ReadOnly bool
	// Write is set when the call may change something: readOnlyHint is
	// not true.
	Write bool
	// Destructive is set when a write may be destructive:
	// destructiveHint absent or true.
	Destructive bool
	// DestructiveMarked is set when destructiveHint is present and
	// true, regardless of readOnlyHint. A tool that claims to be
	// read-only while also marking itself destructive is contradictory,
	// and the mark is still worth gating on even though Destructive --
	// which follows the specification's reading -- is not set for it.
	DestructiveMarked bool
}

// Classify reads tool annotations (the raw annotations object, or nil)
// into a Classification. Unreadable annotations say nothing, so the
// specification's defaults stand: the call is a possibly destructive
// write, as an unannotated tool is.
func Classify(annotations json.RawMessage) Classification {
	var doc struct {
		ReadOnly    *bool `json:"readOnlyHint"`
		Destructive *bool `json:"destructiveHint"`
	}
	if len(annotations) > 0 {
		// An unmarshal error does NOT leave doc zero: Unmarshal
		// populates the fields it could decode before failing, so a
		// malformed object like {"readOnlyHint":true,
		// "destructiveHint":"invalid"} would otherwise be read as
		// read-only from the partial decode. Discard the partial
		// result wholesale: unreadable annotations say nothing, and
		// the defaults below (a possibly destructive write) stand.
		if err := json.Unmarshal(annotations, &doc); err != nil {
			doc.ReadOnly = nil
			doc.Destructive = nil
		}
	}
	c := Classification{}
	if doc.ReadOnly != nil && *doc.ReadOnly {
		c.ReadOnly = true
	}
	c.Write = !c.ReadOnly
	c.Destructive = c.Write && (doc.Destructive == nil || *doc.Destructive)
	c.DestructiveMarked = doc.Destructive != nil && *doc.Destructive
	return c
}

// CanonicalArgs renders a call's arguments in one canonical form for
// display and comparison: JSON, whose marshalling sorts map keys, so
// the same arguments always render the same way and a confirmation
// shows exactly the call that will be made. Arguments that will not
// marshal fall back to their Go rendering rather than failing the
// gate over a display problem. Nil renders as "null".
func CanonicalArgs(args any) string {
	b, err := json.Marshal(args)
	if err != nil {
		return fmt.Sprintf("%v", args)
	}
	return string(b)
}

// DecisionKind is the kind of a Decision.
type DecisionKind int

const (
	// DecideAllow lets the call proceed without asking.
	DecideAllow DecisionKind = iota
	// DecideAsk asks for confirmation first.
	DecideAsk
	// DecideDeny refuses the call.
	DecideDeny
)

// Decision is the outcome of evaluating a policy: what to do, and why,
// in words fit for an error or a question.
type Decision struct {
	Kind   DecisionKind
	Reason string
}

// Input is everything Evaluate decides from.
type Input struct {
	Mode           Mode
	Classification Classification
	// ConfirmDestructiveFloor carries elicit.confirmDestructive into
	// the evaluation: a destructive call asks even under ModeAllow.
	// The floor only ever upgrades allow to ask; it never weakens a
	// deny.
	ConfirmDestructiveFloor bool
}

// Evaluator evaluates a write policy. The two hooks are the places
// later phases plug into, in a fixed order before the mode itself is
// consulted: LedgerCheck (Phase 3) may decide a call from the ledger
// of what has already happened, and ArgumentRules (Phase 2) may decide
// it from its arguments. Both are nil in Phase 1, which is the point:
// the order is settled now so neither phase rearranges the gate when
// it lands. A hook that returns ok false has no opinion and evaluation
// continues.
type Evaluator struct {
	LedgerCheck   func() (Decision, bool)
	ArgumentRules func() (Decision, bool)
}

// Evaluate decides a call, in a fixed order: ledger hook, argument
// rules hook, the write mode, then the destructive floor.
func (e Evaluator) Evaluate(in Input) Decision {
	if e.LedgerCheck != nil {
		if d, ok := e.LedgerCheck(); ok {
			return d
		}
	}
	if e.ArgumentRules != nil {
		if d, ok := e.ArgumentRules(); ok {
			return d
		}
	}
	c := in.Classification
	switch in.Mode {
	case ModeDeny:
		if c.Write || c.DestructiveMarked {
			return Decision{Kind: DecideDeny,
				Reason: "write policy deny: the call may write and policy.write is deny"}
		}
	case ModeAsk:
		if c.Write || c.DestructiveMarked {
			return Decision{Kind: DecideAsk,
				Reason: "write policy ask: the call may write and policy.write is ask"}
		}
	case ModeAllow:
		// No confirmation from the mode; the floor below may still ask.
	}
	if in.ConfirmDestructiveFloor && c.Destructive {
		// The floor reaches only an allow that survived the mode: a
		// deny above has already refused a destructive write, and an
		// ask has already asked once. It upgrades, never weakens.
		return Decision{Kind: DecideAsk,
			Reason: "destructive floor: the call may be destructive and elicit.confirmDestructive is on"}
	}
	return Decision{Kind: DecideAllow, Reason: "the call is not a gated write"}
}
