package daemon

import (
	"context"
	"encoding/json"
	"errors"
	"path/filepath"
	"strings"
	"testing"
	"time"

	"github.com/dezren39/mcpx/internal/config"
	"github.com/dezren39/mcpx/internal/mcpclient"
)

// #1933 slice 1, the tailscale shape from the issue: an alias view
// exposing exactly listTailnet and listHostnames, the second with
// its tailnet argument pinned and hidden.
func tailscaleView(t *testing.T) *config.Resolved {
	t.Helper()
	c := &config.Config{MCPServers: map[string]*config.Server{
		"tailscale": {Name: "tailscale", Command: "x"},
		"tailscale-peek": {Name: "tailscale-peek", AliasOf: "tailscale", Mcpx: &config.Extras{
			ToolViews: map[string]config.ToolView{
				"listTailnet":   {From: "list_tailnets"},
				"listHostnames": {From: "list_devices", Pin: map[string]any{"tailnet": "-"}, Hide: []string{"tailnet"}, Description: "hostnames only"},
			}}},
	}}
	v, err := c.Resolve("tailscale-peek")
	if err != nil {
		t.Fatal(err)
	}
	return v
}

func TestViewToolsExposesRenamedToolsOnly(t *testing.T) {
	view := tailscaleView(t)
	all := []mcpclient.Tool{
		{Name: "list_tailnets", Description: "tailnets", InputSchema: json.RawMessage(`{"type":"object","properties":{"format":{"type":"string"}}}`)},
		{Name: "list_devices", Description: "devices", InputSchema: json.RawMessage(`{"type":"object","properties":{"tailnet":{"type":"string"},"limit":{"type":"number"}},"required":["tailnet"]}`)},
		{Name: "delete_device", Description: "not in the view", InputSchema: json.RawMessage(`{"type":"object","properties":{}}`)},
	}
	out := visibleTools(view, all)
	if len(out) != 2 || out[0].Name != "listHostnames" || out[1].Name != "listTailnet" {
		t.Fatalf("view should expose exactly the renamed tools, sorted: %+v", out)
	}
	if out[0].Description != "hostnames only" {
		t.Errorf("description override not applied: %q", out[0].Description)
	}
	if strings.Contains(string(out[0].InputSchema), "tailnet") {
		t.Errorf("pinned property must be hidden from the exposed schema: %s", out[0].InputSchema)
	}
	if !strings.Contains(string(out[0].InputSchema), "limit") {
		t.Errorf("unpinned properties stay: %s", out[0].InputSchema)
	}
}

func TestMergePinnedArgsPinsWin(t *testing.T) {
	pin := map[string]any{"tailnet": "-"}
	got, ok := mergePinnedArgs(map[string]any{"tailnet": "other", "limit": 3.0}, pin).(map[string]any)
	if !ok || got["tailnet"] != "-" || got["limit"] != 3.0 {
		t.Errorf("caller must not override a pin: %v", got)
	}
	got, _ = mergePinnedArgs(nil, pin).(map[string]any)
	if got["tailnet"] != "-" {
		t.Errorf("pins apply to an empty call: %v", got)
	}
	got, _ = mergePinnedArgs(json.RawMessage(`{"limit":2}`), pin).(map[string]any)
	if got["tailnet"] != "-" || got["limit"] != 2.0 {
		t.Errorf("pins merge over raw JSON args: %v", got)
	}
	if mergePinnedArgs("raw", nil) != "raw" {
		t.Error("no pins: args pass through")
	}
}

func TestAViewCallReachesTheGuardUnderTheViewName(t *testing.T) {
	dir := t.TempDir()
	cfg := &config.Config{MCPServers: map[string]*config.Server{
		"demo": {Command: filepath.Join(dir, "never-run")},
		"view": {AliasOf: "demo", Mcpx: &config.Extras{ToolViews: map[string]config.ToolView{
			"peek": {From: "get_workflow"},
		}}},
	}}
	r, err := NewRegistry(cfg, Paths{State: dir, Cache: dir}, nil)
	if err != nil {
		t.Fatal(err)
	}
	ctx, cancel := context.WithTimeout(context.Background(), 20*time.Second)
	defer cancel()
	cc := config.CallContext{}
	// The view name passes the guard and fails at the pool (the
	// command does not exist); the target's real name is hidden
	// through the view.
	var hidden HiddenTool
	if err := callErr(r, ctx, "view", "peek", cc); err == nil || errors.As(err, &hidden) {
		t.Errorf("view name should pass the guard, got %v", err)
	}
	if err := callErr(r, ctx, "view", "get_workflow", cc); !errors.As(err, &hidden) {
		t.Errorf("target name through the view should be HiddenTool, got %v", err)
	}
}

