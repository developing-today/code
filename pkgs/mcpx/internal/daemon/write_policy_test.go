package daemon

import (
	"context"
	"crypto/sha256"
	"encoding/hex"
	"encoding/json"
	"errors"
	"os"
	"path/filepath"
	"strings"
	"testing"
	"time"

	"github.com/developing-today/code/pkgs/mcpx/internal/config"
	"github.com/developing-today/code/pkgs/mcpx/internal/elicit"
	"github.com/developing-today/code/pkgs/mcpx/internal/mcpclient"
	"github.com/developing-today/code/pkgs/mcpx/internal/pool"
	"github.com/developing-today/code/pkgs/mcpx/internal/testsupport"
)

// The write policy gate (#1979), in the destructive_test.go fixture
// style: a pool over a locally declared server, its schemas seeded
// into the cache with SetSchemas so no process starts, and a registry
// whose consumer policy is the one under test. The registry here has
// no broker unless a test installs one, which is itself one of the
// cases: ask with nobody to answer must refuse.

func writePolicyPool(t *testing.T, writePolicy string, annotations string) *pool.Pool {
	t.Helper()
	p := pool.New(&config.Resolved{
		Server:      &config.Server{Name: "demo", Command: "demo"},
		Namespace:   "demo",
		WritePolicy: writePolicy,
	})
	p.SetSchemas(
		[]mcpclient.Tool{{Name: "wipe", Annotations: json.RawMessage(annotations)}},
		nil, nil, "", time.Now(), time.Time{}, false)
	return p
}

func TestUnknownOutcomeBlocksWritesUntilClearedAndAllowsReadOnly(t *testing.T) {
	dir := t.TempDir()
	ledger, err := OpenWriteLedger(dir)
	if err != nil {
		t.Fatal(err)
	}
	r := writePolicyRegistry("allow")
	r.writeLedger = ledger
	p := writePolicyPool(t, "", `{"readOnlyHint":false}`)
	args := map[string]any{"z": 2, "a": "private-argument"}
	callErr := r.recordUnknownOutcome(p, "demo", "wipe", args, "call")
	var recorded ErrUnknownOutcome
	if !errors.As(callErr, &recorded) {
		t.Fatalf("recordUnknownOutcome error = %T %v, want ErrUnknownOutcome", callErr, callErr)
	}
	wantDigest := sha256.Sum256([]byte(`{"a":"private-argument","z":2}`))
	if got := recorded.Record.ArgsSHA256; got != hex.EncodeToString(wantDigest[:]) {
		t.Fatalf("argument digest = %s, want SHA-256 of canonical arguments", got)
	}
	data, err := os.ReadFile(filepath.Join(dir, writeLedgerFilename))
	if err != nil {
		t.Fatal(err)
	}
	if strings.Contains(string(data), "private-argument") {
		t.Fatalf("ledger persisted original arguments: %s", data)
	}
	if _, err := r.applyWritePolicy(context.Background(), p, "demo", "wipe", config.CallContext{}, nil); err == nil {
		t.Fatal("write passed despite the server's unknown outcome")
	} else {
		var blocked ErrUnknownOutcome
		if !errors.As(err, &blocked) {
			t.Fatalf("applyWritePolicy error = %T %v, want ErrUnknownOutcome", err, err)
		}
		if got := failureStatus(err); got != 409 {
			t.Errorf("failureStatus(ErrUnknownOutcome) = %d, want 409", got)
		}
		if !strings.Contains(err.Error(), "mcpx writes clear demo") {
			t.Errorf("refusal omits the clear command: %v", err)
		}
	}

	// A positively declared read-only call remains available on a blocked
	// server, but only after the schema says so.
	p.SetSchemas(
		[]mcpclient.Tool{{Name: "wipe", Annotations: json.RawMessage(`{"readOnlyHint":true}`)}},
		nil, nil, "", time.Now(), time.Time{}, false)
	if _, err := r.applyWritePolicy(context.Background(), p, "demo", "wipe", config.CallContext{}, nil); err != nil {
		t.Fatalf("read-only call was blocked: %v", err)
	}

	cleared, err := ledger.Clear("demo")
	if err != nil || !cleared {
		t.Fatalf("Clear() = %v, %v; want true, nil", cleared, err)
	}
	p.SetSchemas(
		[]mcpclient.Tool{{Name: "wipe", Annotations: json.RawMessage(`{"readOnlyHint":false}`)}},
		nil, nil, "", time.Now(), time.Time{}, false)
	if _, err := r.applyWritePolicy(context.Background(), p, "demo", "wipe", config.CallContext{}, nil); err != nil {
		t.Fatalf("write remained blocked after clear: %v", err)
	}
}

