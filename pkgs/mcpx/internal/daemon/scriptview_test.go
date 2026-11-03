package daemon

import (
	"context"
	"encoding/json"
	"errors"
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
			"wait": {Command: "sleep", Args: []string{"5"}, Description: "waits", Timeout: "100ms"},
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
