package daemon

import (
	"context"
	"encoding/json"
	"errors"
	"fmt"
	"strings"
	"testing"
	"time"

	"github.com/dezren39/mcpx/internal/config"
	"github.com/dezren39/mcpx/internal/mcpclient"
	"github.com/dezren39/mcpx/internal/pool"
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

// A tools/list_changed must not make the daemon acquire a pool
// instance just to re-check dead rules: for a session-scoped server
// at max 1 that evicts the caller's live session. The re-check is
// noted, held while the only catalog on hand is the stale
// pre-change one, and discharged by the next fresh catalog a real
// read observes.
func TestDeadRuleRecheckDefersUntilFreshCatalogObserved(t *testing.T) {
	var logs []string
	p := pool.New(&config.Resolved{
		Server: &config.Server{Name: "demo", Command: "demo"}, Namespace: "demo"})
	p.SetSchemas([]mcpclient.Tool{{Name: "wipe"}}, nil, nil, "",
		time.Now(), time.Time{}, false)
	r := &Registry{
		views: map[string]*config.Resolved{"demo": argumentRuleView(t)},
		pools: map[string]*pool.Pool{"demo": p},
		logf: func(format string, args ...any) {
			logs = append(logs, fmt.Sprintf(format, args...))
		},
	}

	r.noteDeadRuleRecheck("demo")
	if _, pending := r.deadRulePending.Load("demo"); !pending {
		t.Fatal("list_changed note did not record a pending re-check")
	}

	// The handler invalidated the cache; the tool list it still
	// serves is the previous catalog, where wipe still exists.
	p.Invalidate()
	r.recheckDeadArgumentRules("demo")
	if len(logs) != 0 {
		t.Fatalf("warned from the stale pre-change catalog: %v", logs)
	}
	if _, pending := r.deadRulePending.Load("demo"); !pending {
		t.Fatal("pending re-check discharged without a fresh catalog")
	}

	// A fresh catalog arrives (observed by real work) and wipe is
	// gone from it: the rule is dead, warned once, note discharged.
	p.SetSchemas([]mcpclient.Tool{{Name: "other"}}, nil, nil, "",
		time.Now(), time.Time{}, false)
	r.recheckDeadArgumentRules("demo")
	if len(logs) != 1 || !strings.Contains(logs[0], `"wipe"`) {
		t.Fatalf("want exactly one dead-rule warning for wipe, got %v", logs)
	}
	if _, pending := r.deadRulePending.Load("demo"); pending {
		t.Fatal("pending re-check not discharged after the fresh catalog")
	}
}

// A server with no argument rules takes no note at all: there is
// nothing a changed catalog could orphan.
func TestDeadRuleRecheckNotNotedWithoutRules(t *testing.T) {
	r := &Registry{
		views: map[string]*config.Resolved{"plain": {Namespace: "plain"}},
	}
	r.noteDeadRuleRecheck("plain")
	if _, pending := r.deadRulePending.Load("plain"); pending {
		t.Fatal("pending re-check recorded for a server with no argument rules")
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