func TestRegistryPreDispatchFailureDoesNotRecordUnknownOutcome(t *testing.T) {
	ledger, err := OpenWriteLedger(t.TempDir())
	if err != nil {
		t.Fatal(err)
	}
	p := writePolicyPool(t, "", `{"readOnlyHint":false}`)
	r := &Registry{
		pools:       map[string]*pool.Pool{"demo": p},
		writeLedger: ledger,
		logf:        func(string, ...any) {},
	}
	ctx, cancel := context.WithCancel(context.Background())
	cancel()
	_, err = r.Call(ctx, "demo", "wipe", config.CallContext{}, map[string]any{"id": 1})
	if err == nil {
		t.Fatal("Call() with a cancelled context unexpectedly succeeded")
	}
	if _, blocked := ledger.Lookup("demo"); blocked {
		t.Fatal("pre-dispatch failure incorrectly recorded an unknown outcome")
	}
}

// (#6493) The post-dispatch ledger classification honors the operator's
// readOnlyTools declaration, not just tool annotations. Oracle: after a
// genuinely post-dispatch failure -- the fake server's "slow" tool is
// dispatched, then the call context times out before any response --
// the ledger must contain no unknown-outcome record for the declared
// read-only tool, and must still contain one for the same tool when it
// is not declared. "slow" carries no annotations, so the annotation
// path alone classifies it as a write in both cases.
func TestCallWithLedgerHonorsDeclaredReadOnlyAfterDispatch(t *testing.T) {
	run := func(t *testing.T, declared bool) (error, bool) {
		t.Helper()
		ledger, err := OpenWriteLedger(t.TempDir())
		if err != nil {
			t.Fatal(err)
		}
		var r *Registry
		if declared {
			r = writePolicyRegistryWithReadOnly("allow", "slow")
		} else {
			r = writePolicyRegistry("allow")
		}
		r.writeLedger = ledger
		// Resolve through the real config path, as the pool tests do:
		// a hand-built Resolved leaves the folded defaults (instance
		// cap, start/call timeouts) at zero and the pool cannot serve.
		cfg := &config.Config{MCPServers: map[string]*config.Server{
			"demo": {Name: "demo", Command: testsupport.FakeMCPBinary(t)},
		}}
		resolved, err := cfg.Resolve("demo")
		if err != nil {
			t.Fatalf("resolve: %v", err)
		}
		scopeKey, _ := resolved.Scope.Key(config.CallContext{})
		p := pool.New(resolved)
		t.Cleanup(p.Close)
		// Warm the pool with a generous context first: cold start and
		// discovery must not eat the timed call's budget, or the failure
		// lands in discovery (pre-dispatch) instead of after dispatch.
		warmCtx, warmCancel := context.WithTimeout(context.Background(), 90*time.Second)
		defer warmCancel()
		if _, err := p.Call(warmCtx, config.ResolvedScope{Key: scopeKey}, "echo", map[string]any{"message": "warm"}); err != nil {
			t.Fatalf("warm-up call: %v", err)
		}
		ctx, cancel := context.WithTimeout(context.Background(), 5*time.Second)
		defer cancel()
		_, callErr := r.callWithLedger(ctx, p, "demo", "slow", map[string]any{"ms": 30000}, config.ResolvedScope{Key: scopeKey}, "call")
		if callErr == nil {
			t.Fatal("slow call unexpectedly succeeded")
		}
		_, blocked := ledger.Lookup("demo")
		return callErr, blocked
	}

	callErr, blocked := run(t, true)
	var unknown ErrUnknownOutcome
	if errors.As(callErr, &unknown) {
		t.Fatalf("declared read-only post-dispatch failure = %v, want the raw call error", callErr)
	}
	if blocked {
		t.Fatal("declared read-only tool recorded an unknown outcome after a post-dispatch failure")
	}

	callErr, blocked = run(t, false)
	if !errors.As(callErr, &unknown) {
		t.Fatalf("undeclared post-dispatch failure = %T %v, want ErrUnknownOutcome", callErr, callErr)
	}
	if !blocked {
		t.Fatal("undeclared write tool recorded no unknown outcome after a post-dispatch failure")
	}
}

