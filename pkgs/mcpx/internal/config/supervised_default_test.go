package config_test

import (
	"testing"

	"github.com/developing-today/code/pkgs/mcpx/internal/config"
)

// Supervised stdio is opt-in: a stdio server with no supervised key
// runs direct-spawn; "supervised": true opts in, false stays out, and
// HTTP servers are untouched. (#9963 made it the default; that is
// paused until the supervisor socket is keyed per instance.)
func TestSupervisedIsOptInForStdioFromFile(t *testing.T) {
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
	if c.MCPServers["plain"].Mcpx.SupervisedEnabled() {
		t.Fatal("a stdio server with no supervised key should not be supervised")
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
	r, err := c.Resolve("optin")
	if err != nil {
		t.Fatal(err)
	}
	if !r.Mcpx.SupervisedEnabled() {
		t.Fatal("the resolved config should carry the supervised opt-in to the pool")
	}
}
