package daemon

import (
	"context"
	"encoding/json"
	"errors"
	"io"
	"log"
	"os"
	"path/filepath"
	"strings"
	"testing"
	"time"

	"github.com/developing-today/code/pkgs/mcpx/internal/config"
	"github.com/developing-today/code/pkgs/mcpx/internal/mcpclient"
)

// #1933 slice 2: a script-backed tool view is a declared local program. The
// view is listed under its own name, its pinned arguments are merged over the
// caller's before the program runs, and the program's stdin carries them as
// one JSON object. None of it may reach an upstream.

// echoPinnedView is a script-only server whose one view runs `cat`, so the
// stdout of a successful call is exactly the JSON the program was given.
func echoPinnedView() *config.Extras {
	return &config.Extras{ToolViews: map[string]config.ToolView{
		"echoPinned": {
			Command:     "cat",
			Description: "echo the pinned arguments",
			InputSchema: json.RawMessage(`{"type":"object","properties":{"tailnet":{"type":"string"},"limit":{"type":"integer"}},"required":["tailnet","limit"]}`),
			Pin:         map[string]any{"tailnet": "-"},
			Hide:        []string{"tailnet"},
		},
	}}
}

func scriptRegistry(t *testing.T, servers map[string]*config.Server) *Registry {
	t.Helper()
	dir := t.TempDir()
	r, err := NewRegistry(&config.Config{MCPServers: servers}, Paths{State: dir, Cache: dir}, nil)
	if err != nil {
		t.Fatal(err)
	}
	return r
}

func scriptOnlyRegistry(t *testing.T) *Registry {
	t.Helper()
	return scriptRegistry(t, map[string]*config.Server{
		"scripts": {Mcpx: echoPinnedView()},
	})
}

// callResult is the MCP CallToolResult a script view returns.
type callResult struct {
	Content []struct {
		Type string `json:"type"`
		Text string `json:"text"`
	} `json:"content"`
	IsError bool `json:"isError"`
}

func decodeCallResult(t *testing.T, raw json.RawMessage) callResult {
	t.Helper()
	var res callResult
	if err := json.Unmarshal(raw, &res); err != nil {
		t.Fatalf("result is not a CallToolResult: %v (%s)", err, raw)
	}
	if len(res.Content) == 0 {
		t.Fatalf("result has no content: %s", raw)
	}
	return res
}

func scriptCtx(t *testing.T) context.Context {
	t.Helper()
	ctx, cancel := context.WithTimeout(context.Background(), 20*time.Second)
	t.Cleanup(cancel)
	return ctx
}

func TestScriptViewIsListedWithItsPinHiddenAndRunsWithPinsMerged(t *testing.T) {
	r := scriptOnlyRegistry(t)
	ctx := scriptCtx(t)
	cc := config.CallContext{}

	var listed *ToolInfo
	for _, tool := range r.Tools([]string{"scripts"}) {
		if tool.Tool == "echoPinned" {
			tool := tool
			listed = &tool
		}
	}
	if listed == nil {
		t.Fatal("the script view must be listed in its namespace under its own name")
	}
	if listed.Description != "echo the pinned arguments" {
		t.Errorf("description = %q", listed.Description)
	}
	schema := string(listed.InputSchema)
	if strings.Contains(schema, "tailnet") {
		t.Errorf("the pinned, hidden argument must be absent from the exposed schema: %s", schema)
	}
	if !strings.Contains(schema, `"limit"`) {
		t.Errorf("the caller's own argument must stay in the exposed schema: %s", schema)
	}

	// The caller tries to set the pinned argument; the pin wins, and the
	// program receives the merged arguments on stdin.
	raw, err := r.Call(ctx, "scripts", "echoPinned", cc, map[string]any{"tailnet": "other", "limit": 3})
	if err != nil {
		t.Fatalf("call: %v", err)
	}
	res := decodeCallResult(t, raw)
	if res.IsError {
		t.Fatalf("a clean exit is not an error: %s", raw)
	}
	var got map[string]any
	if err := json.Unmarshal([]byte(res.Content[0].Text), &got); err != nil {
		t.Fatalf("the program's stdout is not the merged arguments: %v (%q)", err, res.Content[0].Text)
	}
	if got["tailnet"] != "-" || got["limit"] != 3.0 {
		t.Errorf("program received %v, want the pin over the caller's value and the caller's limit", got)
	}
}