// A catalogue that has expired, or been invalidated by tools/list_changed,
// no longer proves a tool is read-only: a tool that has become writable
// would run, lose its response, and be exempted from the ledger, leaving a
// retry unblocked. Only a fresh catalogue may grant the exemption.
func TestToolReadOnlyRejectsStaleCatalogue(t *testing.T) {
	p := writePolicyPool(t, "", `{"readOnlyHint":true}`)
	readOnly := []mcpclient.Tool{{Name: "wipe", Annotations: json.RawMessage(`{"readOnlyHint":true}`)}}

	p.SetSchemas(readOnly, nil, nil, "", time.Now(), time.Time{}, false)
	if !toolReadOnly(p, "wipe") {
		t.Fatal("a fresh read-only annotation was not honoured")
	}

	p.SetSchemas(readOnly, nil, nil, "", time.Now().Add(-time.Hour), time.Now().Add(-time.Minute), false)
	if toolReadOnly(p, "wipe") {
		t.Fatal("an expired catalogue granted the read-only exemption")
	}

	p.SetSchemas(readOnly, nil, nil, "", time.Now(), time.Time{}, false)
	p.Invalidate()
	if toolReadOnly(p, "wipe") {
		t.Fatal("an invalidated catalogue granted the read-only exemption")
	}
}

// Pools are shared between aliases by process identity, and the pool keeps
// the name of whichever alias created it. A block persisted under the
// original name must still block every alias after a restart reorders them.
func TestLedgerBlockSurvivesSharedPoolAliasChange(t *testing.T) {
	ledger, err := OpenWriteLedger(t.TempDir())
	if err != nil {
		t.Fatal(err)
	}
	// The pool's creating name differs from both aliases, which is exactly
	// the state a reordered ResolveAll leaves behind.
	p := writePolicyPool(t, "", `{"readOnlyHint":false}`)
	views := map[string]*config.Resolved{
		"aa-github": {Namespace: "aa-github"},
		"github":    {Namespace: "github"},
	}
	r := &Registry{
		pools:       map[string]*pool.Pool{"aa-github": p, "github": p},
		views:       views,
		order:       []string{"aa-github", "github"},
		writeLedger: ledger,
		logf:        func(string, ...any) {},
	}
	if err := ledger.Record(unknownWriteRecord{
		Server: "github", Tool: "wipe", ArgsSHA256: strings.Repeat("d", 64),
		At: time.Now().UTC(), Surface: "call",
	}); err != nil {
		t.Fatal(err)
	}

	for _, alias := range []string{"github", "aa-github"} {
		if _, blocked := r.ledgerLookupFor(alias, p); !blocked {
			t.Fatalf("the block recorded under the original alias was lost for %q", alias)
		}
		if _, err := r.applyWritePolicy(context.Background(), p, alias, "wipe", config.CallContext{}, nil); err == nil {
			t.Fatalf("%q was allowed to write despite the recorded unknown outcome", alias)
		}
	}

	// Clearing through either alias unblocks both.
	cleared, err := r.clearUnknownOutcome("aa-github")
	if err != nil || !cleared {
		t.Fatalf("clearUnknownOutcome() = %v, %v; want true, nil", cleared, err)
	}
	for _, alias := range []string{"github", "aa-github"} {
		if _, blocked := r.ledgerLookupFor(alias, p); blocked {
			t.Fatalf("%q stayed blocked after the block was cleared", alias)
		}
	}
}

