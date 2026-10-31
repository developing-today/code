package config_test

import (
	"testing"

	"github.com/dezren39/mcpx/internal/config"
)

func res(t *testing.T, c *config.Config, name string) *config.Resolved {
	t.Helper()
	r, err := c.Resolve(name)
	if err != nil {
		t.Fatalf("resolve %s: %v", name, err)
	}
	return r
}

func boolp(b bool) *bool { return &b }

func profileConfig() *config.Config {
	return &config.Config{MCPServers: map[string]*config.Server{
		"always": {Name: "always", Command: "a"},
		"web": {Name: "web", Command: "w", Mcpx: &config.Extras{
			Profiles: []string{"web"}, Default: boolp(false)}},
		"code": {Name: "code", Command: "c", Mcpx: &config.Extras{
			Profiles: []string{"code"}}},
	}}
}

func TestDefaultProfileIncludesEverythingNotOptedOut(t *testing.T) {
	c := profileConfig()
	p := config.Profile{}
	if !p.Includes(res(t, c, "always")) {
		t.Error("a server with no profile settings should be default-on")
	}
	if !p.Includes(res(t, c, "code")) {
		t.Error("belonging to a profile should not remove a server from the default set")
	}
	if p.Includes(res(t, c, "web")) {
		t.Error("default:false should exclude a server when no profile is asked for")
	}
}

func TestNamedProfileAddsToTheDefaultSet(t *testing.T) {
	c := profileConfig()
	p := config.Profile{Names: []string{"web"}}
	if !p.Includes(res(t, c, "web")) {
		t.Error("--profile web should include the web server")
	}
	if !p.Includes(res(t, c, "always")) {
		t.Error("--profile should add to, not replace, the default set")
	}
}

func TestSkipDefaultNarrowsToExactlyTheProfile(t *testing.T) {
	c := profileConfig()
	p := config.Profile{Names: []string{"web"}, SkipDefault: true}
	if !p.Includes(res(t, c, "web")) {
		t.Error("the requested profile must survive")
	}
	if p.Includes(res(t, c, "always")) {
		t.Error("--skip-default should drop the default set")
	}
}

func TestAllProfilesIgnoresEveryExclusion(t *testing.T) {
	c := profileConfig()
	p := config.Profile{All: true}
	for _, n := range []string{"always", "web", "code"} {
		if !p.Includes(res(t, c, n)) {
			t.Errorf("--all-profiles should include %s", n)
		}
	}
}

func TestProfileMatchingIsCaseInsensitive(t *testing.T) {
	c := profileConfig()
	if !(config.Profile{Names: []string{"WEB"}}).Includes(res(t, c, "web")) {
		t.Error("profile names should match case-insensitively")
	}
}

func TestGlobalDefaultOffFlipsTheBaseline(t *testing.T) {
	c := profileConfig()
	c.Pool = config.Extras{Default: boolp(false)}
	if (config.Profile{}).Includes(res(t, c, "code")) {
		t.Error("defaults.default=false should exclude servers that do not opt in")
	}
	// A server can still opt back in.
	c.MCPServers["code"].Mcpx.Default = boolp(true)
	if !(config.Profile{}).Includes(res(t, c, "code")) {
		t.Error("a per-server default should override the global one")
	}
}

func TestAliasSharesAPoolWhenLeasingMatches(t *testing.T) {
	c := &config.Config{MCPServers: map[string]*config.Server{
		"base":  {Name: "base", Command: "x", Args: []string{"-a"}},
		"view":  {Name: "view", AliasOf: "base", Mcpx: &config.Extras{Tools: []string{"one"}}},
		"other": {Name: "other", AliasOf: "base", Mcpx: &config.Extras{Scope: config.ScopeSession}},
	}}
	base, view, other := res(t, c, "base"), res(t, c, "view"), res(t, c, "other")

	if base.PoolID() != view.PoolID() {
		t.Error("an alias differing only in its tool subset must share the pool")
	}
	if base.PoolID() == other.PoolID() {
		t.Error("an alias with different leasing must not share the pool")
	}
	if view.Command != "base command" && view.Command != base.Command {
		t.Fatalf("an alias should inherit the process definition, got %q", view.Command)
	}
}