func TestScriptViewRejectsArgumentsThatBreakItsSchema(t *testing.T) {
	r := scriptOnlyRegistry(t)
	ctx := scriptCtx(t)
	cc := config.CallContext{}
	for name, args := range map[string]map[string]any{
		"wrong type":    {"limit": "three"},
		"missing limit": {},
		"unknown extra": {"limit": 1, "tailnet": "x"}, // accepted: the pin overrides it
	} {
		t.Run(name, func(t *testing.T) {
			_, err := r.Call(ctx, "scripts", "echoPinned", cc, args)
			if name == "unknown extra" {
				if err != nil {
					t.Fatalf("a pinned argument supplied by the caller is overridden, not rejected: %v", err)
				}
				return
			}
			var invalid *mcpclient.InvalidParamsError
			if !errors.As(err, &invalid) {
				t.Fatalf("want InvalidParamsError for %v, got %v", args, err)
			}
		})
	}
}

func TestScriptViewIsReachableThroughEveryCallPath(t *testing.T) {
	r := scriptOnlyRegistry(t)
	ctx := scriptCtx(t)
	cc := config.CallContext{}
	args := map[string]any{"limit": 7}

	paths := map[string]func() (json.RawMessage, error){
		"Call": func() (json.RawMessage, error) { return r.Call(ctx, "scripts", "echoPinned", cc, args) },
		"CallReporting": func() (json.RawMessage, error) {
			return r.CallReporting(ctx, "id-rep", "scripts", "echoPinned", cc, args)
		},
		"CallAsk": func() (json.RawMessage, error) { return r.CallAsk(ctx, "id-ask", "scripts", "echoPinned", cc, args) },
	}
	for name, call := range paths {
		t.Run(name, func(t *testing.T) {
			raw, err := call()
			if err != nil {
				t.Fatalf("%s: %v", name, err)
			}
			var got map[string]any
			if err := json.Unmarshal([]byte(decodeCallResult(t, raw).Content[0].Text), &got); err != nil {
				t.Fatal(err)
			}
			if got["tailnet"] != "-" || got["limit"] != 7.0 {
				t.Errorf("%s: program received %v", name, got)
			}
		})
	}
}

// A script view must never fall through to an upstream tool. The alias below
// has an upstream that cannot start, and a script view named like one of its
// tools: the script answers under that name, and the renamed view of the
// upstream tool still goes upstream, which fails.
func TestScriptViewDoesNotFallThroughToUpstream(t *testing.T) {
	dir := t.TempDir()
	r := scriptRegistry(t, map[string]*config.Server{
		"demo": {Command: filepath.Join(dir, "never-run")},
		"view": {AliasOf: "demo", Mcpx: &config.Extras{ToolViews: map[string]config.ToolView{
			"list_devices": {Command: "cat", Description: "scripted", Pin: map[string]any{"fixed": "yes"}, Hide: []string{"fixed"},
				InputSchema: json.RawMessage(`{"type":"object","properties":{"fixed":{"type":"string"}}}`)},
			"peek": {From: "list_devices"},
		}}},
	})
	ctx := scriptCtx(t)
	cc := config.CallContext{}

	raw, err := r.Call(ctx, "view", "list_devices", cc, map[string]any{})
	if err != nil {
		t.Fatalf("the script view must answer, not go upstream: %v", err)
	}
	if !strings.Contains(decodeCallResult(t, raw).Content[0].Text, `"fixed":"yes"`) {
		t.Errorf("script view did not receive its pin: %s", raw)
	}

	if _, err := r.Call(ctx, "view", "peek", cc, map[string]any{}); err == nil {
		t.Fatal("a renamed view of an upstream tool must still reach the upstream, which cannot start")
	}

	var hidden HiddenTool
	if _, err := r.Call(ctx, "view", "cat", cc, map[string]any{}); !errors.As(err, &hidden) {
		t.Errorf("the command is not a callable name through the view, got %v", err)
	}
}

