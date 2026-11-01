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
	"github.com/dezren39/mcpx/internal/schemaval"
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

// Codex P2 on the #1986 merge (46ac651db): hiding a property must not
// rewrite unrelated numbers in the schema -- a float64 round-trip
// turned the enum bound 9007199254740993 into 9007199254740992.
func TestHideSchemaPropsPreservesNumberPrecision(t *testing.T) {
	schema := json.RawMessage(`{"type":"object","properties":{"secret":{"type":"string"},"limit":{"type":"integer","enum":[9007199254740993]}},"required":["secret","limit"]}`)
	out := hideSchemaProps(schema, []string{"secret"})
	if strings.Contains(string(out), "secret") {
		t.Errorf("hidden property must be removed: %s", out)
	}
	if !strings.Contains(string(out), "9007199254740993") {
		t.Errorf("large integer must survive the hide round-trip exactly: %s", out)
	}
	if strings.Contains(string(out), "9007199254740992") {
		t.Errorf("large integer was rewritten by float64 rounding: %s", out)
	}
}

// Codex P2 on the #1986 merge (46ac651db): argument rules are enforced
// under the view target's real name (resolveAndGuard maps the alias to
// From before applyWritePolicy), so the schema advertised for the
// alias must be generated under that same identity or the rule is
// enforced but never advertised.
func TestAdvertisedViewSchemaUsesRuleTargetIdentity(t *testing.T) {
	view := tailscaleView(t)
	set, err := config.CompileArgumentRules([]config.ArgumentRuleSource{
		{Key: "list_devices", Origin: "test config", Schema: json.RawMessage(`{"properties":{"limit":{"maximum":10}}}`)},
	})
	if err != nil {
		t.Fatalf("CompileArgumentRules: %v", err)
	}
	view.ArgumentRules = set
	if got := ruleToolName(view, "listHostnames"); got != "list_devices" {
		t.Fatalf("alias must resolve to its rule identity (the From target), got %q", got)
	}
	if got := ruleToolName(view, "listTailnet"); got != "list_tailnets" {
		t.Fatalf("alias must resolve to its rule identity (the From target), got %q", got)
	}
	all := []mcpclient.Tool{
		{Name: "list_devices", InputSchema: json.RawMessage(`{"type":"object","properties":{"tailnet":{"type":"string"},"limit":{"type":"number"}}}`)},
		{Name: "list_tailnets", InputSchema: json.RawMessage(`{"type":"object","properties":{}}`)},
	}
	for _, tool := range visibleTools(view, all) {
		advertised := view.ArgumentRules.EffectiveSchema(ruleToolName(view, tool.Name), tool.InputSchema)
		if tool.Name == "listHostnames" && !strings.Contains(string(advertised), "maximum") {
			t.Errorf("rule keyed by the real name must be advertised on the alias: %s", advertised)
		}
		if tool.Name == "listTailnet" && strings.Contains(string(advertised), "maximum") {
			t.Errorf("rule for list_devices must not leak onto listTailnet: %s", advertised)
		}
	}
}

// The advertised schema for a view tool must not demand the pinned
// property a rule requires: the pin satisfies it, the caller cannot
// set it, and the upstream additionalProperties:false leaves them no
// legal call at all (Codex P2 on PR #2976, both catalog paths go
// through effectiveViewSchema).
func TestAdvertisedViewSchemaDropsPinnedRuleRequirements(t *testing.T) {
	view := tailscaleView(t)
	set, err := config.CompileArgumentRules([]config.ArgumentRuleSource{
		{Key: "list_devices", Origin: "test config", Schema: json.RawMessage(`{"required":["tailnet"]}`)},
	})
	if err != nil {
		t.Fatalf("CompileArgumentRules: %v", err)
	}
	view.ArgumentRules = set
	all := []mcpclient.Tool{
		{Name: "list_devices", InputSchema: json.RawMessage(`{"type":"object","additionalProperties":false,"properties":{"tailnet":{"type":"string"},"limit":{"type":"number"}}}`)},
		{Name: "list_tailnets", InputSchema: json.RawMessage(`{"type":"object","properties":{}}`)},
	}
	for _, tool := range visibleTools(view, all) {
		advertised := string(effectiveViewSchema(view, tool.Name, tool.InputSchema))
		if tool.Name == "listHostnames" && strings.Contains(advertised, "tailnet") {
			t.Errorf("advertised view schema must not mention the pinned+hidden property: %s", advertised)
		}
		if tool.Name == "listHostnames" && strings.Contains(advertised, `"required"`) {
			t.Errorf("advertised view schema must not carry the satisfied requirement: %s", advertised)
		}
	}
}