// The record must name the server the caller asked for, not the pool's
// creating alias, so the refusal's `mcpx writes clear <server>` and the
// persisted key agree.
func TestRecordUnknownOutcomeUsesRequestedServerName(t *testing.T) {
	ledger, err := OpenWriteLedger(t.TempDir())
	if err != nil {
		t.Fatal(err)
	}
	p := writePolicyPool(t, "", `{"readOnlyHint":false}`) // pool name is "demo"
	r := &Registry{
		pools:       map[string]*pool.Pool{"github": p},
		writeLedger: ledger,
		logf:        func(string, ...any) {},
	}
	callErr := r.recordUnknownOutcome(p, "github", "wipe", map[string]any{"id": 1}, "call")
	var recorded ErrUnknownOutcome
	if !errors.As(callErr, &recorded) {
		t.Fatalf("recordUnknownOutcome error = %T %v, want ErrUnknownOutcome", callErr, callErr)
	}
	if recorded.Record.Server != "github" {
		t.Fatalf("recorded server = %q, want the requested name %q", recorded.Record.Server, "github")
	}
	if _, blocked := ledger.Lookup("github"); !blocked {
		t.Fatal("the block was not persisted under the requested server name")
	}
	if !strings.Contains(callErr.Error(), "mcpx writes clear github") {
		t.Errorf("refusal omits the clear command for the requested name: %v", callErr)
	}
}

func writePolicyRegistry(mode string) *Registry {
	return &Registry{consumer: ConsumerPolicy{WritePolicy: mode}}
}

// writePolicyRegistryWithView is a registry whose per-name view for
// "demo" carries its own writePolicy override.
func writePolicyRegistryWithView(mode, viewPolicy string) *Registry {
	r := writePolicyRegistry(mode)
	r.views = map[string]*config.Resolved{
		"demo": {Namespace: "demo", WritePolicy: viewPolicy},
	}
	return r
}

// (a) Deny refuses a write tool -- an unannotated tool, which the
// specification's defaults read as a possibly destructive write.
func TestWritePolicyDenyRefusesAWriteTool(t *testing.T) {
	r := writePolicyRegistry("deny")
	p := writePolicyPool(t, "", ``)
	asked, err := r.applyWritePolicy(context.Background(), p, "demo", "wipe", config.CallContext{}, map[string]any{"id": 1})
	if asked {
		t.Error("a denied call must not report it asked")
	}
	var denied ErrWriteDenied
	if !errors.As(err, &denied) {
		t.Fatalf("applyWritePolicy = %v, want ErrWriteDenied", err)
	}
	if got := failureStatus(err); got != 403 {
		t.Errorf("failureStatus(ErrWriteDenied) = %d, want 403", got)
	}
}

// (b) Allow passes a write tool without a broker: no error, no ask,
// and no schema-dependent refusal on the way.
func TestWritePolicyAllowPassesAWriteToolWithoutBroker(t *testing.T) {
	r := writePolicyRegistry("allow")
	p := writePolicyPool(t, "", ``)
	asked, err := r.applyWritePolicy(context.Background(), p, "demo", "wipe", config.CallContext{}, map[string]any{"id": 1})
	if err != nil {
		t.Fatalf("applyWritePolicy = %v, want nil", err)
	}
	if asked {
		t.Error("allow must not ask")
	}
}

// (c) Ask with no broker refuses with ErrNotConfirmed: a caller that
// cannot answer gets a clear refusal naming the escape, not a call
// that proceeds unasked or waits out a deadline.
func TestWritePolicyAskWithNoBrokerRefuses(t *testing.T) {
	r := writePolicyRegistry("ask")
	p := writePolicyPool(t, "", ``)
	asked, err := r.applyWritePolicy(context.Background(), p, "demo", "wipe", config.CallContext{}, map[string]any{"id": 1})
	if asked {
		t.Error("an unanswered ask must not report it asked")
	}
	var notConfirmed ErrNotConfirmed
	if !errors.As(err, &notConfirmed) {
		t.Fatalf("applyWritePolicy = %v, want ErrNotConfirmed", err)
	}
}