func TestScriptViewIsGovernedByTheWritePolicy(t *testing.T) {
	r := scriptRegistry(t, map[string]*config.Server{
		"locked": {Mcpx: &config.Extras{WritePolicy: "deny", ToolViews: echoPinnedView().ToolViews}},
	})
	_, err := r.Call(scriptCtx(t), "locked", "echoPinned", config.CallContext{}, map[string]any{"limit": 1})
	var denied ErrWriteDenied
	if !errors.As(err, &denied) {
		t.Fatalf("a script view is an unannotated write: deny must refuse it, got %v", err)
	}
}

func TestScriptViewNonZeroExitIsAResultNotAnError(t *testing.T) {
	r := scriptRegistry(t, map[string]*config.Server{
		"shell": {Mcpx: &config.Extras{ToolViews: map[string]config.ToolView{
			"fail": {Command: "sh", Args: []string{"-c", "echo partial; echo oops >&2; exit 3"}, Description: "fails"},
		}}},
	})
	raw, err := r.Call(scriptCtx(t), "shell", "fail", config.CallContext{}, map[string]any{})
	if err != nil {
		t.Fatalf("a program that ran and exited non-zero is a result: %v", err)
	}
	res := decodeCallResult(t, raw)
	if !res.IsError {
		t.Error("a non-zero exit must set isError")
	}
	if !strings.Contains(res.Content[0].Text, "partial") {
		t.Errorf("stdout must be kept: %s", raw)
	}
	if len(res.Content) < 2 || !strings.Contains(res.Content[1].Text, "exit status 3") || !strings.Contains(res.Content[1].Text, "oops") {
		t.Errorf("exit status and stderr must be reported: %s", raw)
	}
}

func TestScriptViewIsBoundedByItsTimeout(t *testing.T) {
	r := scriptRegistry(t, map[string]*config.Server{
		"slow": {Mcpx: &config.Extras{ToolViews: map[string]config.ToolView{
			"wait": {Command: "sleep", Args: []string{"5"}, Description: "waits", Timeout: "100ms", ReadOnly: true},
		}}},
	})
	start := time.Now()
	_, err := r.Call(scriptCtx(t), "slow", "wait", config.CallContext{}, map[string]any{})
	if err == nil || !strings.Contains(err.Error(), "did not finish within 100ms") {
		t.Fatalf("want a timeout error naming the budget, got %v", err)
	}
	if elapsed := time.Since(start); elapsed > 4*time.Second {
		t.Errorf("the call ran %s past its 100ms budget", elapsed)
	}
}

func TestScriptOnlyServerIsNeverStarted(t *testing.T) {
	r := scriptOnlyRegistry(t)
	errs := r.Warm(scriptCtx(t), true)
	if err, ok := errs["scripts"]; ok {
		t.Errorf("warming a script-only server must not try to start it: %v", err)
	}
	for _, ns := range r.Namespaces(config.Profile{}) {
		if ns.Namespace == "scripts" {
			if ns.Tools != 1 {
				t.Errorf("the namespace should list its one view, got %d", ns.Tools)
			}
			if ns.Error != "" {
				t.Errorf("a script-only server has no start error, got %q", ns.Error)
			}
			return
		}
	}
	t.Fatal("the script-only namespace is missing from the listing")
}

