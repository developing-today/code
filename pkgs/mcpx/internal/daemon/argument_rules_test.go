package daemon

import (
	"context"
	"encoding/json"
	"errors"
	"strings"
	"testing"

	"github.com/dezren39/mcpx/internal/config"
)

// Argument rules at the gate (#1986): the view carries a compiled rule
// set, and applyWritePolicy must refuse a violating call before the
// write mode is consulted -- including when no write policy is
// configured at all, because rules are their own opt-in.

func argumentRuleView(t *testing.T) *config.Resolved {
	t.Helper()
	set, err := config.CompileArgumentRules([]config.ArgumentRuleSource{
		{Key: "wipe", Origin: "test config", Schema: json.RawMessage(
			`{"required": ["confirm"], "properties": {"confirm": {"const": "yes"}}}`)},
	})
	if err != nil {
		t.Fatalf("CompileArgumentRules: %v", err)
	}
	return &config.Resolved{Namespace: "demo", ArgumentRules: set}
}

func argumentRuleRegistry(t *testing.T, mode string) *Registry {
	t.Helper()
	r := writePolicyRegistry(mode)
	r.views = map[string]*config.Resolved{"demo": argumentRuleView(t)}
	return r
}

func TestArgumentRuleRefusesViolatingCallWithNoWritePolicy(t *testing.T) {
	r := argumentRuleRegistry(t, "")
	p := writePolicyPool(t, "", ``)
	_, err := r.applyWritePolicy(context.Background(), p, "demo", "wipe",
		config.CallContext{}, map[string]any{"confirm": "no"})
	var denied ErrArgumentRuleDenied
	if !errors.As(err, &denied) {
		t.Fatalf("applyWritePolicy = %v, want ErrArgumentRuleDenied", err)
	}
	if got := failureStatus(err); got != 400 {
		t.Errorf("failureStatus(ErrArgumentRuleDenied) = %d, want 400 (HiddenTool style)", got)
	}
	msg := err.Error()
	for _, want := range []string{"$.confirm", `"no"`, "wipe", "test config"} {
		if !strings.Contains(msg, want) {
			t.Errorf("refusal %q does not name %q", msg, want)
		}
	}
}

func TestArgumentRulePassesConformingCall(t *testing.T) {
	r := argumentRuleRegistry(t, "")
	p := writePolicyPool(t, "", ``)
	asked, err := r.applyWritePolicy(context.Background(), p, "demo", "wipe",
		config.CallContext{}, map[string]any{"confirm": "yes"})
	if err != nil {
		t.Fatalf("conforming call refused: %v", err)
	}
	if asked {
		t.Error("conforming call must not report it asked")
	}
	// A tool with no rules is untouched.
	if _, err := r.applyWritePolicy(context.Background(), p, "demo", "other",
		config.CallContext{}, map[string]any{"anything": true}); err != nil {
		t.Fatalf("unruled tool refused: %v", err)
	}
}

func TestArgumentRuleFiresBeforeWriteMode(t *testing.T) {
	// Mode allow would pass this call; the rule still refuses it,
	// because in #2126's order the rules step precedes the mode step.
	r := argumentRuleRegistry(t, "allow")
	p := writePolicyPool(t, "", ``)
	_, err := r.applyWritePolicy(context.Background(), p, "demo", "wipe",
		config.CallContext{}, map[string]any{})
	var denied ErrArgumentRuleDenied
	if !errors.As(err, &denied) {
		t.Fatalf("applyWritePolicy under allow = %v, want ErrArgumentRuleDenied", err)
	}
}
