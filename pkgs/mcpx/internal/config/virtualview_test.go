package config

import (
	"strings"
	"testing"
)

// #1933 slice 3: a virtual namespace is a server entry with no
// command, url or aliasOf whose toolViews mount tools from several
// other servers by qualified "server.tool" From names.

func virtualConfig() *Config {
	return &Config{MCPServers: map[string]*Server{
		"alpha": {Name: "alpha", Command: "alpha-bin"},
		"beta":  {Name: "beta", URL: "http://127.0.0.1:9/beta"},
		"combo": {Name: "combo", Mcpx: &Extras{ToolViews: map[string]ToolView{
			"alphaEcho": {From: "alpha.echo", Pin: map[string]any{"fixed": "A"}, Hide: []string{"fixed"}},
			"betaEcho":  {From: "beta.echo"},
		}}},
	}}
}

func TestVirtualNamespaceResolvesMounts(t *testing.T) {
	v, err := virtualConfig().Resolve("combo")
	if err != nil {
		t.Fatal(err)
	}
	if !v.Virtual() {
		t.Fatal("combo must resolve as a virtual namespace")
	}
	tv := v.ToolViews["alphaEcho"]
	if tv.MountServer != "alpha" || tv.MountTool != "echo" {
		t.Errorf("mount split: got %q/%q, want alpha/echo", tv.MountServer, tv.MountTool)
	}
	if got := v.ToolViews["betaEcho"]; got.MountServer != "beta" || got.MountTool != "echo" {
		t.Errorf("mount split: got %q/%q, want beta/echo", got.MountServer, got.MountTool)
	}
	// The mounted-from servers are not themselves virtual.
	for _, name := range []string{"alpha", "beta"} {
		rv, err := virtualConfig().Resolve(name)
		if err != nil {
			t.Fatal(err)
		}
		if rv.Virtual() {
			t.Errorf("%s must not resolve as virtual", name)
		}
	}
}

func TestVirtualNamespaceRejectsBadMounts(t *testing.T) {
	cases := map[string]ToolView{
		"bare from names no server":   {From: "echo"},
		"unknown server":              {From: "gamma.echo"},
		"server but no tool":          {From: "alpha."},
		"mount of a virtual neighbor": {From: "other.echo"},
		"mount of itself":             {From: "combo.echo"},
	}
	for name, tv := range cases {
		c := virtualConfig()
		c.MCPServers["other"] = &Server{Name: "other", Mcpx: &Extras{ToolViews: map[string]ToolView{
			"x": {From: "alpha.echo"},
		}}}
		c.MCPServers["combo"].Mcpx.ToolViews = map[string]ToolView{"bad": tv}
		if _, err := c.Resolve("combo"); err == nil {
			t.Errorf("%s: Resolve must refuse, got nil error", name)
		} else if !strings.Contains(err.Error(), "combo") {
			t.Errorf("%s: error must name the virtual server, got %v", name, err)
		}
	}
}

func TestVirtualMountSplitsAtLongestServerName(t *testing.T) {
	c := &Config{MCPServers: map[string]*Server{
		"a":   {Name: "a", Command: "a-bin"},
		"a.b": {Name: "a.b", Command: "ab-bin"},
		"pick": {Name: "pick", Mcpx: &Extras{ToolViews: map[string]ToolView{
			"t": {From: "a.b.tool"},
			"u": {From: "a.tool"},
		}}},
	}}
	v, err := c.Resolve("pick")
	if err != nil {
		t.Fatal(err)
	}
	if got := v.ToolViews["t"]; got.MountServer != "a.b" || got.MountTool != "tool" {
		t.Errorf("longest server name must win: got %q/%q, want a.b/tool", got.MountServer, got.MountTool)
	}
	if got := v.ToolViews["u"]; got.MountServer != "a" || got.MountTool != "tool" {
		t.Errorf("shorter name still splits: got %q/%q, want a/tool", got.MountServer, got.MountTool)
	}
}

func TestVirtualNamespaceMayAlsoCarryScriptViews(t *testing.T) {
	c := virtualConfig()
	c.MCPServers["combo"].Mcpx.ToolViews["local"] = ToolView{Command: "cat", Description: "local script"}
	v, err := c.Resolve("combo")
	if err != nil {
		t.Fatal(err)
	}
	if !v.Virtual() {
		t.Error("a namespace mounting tools is virtual even with a script view beside the mounts")
	}
	if got := v.ToolViews["local"]; got.MountServer != "" || got.Command != "cat" {
		t.Errorf("script view must stay a script view: %+v", got)
	}
}

func TestAliasViewFromIsNotAMount(t *testing.T) {
	c := &Config{MCPServers: map[string]*Server{
		"base": {Name: "base", Command: "base-bin"},
		"view": {Name: "view", AliasOf: "base", Mcpx: &Extras{ToolViews: map[string]ToolView{
			"renamed": {From: "echo"},
		}}},
	}}
	v, err := c.Resolve("view")
	if err != nil {
		t.Fatal(err)
	}
	if v.Virtual() {
		t.Error("an alias view is not a virtual namespace")
	}
	if got := v.ToolViews["renamed"]; got.MountServer != "" {
		t.Errorf("an alias view's bare From must not split into a mount: %+v", got)
	}
}