// A config-declared script file is an ordinary tool: its @param header is
// the advertised inputSchema, its declaration carries the behaviour hints,
// and a call round-trips through the daemon's exec service (#1933 slice 2).
// The script is run with the declaration's fixed options, not caller ones.
func TestScriptFileToolListsItsGeneratedSchemaAndRoundTrips(t *testing.T) {
	dir := t.TempDir()
	script := filepath.Join(dir, "greet.sh")
	if err := os.WriteFile(script, []byte("#!/bin/sh\nprintf 'hello %s' @name\n"), 0o755); err != nil {
		t.Fatal(err)
	}
	cfg := &config.Config{
		Path: filepath.Join(dir, "mcpx.json"),
		MCPServers: map[string]*config.Server{
			"ops": {Name: "ops", Mcpx: &config.Extras{ToolViews: map[string]config.ToolView{
				"greet": {File: script, ReadOnly: true},
			}}},
		},
	}
	srv, err := NewServer(Options{Config: cfg, Paths: Paths{State: dir, Cache: dir},
		Version: "test", Logger: log.New(io.Discard, "", 0)})
	if err != nil {
		t.Fatalf("NewServer: %v", err)
	}
	t.Cleanup(srv.reg.Close)

	var listed *ToolInfo
	for _, tool := range srv.reg.Tools([]string{"ops"}) {
		if tool.Tool == "greet" {
			tool := tool
			listed = &tool
		}
	}
	if listed == nil {
		t.Fatal("the config-declared script must appear in tools/list under its namespace")
	}
	if listed.Namespace != "ops" {
		t.Errorf("namespace = %q, want ops", listed.Namespace)
	}
	if !strings.Contains(string(listed.InputSchema), `"name"`) {
		t.Errorf("the @param header must become the inputSchema: %s", listed.InputSchema)
	}
	var hints struct {
		ReadOnly    *bool `json:"readOnlyHint"`
		Destructive *bool `json:"destructiveHint"`
	}
	if err := json.Unmarshal(listed.Annotations, &hints); err != nil {
		t.Fatalf("annotations are not JSON: %v (%s)", err, listed.Annotations)
	}
	if hints.ReadOnly == nil || !*hints.ReadOnly {
		t.Errorf("readOnlyHint must be declared true: %s", listed.Annotations)
	}
	if hints.Destructive == nil || *hints.Destructive {
		t.Errorf("a read-only tool declares destructiveHint false: %s", listed.Annotations)
	}

	raw, err := srv.reg.Call(scriptCtx(t), "ops", "greet", config.CallContext{}, map[string]any{"name": "world"})
	if err != nil {
		t.Fatalf("call: %v", err)
	}
	res := decodeCallResult(t, raw)
	if res.IsError {
		t.Fatalf("a clean exit is not an error: %s", raw)
	}
	if got := strings.TrimSpace(res.Content[0].Text); got != "hello world" {
		t.Errorf("script output = %q, want the rendered @name", got)
	}
}

// A script that does not declare readOnly is a possible destructive write:
// its destructiveHint is true even though nothing upstream annotated it.
func TestScriptFileToolDefaultsToDestructive(t *testing.T) {
	dir := t.TempDir()
	script := filepath.Join(dir, "touch.sh")
	if err := os.WriteFile(script, []byte("#!/bin/sh\nexit 0\n"), 0o755); err != nil {
		t.Fatal(err)
	}
	cfg := &config.Config{
		Path: filepath.Join(dir, "mcpx.json"),
		MCPServers: map[string]*config.Server{
			"ops": {Name: "ops", Mcpx: &config.Extras{ToolViews: map[string]config.ToolView{
				"touch": {File: script},
			}}},
		},
	}
	srv, err := NewServer(Options{Config: cfg, Paths: Paths{State: dir, Cache: dir},
		Version: "test", Logger: log.New(io.Discard, "", 0)})
	if err != nil {
		t.Fatalf("NewServer: %v", err)
	}
	t.Cleanup(srv.reg.Close)
	for _, tool := range srv.reg.Tools([]string{"ops"}) {
		if tool.Tool != "touch" {
			continue
		}
		var hints struct {
			ReadOnly    *bool `json:"readOnlyHint"`
			Destructive *bool `json:"destructiveHint"`
		}
		if err := json.Unmarshal(tool.Annotations, &hints); err != nil {
			t.Fatal(err)
		}
		if hints.ReadOnly == nil || *hints.ReadOnly {
			t.Errorf("readOnlyHint must be declared false: %s", tool.Annotations)
		}
		if hints.Destructive == nil || !*hints.Destructive {
			t.Errorf("an undeclared script declares destructiveHint true: %s", tool.Annotations)
		}
		return
	}
	t.Fatal("the script tool was not listed")
}