func callErr(r *Registry, ctx context.Context, server, tool string, cc config.CallContext) error {
	_, err := r.Call(ctx, server, tool, cc, map[string]any{})
	return err
}

func TestViewToolsRespectsExclude(t *testing.T) {
	view := tailscaleView(t)
	deny, err := config.CompileToolPatterns(append([]string{}, "list_tailnets"))
	if err != nil {
		t.Fatal(err)
	}
	view.ExcludeTools = deny
	all := []mcpclient.Tool{
		{Name: "list_tailnets", Description: "tailnets", InputSchema: json.RawMessage(`{"type":"object","properties":{"format":{"type":"string"}}}`)},
		{Name: "list_devices", Description: "devices", InputSchema: json.RawMessage(`{"type":"object","properties":{"tailnet":{"type":"string"},"limit":{"type":"number"}},"required":["tailnet"]}`)},
	}
	out := visibleTools(view, all)
	if len(out) != 1 || out[0].Name != "listHostnames" {
		t.Fatalf("excluding target list_tailnets should hide listTailnet view: %+v", out)
	}

	view2 := tailscaleView(t)
	deny2, err := config.CompileToolPatterns([]string{"listHostnames"})
	if err != nil {
		t.Fatal(err)
	}
	view2.ExcludeTools = deny2
	out2 := visibleTools(view2, all)
	if len(out2) != 1 || out2[0].Name != "listTailnet" {
		t.Fatalf("excluding view name listHostnames should hide it: %+v", out2)
	}
}

func TestReloadValidatesViewsAgainstCachedSchemas(t *testing.T) {
	dir := t.TempDir()
	cfg := &config.Config{MCPServers: map[string]*config.Server{
		"srv": {Name: "srv", Command: "echo"},
		"v": {Name: "v", AliasOf: "srv", Mcpx: &config.Extras{ToolViews: map[string]config.ToolView{
			"peek": {From: "tool_a"},
		}}},
	}}
	r, err := NewRegistry(cfg, Paths{State: dir, Cache: dir}, nil)
	if err != nil {
		t.Fatal(err)
	}

	// Cache schemas on the pool so validateViews has something to check against.
	r.pools["srv"].SetSchemas([]mcpclient.Tool{
		{Name: "tool_a", InputSchema: json.RawMessage(`{"type":"object","properties":{"arg1":{"type":"string"}}}`)},
	}, nil, nil, "", time.Now(), time.Now().Add(10*time.Minute), false)

	// Valid reload passes
	if _, _, err := r.Reload(cfg); err != nil {
		t.Fatalf("expected valid reload to pass, got %v", err)
	}

	// Reload with unknown target tool fails
	badCfg := &config.Config{MCPServers: map[string]*config.Server{
		"srv": {Name: "srv", Command: "echo"},
		"v": {Name: "v", AliasOf: "srv", Mcpx: &config.Extras{ToolViews: map[string]config.ToolView{
			"peek": {From: "nonexistent"},
		}}},
	}}
	if _, _, err := r.Reload(badCfg); err == nil {
		t.Fatal("expected reload with unknown target to fail")
	}

	// Reload with invalid pin fails
	badPinCfg := &config.Config{MCPServers: map[string]*config.Server{
		"srv": {Name: "srv", Command: "echo"},
		"v": {Name: "v", AliasOf: "srv", Mcpx: &config.Extras{ToolViews: map[string]config.ToolView{
			"peek": {From: "tool_a", Pin: map[string]any{"invalid_arg": 123}},
		}}},
	}}
	if _, _, err := r.Reload(badPinCfg); err == nil {
		t.Fatal("expected reload with invalid pin to fail")
	}

	// Verify registry still works and holds original configuration
	if r.views["v"].ToolViews["peek"].From != "tool_a" {
		t.Errorf("registry state was corrupted after failed reload")
	}
}