func TestAliasKeepsItsOwnView(t *testing.T) {
	c := &config.Config{MCPServers: map[string]*config.Server{
		"base": {Name: "base", Command: "x", Mcpx: &config.Extras{Description: "full"}},
		"view": {Name: "view", AliasOf: "base", Mcpx: &config.Extras{
			Description: "restricted", Tools: []string{"one"}}},
	}}
	v := res(t, c, "view")
	if v.Description != "restricted" {
		t.Errorf("alias description should be its own, got %q", v.Description)
	}
	if v.Namespace != "view" {
		t.Errorf("alias namespace should be its own, got %q", v.Namespace)
	}
	if !v.VisibleTool("one") || v.VisibleTool("two") {
		t.Error("alias tool filtering is not applied")
	}
}

func TestAliasChainIsRejected(t *testing.T) {
	c := &config.Config{MCPServers: map[string]*config.Server{
		"base": {Name: "base", Command: "x"},
		"a":    {Name: "a", AliasOf: "base"},
		"b":    {Name: "b", AliasOf: "a"},
	}}
	if _, err := c.Resolve("b"); err == nil {
		t.Fatal("an alias of an alias should be rejected, not silently followed")
	}
}

func TestAliasOfUnknownServerIsRejected(t *testing.T) {
	c := &config.Config{MCPServers: map[string]*config.Server{
		"a": {Name: "a", AliasOf: "nope"},
	}}
	if _, err := c.Resolve("a"); err == nil {
		t.Fatal("aliasing a server that does not exist must fail")
	}
}

func TestVisibleToolAppliesDenyOverAllow(t *testing.T) {
	c := &config.Config{MCPServers: map[string]*config.Server{
		"a": {Name: "a", Command: "x", Mcpx: &config.Extras{
			Tools: []string{"one", "two"}, ExcludeTools: []string{"two"}}},
	}}
	r := res(t, c, "a")
	if !r.VisibleTool("one") {
		t.Error("allowlisted tool should be visible")
	}
	if r.VisibleTool("two") {
		t.Error("excludeTools must win over tools")
	}
	if r.VisibleTool("three") {
		t.Error("a tool outside the allowlist should be hidden")
	}
}

func TestToolViewsResolveAndVisibility(t *testing.T) {
	// #1933 slice 1, the tailscale shape: an alias view exposing two
	// renamed tools, one with a pinned + hidden property.
	c := &config.Config{MCPServers: map[string]*config.Server{
		"tailscale": {Name: "tailscale", Command: "x"},
		"tailscale-peek": {Name: "tailscale-peek", AliasOf: "tailscale", Mcpx: &config.Extras{
			ToolViews: map[string]config.ToolView{
				"listTailnet":   {From: "list_tailnets"},
				"listHostnames": {From: "list_devices", Pin: map[string]any{"tailnet": "-"}, Hide: []string{"tailnet"}, Description: "hostnames only"},
			}}},
	}}
	v := res(t, c, "tailscale-peek")
	if !v.VisibleTool("listTailnet") || !v.VisibleTool("listHostnames") {
		t.Error("declared views should be visible under their view names")
	}
	if v.VisibleTool("list_devices") || v.VisibleTool("list_tailnets") {
		t.Error("target names are not callable through the view")
	}
	if v.VisibleTool("anything_else") {
		t.Error("a toolViews map is the whole allowlist")
	}
}

func TestToolViewsValidation(t *testing.T) {
	base := func(mcpx *config.Extras, alias bool) *config.Config {
		view := &config.Server{Name: "view", Mcpx: mcpx}
		if alias {
			view.AliasOf = "base"
		} else {
			view.Command = "x"
		}
		return &config.Config{MCPServers: map[string]*config.Server{
			"base": {Name: "base", Command: "x"},
			"view": view,
		}}
	}
	if _, err := base(&config.Extras{ToolViews: map[string]config.ToolView{"v": {From: "t"}}}, false).Resolve("view"); err == nil {
		t.Error("toolViews without aliasOf should be rejected")
	}
	if _, err := base(&config.Extras{ToolViews: map[string]config.ToolView{"v": {}}}, true).Resolve("view"); err == nil {
		t.Error("a toolView without from should be rejected")
	}
	if _, err := base(&config.Extras{ToolViews: map[string]config.ToolView{
		"v": {From: "t", Hide: []string{"a"}}}}, true).Resolve("view"); err == nil {
		t.Error("hide of an unpinned property should be rejected")
	}
	if _, err := base(&config.Extras{ToolViews: map[string]config.ToolView{
		"": {From: "t"}}}, true).Resolve("view"); err == nil {
		t.Error("an empty toolView name should be rejected")
	}
}