// The declaration's hints are what the write policy classifies a script by.
// A read-only script passes a deny policy; an undeclared one is refused,
// because it is a possible destructive write.
func TestScriptFileHintsGovernTheWritePolicy(t *testing.T) {
	dir := t.TempDir()
	script := filepath.Join(dir, "noop.sh")
	if err := os.WriteFile(script, []byte("#!/bin/sh\nexit 0\n"), 0o755); err != nil {
		t.Fatal(err)
	}
	for name, tc := range map[string]struct {
		view config.ToolView
		want error
	}{
		"declared read-only": {view: config.ToolView{File: script, ReadOnly: true}},
		"undeclared":         {view: config.ToolView{File: script}},
	} {
		t.Run(name, func(t *testing.T) {
			tc.view.File = script
			cfg := &config.Config{
				Path: filepath.Join(dir, "mcpx.json"),
				MCPServers: map[string]*config.Server{
					"ops": {Name: "ops", Mcpx: &config.Extras{
						WritePolicy: "deny", ToolViews: map[string]config.ToolView{"noop": tc.view}}},
				},
			}
			srv, err := NewServer(Options{Config: cfg, Paths: Paths{State: dir, Cache: dir},
				Version: "test", Logger: log.New(io.Discard, "", 0)})
			if err != nil {
				t.Fatalf("NewServer: %v", err)
			}
			t.Cleanup(srv.reg.Close)
			_, err = srv.reg.Call(scriptCtx(t), "ops", "noop", config.CallContext{}, map[string]any{})
			if name == "declared read-only" {
				if err != nil {
					t.Fatalf("a declared read-only script must pass deny: %v", err)
				}
				return
			}
			var denied ErrWriteDenied
			if !errors.As(err, &denied) {
				t.Fatalf("an undeclared script must be refused by deny, got %v", err)
			}
		})
	}
}

// Oracle: the pool's resolved CallTimeout (Codex P2 on PR #8317). A
// command-backed view that declares no timeout of its own inherits the
// pool budget its server was configured with -- here 100ms -- not the
// package default, so a 5s sleep is cut at the pool's bound.
func TestScriptViewInheritsPoolCallTimeout(t *testing.T) {
	r := scriptRegistry(t, map[string]*config.Server{
		"slow": {Mcpx: &config.Extras{CallTimeout: "100ms", ToolViews: map[string]config.ToolView{
			"wait": {Command: "sleep", Args: []string{"5"}, Description: "waits", ReadOnly: true},
		}}},
	})
	start := time.Now()
	_, err := r.Call(scriptCtx(t), "slow", "wait", config.CallContext{}, map[string]any{})
	if err == nil || !strings.Contains(err.Error(), "did not finish within 100ms") {
		t.Fatalf("want the inherited pool budget to bound the call, got %v", err)
	}
	if elapsed := time.Since(start); elapsed > 4*time.Second {
		t.Errorf("the call ran %s past its inherited 100ms budget", elapsed)
	}
}

// Oracle: the declared file's language (Codex P2 on PR #8317). A .py
// script with no shebang is Python because its declaration names a .py
// file; routing it to the TypeScript runtime would fail on print().
// The rendered body must round-trip through the Python runtime.
func TestScriptFileToolPythonWithoutShebangRunsAsPython(t *testing.T) {
	dir := t.TempDir()
	script := filepath.Join(dir, "greet.py")
	body := "# @param name:string   who to greet\nprint(\"hello \" + @name)\n"
	if err := os.WriteFile(script, []byte(body), 0o644); err != nil {
		t.Fatal(err)
	}
	cfg := &config.Config{
		Path: filepath.Join(dir, "mcpx.json"),
		MCPServers: map[string]*config.Server{
			"ops": {Name: "ops", Mcpx: &config.Extras{ToolViews: map[string]config.ToolView{
				"greet": {File: script, ReadOnly: true},
			}}},
		},
	}
	srv, err := NewServer(Options{Config: cfg, Paths: Paths{State: dir, Cache: dir},
		Version: "test", Logger: log.New(io.Discard, "", 0)})
	if err != nil {
		t.Fatalf("NewServer: %v", err)
	}
	t.Cleanup(srv.reg.Close)
	raw, err := srv.reg.Call(scriptCtx(t), "ops", "greet", config.CallContext{}, map[string]any{"name": "world"})
	if err != nil {
		t.Fatalf("call: %v", err)
	}
	res := decodeCallResult(t, raw)
	if res.IsError {
		t.Fatalf("a clean Python run is not an error: %s", raw)
	}
	if got := strings.TrimSpace(res.Content[0].Text); got != "hello world" {
		t.Errorf("script output = %q, want the rendered @name from Python", got)
	}
}