// (d) A per-server writePolicy beats the daemon mode, in both
// directions: deny on the server refuses under a daemon-wide allow,
// and allow on the server passes under a daemon-wide deny.
func TestWritePolicyPerServerOverrideBeatsDaemonMode(t *testing.T) {
	r := writePolicyRegistryWithView("allow", "deny")
	p := writePolicyPool(t, "", ``)
	if _, err := r.applyWritePolicy(context.Background(), p, "demo", "wipe", config.CallContext{}, nil); err == nil {
		t.Error("server deny under daemon allow: want a refusal, got nil")
	} else {
		var denied ErrWriteDenied
		if !errors.As(err, &denied) {
			t.Errorf("server deny under daemon allow: got %v, want ErrWriteDenied", err)
		}
	}

	r = writePolicyRegistryWithView("deny", "allow")
	p = writePolicyPool(t, "", ``)
	asked, err := r.applyWritePolicy(context.Background(), p, "demo", "wipe", config.CallContext{}, nil)
	if err != nil || asked {
		t.Errorf("server allow under daemon deny: asked=%v err=%v, want a pass", asked, err)
	}
}

// The override is read from the requested server's view, not the
// pool's creating config (Codex P1 on PR #2829): aliases share one
// pool, so a pool created from an allow config must still honour a
// deny on the alias the call actually requested.
func TestWritePolicyViewOverrideBeatsPoolConfig(t *testing.T) {
	r := writePolicyRegistryWithView("ask", "deny")
	p := writePolicyPool(t, "allow", ``)
	if _, err := r.applyWritePolicy(context.Background(), p, "demo", "wipe", config.CallContext{}, nil); err == nil {
		t.Error("view deny over pool-config allow: want a refusal, got nil")
	} else {
		var denied ErrWriteDenied
		if !errors.As(err, &denied) {
			t.Errorf("view deny over pool-config allow: got %v, want ErrWriteDenied", err)
		}
	}
}

// A read-only tool is not a write, so even deny lets it through.
func TestWritePolicyDenyPassesAReadOnlyTool(t *testing.T) {
	r := writePolicyRegistry("deny")
	p := writePolicyPool(t, "", `{"readOnlyHint":true}`)
	asked, err := r.applyWritePolicy(context.Background(), p, "demo", "wipe", config.CallContext{}, nil)
	if err != nil || asked {
		t.Errorf("read-only under deny: asked=%v err=%v, want a pass", asked, err)
	}
}

// writePolicyRegistryWithReadOnly is a registry whose per-name view
// for "demo" declares the named tools read-only (#6116).
func writePolicyRegistryWithReadOnly(mode string, tools ...string) *Registry {
	r := writePolicyRegistry(mode)
	r.views = map[string]*config.Resolved{
		"demo": {Namespace: "demo", ReadOnlyTools: tools},
	}
	return r
}

// (#6116) A tool the operator declared read-only passes the gate in
// every mode, with no broker and nobody to answer: the declaration is
// the pre-authorization a headless session can actually give. The
// tool here is unannotated, so without the declaration the
// specification's defaults make it a possibly destructive write --
// the fff.find_files case, gated identically to a delete.
func TestWritePolicyDeclaredReadOnlyPassesWithNoAnswerer(t *testing.T) {
	for _, mode := range []string{"ask", "deny"} {
		r := writePolicyRegistryWithReadOnly(mode, "wipe")
		p := writePolicyPool(t, "", ``)
		asked, err := r.applyWritePolicy(context.Background(), p, "demo", "wipe", config.CallContext{}, nil)
		if err != nil || asked {
			t.Errorf("mode %s: declared read-only: asked=%v err=%v, want a pass", mode, asked, err)
		}
	}
}

