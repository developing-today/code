package config_test

import (
	"testing"

	"github.com/developing-today/code/pkgs/mcpx/internal/config"
)

// Supervised stdio is the default for file-loaded configs (#9963,
// restored after #11343's pause): a stdio server with no supervised
// key runs supervised, "supervised": false is the one opt-out into
// the legacy direct-spawn path, an explicit true stays supervised,
// and HTTP servers are untouched. The default is materialized at
// parse, so the resolved config the pool consumes carries it.
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
		t.Fatal("a stdio server with no supervised key should be supervised by default")
	}
	if c.MCPServers["optout"].Mcpx.SupervisedEnabled() {
		t.Fatal("supervised:false should not be supervised")
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
	r, err = c.Resolve("optout")
	if err != nil {
		t.Fatal(err)
	}
	if r.Mcpx.SupervisedEnabled() {
		t.Fatal("the resolved config should carry the supervised opt-out to the pool")
	}
}