// Oracle: the script result contract (Codex P2 on PR #8317). Values a
// file-backed script passes to emit(...) are part of its tool result,
// not only its stdout: an emit-only script returns the emitted value.
func TestScriptFileToolResultIncludesEmits(t *testing.T) {
	dir := t.TempDir()
	script := filepath.Join(dir, "answer.ts")
	body := "// @param name:string   who to greet\nemit({ greeted: @name });\n"
	if err := os.WriteFile(script, []byte(body), 0o644); err != nil {
		t.Fatal(err)
	}
	cfg := &config.Config{
		Path: filepath.Join(dir, "mcpx.json"),
		MCPServers: map[string]*config.Server{
			"ops": {Name: "ops", Mcpx: &config.Extras{ToolViews: map[string]config.ToolView{
				"answer": {File: script, ReadOnly: true},
			}}},
		},
	}
	srv, err := NewServer(Options{Config: cfg, Paths: Paths{State: dir, Cache: dir},
		Version: "test", Logger: log.New(io.Discard, "", 0)})
	if err != nil {
		t.Fatalf("NewServer: %v", err)
	}
	t.Cleanup(srv.reg.Close)
	raw, err := srv.reg.Call(scriptCtx(t), "ops", "answer", config.CallContext{}, map[string]any{"name": "world"})
	if err != nil {
		t.Fatalf("call: %v", err)
	}
	res := decodeCallResult(t, raw)
	if res.IsError {
		t.Fatalf("a clean emit-only run is not an error: %s", raw)
	}
	joined := ""
	for _, c := range res.Content {
		joined += c.Text
	}
	if !strings.Contains(joined, `"greeted"`) || !strings.Contains(joined, `"world"`) {
		t.Errorf("the emitted value must appear in the tool result: %s", raw)
	}
}

// Oracle: the write ledger (Codex P1 on PR #8317). A non-read-only
// script tool whose call times out has an unknown outcome -- the side
// effect may have happened -- so the call records it: the caller sees
// ErrUnknownOutcome, the ledger holds the record, and a retry is
// refused by the preflight instead of running the script again.
func TestScriptViewTimeoutRecordsUnknownOutcome(t *testing.T) {
	r := scriptRegistry(t, map[string]*config.Server{
		"slow": {Mcpx: &config.Extras{ToolViews: map[string]config.ToolView{
			"wait": {Command: "sleep", Args: []string{"5"}, Description: "waits", Timeout: "100ms"},
		}}},
	})
	if r.writeLedger == nil {
		t.Fatal("the test registry must carry a write ledger")
	}
	_, err := r.Call(scriptCtx(t), "slow", "wait", config.CallContext{}, map[string]any{})
	var unknown ErrUnknownOutcome
	if !errors.As(err, &unknown) {
		t.Fatalf("a timed-out write must surface as an unknown outcome, got %v", err)
	}
	if _, blocked := r.writeLedger.Lookup("slow"); !blocked {
		t.Fatal("the ledger must hold the unknown-outcome record for the server")
	}
	_, err = r.Call(scriptCtx(t), "slow", "wait", config.CallContext{}, map[string]any{})
	if !errors.As(err, &unknown) {
		t.Fatalf("a retry must be refused while the outcome is unknown, got %v", err)
	}
}

// Oracle: the same ledger discipline exempts a declared read-only
// script -- its timeout cannot have performed a write, so nothing is
// recorded and the caller sees the plain timeout.
func TestScriptViewReadOnlyTimeoutRecordsNothing(t *testing.T) {
	r := scriptRegistry(t, map[string]*config.Server{
		"slow": {Mcpx: &config.Extras{ToolViews: map[string]config.ToolView{
			"wait": {Command: "sleep", Args: []string{"5"}, Description: "waits", Timeout: "100ms", ReadOnly: true},
		}}},
	})
	_, err := r.Call(scriptCtx(t), "slow", "wait", config.CallContext{}, map[string]any{})
	var unknown ErrUnknownOutcome
	if errors.As(err, &unknown) {
		t.Fatalf("a read-only timeout must not record an unknown outcome, got %v", err)
	}
	if err == nil || !strings.Contains(err.Error(), "did not finish within 100ms") {
		t.Fatalf("want the plain timeout error, got %v", err)
	}
	if _, blocked := r.writeLedger.Lookup("slow"); blocked {
		t.Fatal("a read-only call must not write the ledger")
	}
}