// The declaration names tools exactly: a tool that is not listed
// keeps the gate, in the same registry, on the same server.
func TestWritePolicyDeclarationDoesNotWidenToUnlistedTools(t *testing.T) {
	r := writePolicyRegistryWithReadOnly("ask", "wipe")
	p := writePolicyPool(t, "", ``)
	_, err := r.applyWritePolicy(context.Background(), p, "demo", "other", config.CallContext{}, nil)
	var notConfirmed ErrNotConfirmed
	if !errors.As(err, &notConfirmed) {
		t.Fatalf("unlisted tool under ask with no broker = %v, want ErrNotConfirmed", err)
	}
}

// The destructive floor is cleared by the declaration too, and only
// by it: with a broker installed and confirmDestructive on, the
// declared tool is not asked about (it returns at once, where an
// unanswered question would wait out the ask timeout and refuse),
// and the undeclared one is refused when nobody answers.
func TestDeclaredReadOnlyClearsTheDestructiveFloor(t *testing.T) {
	broker, err := elicit.Open(filepath.Join(t.TempDir(), "elicit.db"))
	if err != nil {
		t.Fatal(err)
	}
	defer func() { _ = broker.Close() }()
	r := &Registry{
		consumer: ConsumerPolicy{
			WritePolicy:        "allow",
			ConfirmDestructive: true,
			AskTimeout:         2 * time.Second,
		},
		broker: broker,
		views: map[string]*config.Resolved{
			"demo": {Namespace: "demo", ReadOnlyTools: []string{"seek"}},
		},
	}
	p := pool.New(&config.Resolved{
		Server:    &config.Server{Name: "demo", Command: "demo"},
		Namespace: "demo",
	})
	p.SetSchemas(
		[]mcpclient.Tool{{Name: "wipe"}, {Name: "seek"}},
		nil, nil, "", time.Now(), time.Time{}, false)

	if err := r.confirmDestructive(context.Background(), p, "demo", "seek", config.CallContext{}); err != nil {
		t.Fatalf("declared read-only tool was asked about: %v", err)
	}
	err = r.confirmDestructive(context.Background(), p, "demo", "wipe", config.CallContext{})
	var notConfirmed ErrNotConfirmed
	if !errors.As(err, &notConfirmed) {
		t.Fatalf("undeclared tool with nobody to answer = %v, want ErrNotConfirmed", err)
	}
}

// (#6116) A gate refusal is a typed refusal, not a tool failure: HTTP
// 403 and a machine-readable code in the error body, so a headless
// caller can tell "the gate refused this call" apart from "the tool
// ran and failed" (502, no code) without parsing the error text.
func TestGateRefusalsAreTyped(t *testing.T) {
	r := writePolicyRegistry("ask")
	p := writePolicyPool(t, "", ``)
	_, err := r.applyWritePolicy(context.Background(), p, "demo", "wipe", config.CallContext{}, nil)
	if got := failureStatus(err); got != 403 {
		t.Errorf("failureStatus(ErrNotConfirmed) = %d, want 403", got)
	}
	if body := callErrorBody(err); body.Code != "confirmation_required" {
		t.Errorf("callErrorBody(ErrNotConfirmed).Code = %q, want confirmation_required", body.Code)
	}

	r = writePolicyRegistry("deny")
	_, err = r.applyWritePolicy(context.Background(), p, "demo", "wipe", config.CallContext{}, nil)
	if body := callErrorBody(err); body.Code != "write_policy_denied" {
		t.Errorf("callErrorBody(ErrWriteDenied).Code = %q, want write_policy_denied", body.Code)
	}

	plain := errors.New("boom")
	if got := failureStatus(plain); got != 502 {
		t.Errorf("failureStatus(plain error) = %d, want 502", got)
	}
	if body := callErrorBody(plain); body.Code != "" {
		t.Errorf("callErrorBody(plain error).Code = %q, want empty", body.Code)
	}
}