// Codex P2 on PR #2976: hiding a pinned property must also project
// it out of same-instance allOf constraints. Left in place, an
// allOf member requiring the hidden property -- whose declaration
// the hide deleted -- makes the advertised schema unsatisfiable
// against additionalProperties:false, even though calls succeed
// because pins merge before forwarding.
// Codex P2 on PR #2976 (third round): a rule whose local $ref carries
// sibling constraints is a conjunction, not a merge. The sibling
// "required":["tailnet"] must not overwrite the referenced
// "required":["limit"]: tailnet is pinned (projected away), limit is
// not, and the advertised schema must still demand it -- the merged
// projection accepted {} while enforcement of the original rule
// rejects it.
func TestAdvertisedViewSchemaConjoinsRefSiblingRequirements(t *testing.T) {
	view := tailscaleView(t)
	set, err := config.CompileArgumentRules([]config.ArgumentRuleSource{
		{Key: "list_devices", Origin: "test config", Schema: json.RawMessage(`{"$defs":{"need":{"required":["limit"]}},"$ref":"#/$defs/need","required":["tailnet"]}`)},
	})
	if err != nil {
		t.Fatalf("CompileArgumentRules: %v", err)
	}
	view.ArgumentRules = set
	all := []mcpclient.Tool{
		{Name: "list_devices", InputSchema: json.RawMessage(`{"type":"object","additionalProperties":false,"properties":{"tailnet":{"type":"string"},"limit":{"type":"number"}}}`)},
	}
	for _, tool := range visibleTools(view, all) {
		advertised := string(effectiveViewSchema(view, tool.Name, tool.InputSchema))
		if tool.Name != "listHostnames" {
			continue
		}
		if !strings.Contains(advertised, `"required":["limit"]`) {
			t.Errorf("referenced requirement must survive as a conjunct: %s", advertised)
		}
		if strings.Contains(advertised, "tailnet") {
			t.Errorf("pinned+hidden property must not be advertised: %s", advertised)
		}
	}
}

// Codex P2 on PR #2976: a requirement on a hidden property can also
// reach the advertised schema through an upstream local $ref: the
// root $ref to a $defs entry requiring the hidden property survives
// the hide, while the property declaration is deleted -- against
// additionalProperties:false the advertised schema is unsatisfiable
// even though calls succeed because pins merge before forwarding.
func TestHideSchemaPropsProjectsRefRequirements(t *testing.T) {
	schema := json.RawMessage(`{"type":"object","additionalProperties":false,"properties":{"tailnet":{"type":"string"},"limit":{"type":"integer"}},"$defs":{"need":{"required":["tailnet","limit"]}},"$ref":"#/$defs/need"}`)
	out := hideSchemaProps(schema, []string{"tailnet"})
	compiled, err := schemaval.Compile(out)
	if err != nil {
		t.Fatal(err)
	}
	if err := compiled.ValidateJSON(json.RawMessage(`{"limit":1}`)); err != nil {
		t.Fatalf("projected ref rejected visible input: %v", err)
	}
	if err := compiled.ValidateJSON(json.RawMessage(`{}`)); err == nil {
		t.Fatal("unhidden requirement lost")
	}
}

func TestHideSchemaPropsSharedRefPreservesNestedConstraints(t *testing.T) {
	for _, applicator := range []string{`"$ref":"#/$defs/need"`, `"allOf":[{"$ref":"#/$defs/need"}]`} {
		t.Run(applicator, func(t *testing.T) {
			schema := json.RawMessage(`{"$schema":"https://json-schema.org/draft/2020-12/schema",` + applicator + `,"$defs":{"need":{"type":"object","properties":{"secret":{"type":"string"},"child":{"$ref":"#/$defs/need"}},"required":["secret"]}},"properties":{"nested":{"$ref":"#/$defs/need"}}}`)
			out := hideSchemaProps(schema, []string{"secret"})
			compiled, err := schemaval.Compile(out)
			if err != nil {
				t.Fatalf("compile %s: %v", out, err)
			}
			for _, tc := range []struct {
				input string
				valid bool
			}{
				{`{}`, true},
				{`{"nested":{"secret":"x"}}`, true},
				{`{"nested":{}}`, false},
				{`{"nested":{"secret":"x","child":{}}}`, false},
				{`{"nested":{"secret":"x","child":{"secret":"y"}}}`, true},
			} {
				err := compiled.ValidateJSON(json.RawMessage(tc.input))
				if (err == nil) != tc.valid {
					t.Errorf("%s: valid=%v, err=%v; schema=%s", tc.input, tc.valid, err, out)
				}
			}
		})
	}
}