// Oracle: shell-safe argument handling (Codex P1 on PR #8317). A
// schema-valid argument that is a shell command substitution must
// reach a shell-backed tool as data: the rendered body carries it
// as one single-quoted word, so the shell never executes it.
func TestScriptFileToolShellArgumentStaysData(t *testing.T) {
	dir := t.TempDir()
	script := filepath.Join(dir, "greet.sh")
	if err := os.WriteFile(script, []byte("#!/bin/sh\n# @param name:string\nprintf 'hello %s' @name\n"), 0o755); err != nil {
		t.Fatal(err)
	}
	cfg := &config.Config{
		Path: filepath.Join(dir, "mcpx.json"),
		MCPServers: map[string]*config.Server{
			"ops": {Name: "ops", Mcpx: &config.Extras{ToolViews: map[string]config.ToolView{
				"greet": {File: script, ReadOnly: true},
			}}},
		},
	}
	srv, err := NewServer(Options{Config: cfg, Paths: Paths{State: dir, Cache: dir},
		Version: "test", Logger: log.New(io.Discard, "", 0)})
	if err != nil {
		t.Fatalf("NewServer: %v", err)
	}
	t.Cleanup(srv.reg.Close)
	pwned := filepath.Join(dir, "pwned")
	payload := "$(touch " + pwned + ")"
	raw, err := srv.reg.Call(scriptCtx(t), "ops", "greet", config.CallContext{}, map[string]any{"name": payload})
	if err != nil {
		t.Fatalf("call: %v", err)
	}
	res := decodeCallResult(t, raw)
	if res.IsError {
		t.Fatalf("a clean exit is not an error: %s", raw)
	}
	if got := strings.TrimSpace(res.Content[0].Text); got != "hello "+payload {
		t.Errorf("script output = %q, want the argument as literal data", got)
	}
	if _, err := os.Stat(pwned); !os.IsNotExist(err) {
		t.Errorf("the argument was executed by the shell: %s exists", pwned)
	}
}

// Oracle: Python-compatible literals (Codex P2 on PR #8317). A
// boolean argument renders as Python's True, and an omitted
// optional parameter as None: print(@enabled, @note) must not raise
// NameError on the JavaScript spellings.
func TestScriptFileToolPythonBooleanAndOptionalRender(t *testing.T) {
	dir := t.TempDir()
	script := filepath.Join(dir, "check.py")
	body := "# @param enabled:boolean\n# @param note:string?\nprint(@enabled, @note)\n"
	if err := os.WriteFile(script, []byte(body), 0o644); err != nil {
		t.Fatal(err)
	}
	cfg := &config.Config{
		Path: filepath.Join(dir, "mcpx.json"),
		MCPServers: map[string]*config.Server{
			"ops": {Name: "ops", Mcpx: &config.Extras{ToolViews: map[string]config.ToolView{
				"check": {File: script, ReadOnly: true},
			}}},
		},
	}
	srv, err := NewServer(Options{Config: cfg, Paths: Paths{State: dir, Cache: dir},
		Version: "test", Logger: log.New(io.Discard, "", 0)})
	if err != nil {
		t.Fatalf("NewServer: %v", err)
	}
	t.Cleanup(srv.reg.Close)
	raw, err := srv.reg.Call(scriptCtx(t), "ops", "check", config.CallContext{}, map[string]any{"enabled": true})
	if err != nil {
		t.Fatalf("call: %v", err)
	}
	res := decodeCallResult(t, raw)
	if res.IsError {
		t.Fatalf("a clean Python run is not an error: %s", raw)
	}
	if got := strings.TrimSpace(res.Content[0].Text); got != "True None" {
		t.Errorf("script output = %q, want Python literals True None", got)
	}
}

