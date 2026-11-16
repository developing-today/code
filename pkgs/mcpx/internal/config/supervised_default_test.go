package config_test

import (
	"testing"

	"github.com/developing-today/code/pkgs/mcpx/internal/config"
)

// Phase 3 of #1384 (#9963): a config loaded from a file defaults its
// stdio servers to supervised; "supervised": false is the opt-out
// into the legacy direct-spawn path, and HTTP servers are untouched.
func TestSupervisedDefaultsOnForStdioFromFile(t *testing.T) {
	dir := t.TempDir()
	p := write(t, dir, "c.json", `{
	  "mcpServers": {
	    "plain": { "command": "x" },
	    "optout": { "command": "x", "mcpx": { "supervised": false } },
	    "optin": { "command": "x", "mcpx": { "supervised": true } },
	    "web": { "url": "https://example.com/mcp" }
	  }
	}`)
	c, err := config.Load(p)
	if err != nil {
		t.Fatal(err)
	}
	if !c.MCPServers["plain"].Mcpx.SupervisedEnabled() {
		t.Fatal("a stdio server with no supervised key should default to supervised after a file load")
	}
	if c.MCPServers["optout"].Mcpx.SupervisedEnabled() {
		t.Fatal("supervised:false should opt out of the default")
	}
	if !c.MCPServers["optin"].Mcpx.SupervisedEnabled() {
		t.Fatal("supervised:true should stay supervised")
	}
	if c.MCPServers["web"].Mcpx != nil && c.MCPServers["web"].Mcpx.SupervisedEnabled() {
		t.Fatal("an HTTP server should not be defaulted to supervised")
	}
	r, err := c.Resolve("plain")
	if err != nil {
		t.Fatal(err)
	}
	if !r.Mcpx.SupervisedEnabled() {
		t.Fatal("the resolved config should carry the supervised default to the pool")
	}
}