func TestHideSchemaPropsProjectsAllOfRequirements(t *testing.T) {
	schema := json.RawMessage(`{"type":"object","additionalProperties":false,"properties":{"tailnet":{"type":"string"},"limit":{"type":"integer"}},"required":["limit"],"allOf":[{"required":["tailnet","limit"]}]}`)
	out := hideSchemaProps(schema, []string{"tailnet"})
	if strings.Contains(string(out), "tailnet") {
		t.Errorf("hidden property must be projected out of allOf constraints too: %s", out)
	}
	if !strings.Contains(string(out), `"required":["limit"]`) {
		t.Errorf("unhidden requirements must survive in root and allOf member: %s", out)
	}
}

// Codex P2 on PR #2976: an expired cached catalogue is stale data
// and must not veto a reload -- a view using a tool or argument the
// upstream added after the cache was fetched would fail validation
// until an operator refreshed the server by hand. Validation
// defers until the catalogue is refreshed.
func TestReloadDefersViewValidationOnExpiredCatalog(t *testing.T) {
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
	// Cache schemas whose freshness deadline has already passed.
	r.pools["srv"].SetSchemas([]mcpclient.Tool{
		{Name: "tool_a", InputSchema: json.RawMessage(`{"type":"object","properties":{"arg1":{"type":"string"}}}`)},
	}, nil, nil, "", time.Now().Add(-20*time.Minute), time.Now().Add(-10*time.Minute), false)
	if !r.pools["srv"].SchemaExpired() {
		t.Fatal("seeded catalog must read as expired for this test to mean anything")
	}
	// A view the stale catalogue would veto (unknown target) must
	// reload without error: validation defers to the refresh.
	badCfg := &config.Config{MCPServers: map[string]*config.Server{
		"srv": {Name: "srv", Command: "echo"},
		"v": {Name: "v", AliasOf: "srv", Mcpx: &config.Extras{ToolViews: map[string]config.ToolView{
			"peek": {From: "nonexistent"},
		}}},
	}}
	if _, _, err := r.Reload(badCfg); err != nil {
		t.Fatalf("expired catalog must defer view validation, got %v", err)
	}
}

// Codex P2 on PR #2976: validateViews collects argument names via
// schemaProps; a target declaring an argument through same-instance
// composition (allOf) or a local reference declares it for pin
// purposes exactly as a root-level declaration does.
func TestSchemaPropsCollectsComposedDeclarations(t *testing.T) {
	composed := schemaProps(json.RawMessage(`{"type":"object","allOf":[{"properties":{"tailnet":{"type":"string"}}}]}`))
	if !composed["tailnet"] {
		t.Errorf("allOf-declared property must count as an argument: %v", composed)
	}
	referred := schemaProps(json.RawMessage(`{"$defs":{"base":{"properties":{"limit":{"type":"number"}}}},"$ref":"#/$defs/base"}`))
	if !referred["limit"] {
		t.Errorf("reference-declared property must count as an argument: %v", referred)
	}
	root := schemaProps(json.RawMessage(`{"type":"object","properties":{"name":{"type":"string"}}}`))
	if !root["name"] || len(root) != 1 {
		t.Errorf("root declarations unchanged: %v", root)
	}
}

// Codex P2 on PR #2976 (disposition evidence): a definition referenced
// both at the top level and from a nested property is projected into
// a separate same-instance target by hideSchemaProps; the nested use
// keeps the original definition, so {"options":{}} stays invalid --
// merging a top-level pin does not supply options.tailnet.
func TestHideSchemaPropsSharedDefRootAndNested(t *testing.T) {
	schema := json.RawMessage(`{"$ref":"#/$defs/need","$defs":{"need":{"required":["tailnet"]}},"properties":{"options":{"$ref":"#/$defs/need"},"tailnet":{"type":"string"}}}`)
	out := hideSchemaProps(schema, []string{"tailnet"})
	compiled, err := schemaval.Compile(out)
	if err != nil {
		t.Fatalf("compile %s: %v", out, err)
	}
	for _, tc := range []struct {
		input string
		valid bool
	}{
		{`{}`, true},
		{`{"options":{"tailnet":"x"}}`, true},
		{`{"options":{}}`, false},
	} {
		err := compiled.ValidateJSON(json.RawMessage(tc.input))
		if (err == nil) != tc.valid {
			t.Errorf("%s: valid=%v err=%v schema=%s", tc.input, tc.valid, err, out)
		}
	}
}