// Oracle: relative module imports (Codex P2 on PR #8317). A declared
// script importing a sibling module must find it: the rendered body
// runs in a directory where the declared script's siblings are
// visible, not in a generated cache directory of strangers.
func TestScriptFileToolRelativeImportResolves(t *testing.T) {
	dir := t.TempDir()
	if err := os.WriteFile(filepath.Join(dir, "helper.py"), []byte("VALUE = \"from-helper\"\n"), 0o644); err != nil {
		t.Fatal(err)
	}
	script := filepath.Join(dir, "worker.py")
	if err := os.WriteFile(script, []byte("import helper\nprint(helper.VALUE)\n"), 0o644); err != nil {
		t.Fatal(err)
	}
	cfg := &config.Config{
		Path: filepath.Join(dir, "mcpx.json"),
		MCPServers: map[string]*config.Server{
			"ops": {Name: "ops", Mcpx: &config.Extras{ToolViews: map[string]config.ToolView{
				"work": {File: script, ReadOnly: true},
			}}},
		},
	}
	srv, err := NewServer(Options{Config: cfg, Paths: Paths{State: dir, Cache: dir},
		Version: "test", Logger: log.New(io.Discard, "", 0)})
	if err != nil {
		t.Fatalf("NewServer: %v", err)
	}
	t.Cleanup(srv.reg.Close)
	raw, err := srv.reg.Call(scriptCtx(t), "ops", "work", config.CallContext{}, map[string]any{})
	if err != nil {
		t.Fatalf("call: %v", err)
	}
	res := decodeCallResult(t, raw)
	if res.IsError {
		t.Fatalf("the sibling import must resolve: %s", raw)
	}
	if got := strings.TrimSpace(res.Content[0].Text); got != "from-helper" {
		t.Errorf("script output = %q, want the sibling module's value", got)
	}
}

// Oracle: the configured environment (Codex P2 on PR #8317). A
// command-backed tool on a server declaring env: (which is also how
// its requiresSecrets are satisfied) runs with those values in its
// environment, not only the daemon's own.
func TestScriptViewCommandSeesConfiguredEnv(t *testing.T) {
	r := scriptRegistry(t, map[string]*config.Server{
		"ops": {Name: "ops",
			Env: map[string]string{"SCRIPT_TOKEN": "from-config"},
			Mcpx: &config.Extras{RequiresSecrets: []string{"SCRIPT_TOKEN"}, ToolViews: map[string]config.ToolView{
				"token": {Command: "sh", Args: []string{"-c", "printf %s \"$SCRIPT_TOKEN\""}, Description: "print the configured token", ReadOnly: true},
			}}},
	})
	raw, err := r.Call(scriptCtx(t), "ops", "token", config.CallContext{}, map[string]any{})
	if err != nil {
		t.Fatalf("call: %v", err)
	}
	res := decodeCallResult(t, raw)
	if got := strings.TrimSpace(res.Content[0].Text); got != "from-config" {
		t.Errorf("program environment = %q, want the server's configured env value", got)
	}
}

// Oracle: preflight before side effects (Codex P1 on PR #8317). A
// declared script that reaches a namespace whose required secret is
// unavailable is refused before it starts: no local side effect of
// the script's first statements may happen.
func TestScriptFileToolRefusesUnavailableSecretBeforeRunning(t *testing.T) {
	dir := t.TempDir()
	sideEffect := filepath.Join(dir, "side-effect.txt")
	script := filepath.Join(dir, "reach.ts")
	body := "import { writeFileSync } from \"node:fs\";\nwriteFileSync(\"" + sideEffect + "\", \"ran\");\nawait tools.waiting.ping();\n"
	if err := os.WriteFile(script, []byte(body), 0o644); err != nil {
		t.Fatal(err)
	}
	cfg := &config.Config{
		Path: filepath.Join(dir, "mcpx.json"),
		MCPServers: map[string]*config.Server{
			"ops": {Name: "ops", Mcpx: &config.Extras{ToolViews: map[string]config.ToolView{
				"reach": {File: script, ReadOnly: true},
			}}},
			"waiting": {Name: "waiting", Command: "cat",
				Mcpx: &config.Extras{RequiresSecrets: []string{"MCPX_MISSING_14190_TOKEN"}}},
		},
	}
	srv, err := NewServer(Options{Config: cfg, Paths: Paths{State: dir, Cache: dir},
		Version: "test", Logger: log.New(io.Discard, "", 0)})
	if err != nil {
		t.Fatalf("NewServer: %v", err)
	}
	t.Cleanup(srv.reg.Close)
	_, err = srv.reg.Call(scriptCtx(t), "ops", "reach", config.CallContext{}, map[string]any{})
	var refusal SecretRequired
	var refusalP *SecretRequired
	if !errors.As(err, &refusal) && !errors.As(err, &refusalP) {
		t.Fatalf("want a SecretRequired refusal before the script runs, got %v", err)
	}
	if _, serr := os.Stat(sideEffect); !os.IsNotExist(serr) {
		t.Errorf("the script ran before the refusal: %s exists", sideEffect)
	}
}
