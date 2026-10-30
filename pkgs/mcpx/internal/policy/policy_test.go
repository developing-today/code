package policy

import (
	"encoding/json"
	"testing"
)

func TestClassifyUsesTheSpecDefaults(t *testing.T) {
	for _, c := range []struct {
		name string
		raw  string
		want Classification
	}{
		{"unannotated", ``, Classification{Write: true, Destructive: true}},
		{"empty object", `{}`, Classification{Write: true, Destructive: true}},
		{"readOnly true", `{"readOnlyHint":true}`,
			Classification{ReadOnly: true}},
		{"readOnly true and destructive true", `{"readOnlyHint":true,"destructiveHint":true}`,
			Classification{ReadOnly: true, DestructiveMarked: true}},
		{"destructive false", `{"destructiveHint":false}`,
			Classification{Write: true}},
		{"destructive true", `{"destructiveHint":true}`,
			Classification{Write: true, Destructive: true, DestructiveMarked: true}},
		{"readOnly false", `{"readOnlyHint":false}`,
			Classification{Write: true, Destructive: true}},
	} {
		t.Run(c.name, func(t *testing.T) {
			if got := Classify(json.RawMessage(c.raw)); got != c.want {
				t.Errorf("Classify(%q) = %+v, want %+v", c.raw, got, c.want)
			}
		})
	}
}

func TestCanonicalArgsSortsKeys(t *testing.T) {
	got := CanonicalArgs(map[string]any{"b": 1, "a": 2})
	if got != `{"a":2,"b":1}` {
		t.Errorf("CanonicalArgs = %s, want keys sorted", got)
	}
	if got := CanonicalArgs(nil); got != "null" {
		t.Errorf("CanonicalArgs(nil) = %s, want null", got)
	}
}

func TestParseMode(t *testing.T) {
	for _, s := range []string{"ask", "allow", "deny"} {
		if m, ok := ParseMode(s); !ok || m != Mode(s) || !m.Valid() {
			t.Errorf("ParseMode(%q) = %q, %v", s, m, ok)
		}
	}
	if _, ok := ParseMode("sometimes"); ok {
		t.Error("ParseMode accepted an unknown mode")
	}
}

// The gate's whole table: three modes against a write, a read-only
// call and a destructive write, plus the floor's two directions -- it
// upgrades allow to ask, and it never weakens deny.
func TestEvaluate(t *testing.T) {
	write := Classification{Write: true}
	readOnly := Classification{ReadOnly: true}
	destructive := Classification{Write: true, Destructive: true, DestructiveMarked: true}
	for _, c := range []struct {
		name  string
		mode  Mode
		class Classification
		floor bool
		want  DecisionKind
	}{
		{"deny refuses a write", ModeDeny, write, false, DecideDeny},
		{"deny passes a read-only call", ModeDeny, readOnly, false, DecideAllow},
		{"deny refuses a destructive write", ModeDeny, destructive, false, DecideDeny},
		{"ask asks about a write", ModeAsk, write, false, DecideAsk},
		{"ask passes a read-only call", ModeAsk, readOnly, false, DecideAllow},
		{"ask asks about a destructive write", ModeAsk, destructive, false, DecideAsk},
		{"allow passes a write", ModeAllow, write, false, DecideAllow},
		{"allow passes a read-only call", ModeAllow, readOnly, false, DecideAllow},
		{"allow passes a destructive write without the floor", ModeAllow, destructive, false, DecideAllow},
		{"floor upgrades allow to ask for a destructive call", ModeAllow, destructive, true, DecideAsk},
		{"floor leaves a non-destructive write under allow alone", ModeAllow, write, true, DecideAllow},
		{"floor does not weaken deny", ModeDeny, destructive, true, DecideDeny},
	} {
		t.Run(c.name, func(t *testing.T) {
			d := Evaluator{}.Evaluate(Input{Mode: c.mode, Classification: c.class, ConfirmDestructiveFloor: c.floor})
			if d.Kind != c.want {
				t.Errorf("Evaluate = %v (%s), want %v", d.Kind, d.Reason, c.want)
			}
			if d.Reason == "" {
				t.Error("Evaluate returned no reason")
			}
		})
	}
}
