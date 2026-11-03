package config

import (
	"sort"
	"strings"
	"testing"
)

// #1933 slice 2: a script-backed tool view names a local command instead of
// an aliased tool. It needs no aliasOf and no from, and a server made only of
// such views has no process of its own.
func TestScriptBackedViewNeedsNoAliasOrFrom(t *testing.T) {
	c, err := parse([]byte(`{"mcpServers":{"scripts":{"mcpx":{"toolViews":{
		"echo":{"command":"cat","description":"echo the arguments"}}}}}}`))
	if err != nil {
		t.Fatalf("parse: %v", err)
	}
	r, err := c.Resolve("scripts")
	if err != nil {
		t.Fatalf("resolve: %v", err)
	}
	if !r.ScriptOnly() {
		t.Error("a server with only script views runs no process")
	}
	if !r.VisibleTool("echo") {
		t.Error("the script view should be visible under its own name")
	}
	if r.VisibleTool("cat") {
		t.Error("the command is not a callable tool name")
	}
}

func TestScriptBackedViewOnAliasKeepsItsProcess(t *testing.T) {
	c, err := parse([]byte(`{"mcpServers":{
		"base":{"command":"x"},
		"view":{"aliasOf":"base","mcpx":{"toolViews":{
			"echo":{"command":"cat","description":"echo"},
			"peek":{"from":"get_thing"}}}}}}`))
	if err != nil {
		t.Fatalf("parse: %v", err)
	}
	r, err := c.Resolve("view")
	if err != nil {
		t.Fatalf("a script view and a renamed view may share an alias: %v", err)
	}
	if r.ScriptOnly() {
		t.Error("an alias of a process server still has its process")
	}
	if !r.VisibleTool("echo") || !r.VisibleTool("peek") {
		t.Error("both views should be visible")
	}
}

// Each malformed view is refused at load with an error that names the server,
// the view and the rule it broke.
func TestToolViewValidationNamesTheRuleItBroke(t *testing.T) {
	for _, tc := range []struct {
		name, doc, want string
	}{
		{"missing command", `{"mcpServers":{"s":{"mcpx":{"toolViews":{
			"v":{"description":"d"}}}}}}`, "command is required"},
		{"from and command", `{"mcpServers":{"base":{"command":"x"},"s":{"aliasOf":"base","mcpx":{"toolViews":{
			"v":{"from":"t","command":"cat","description":"d"}}}}}}`, "exclusive"},
		{"from without alias", `{"mcpServers":{"s":{"command":"x","mcpx":{"toolViews":{
			"v":{"from":"t"}}}}}}`, "from needs aliasOf"},
		{"script without description", `{"mcpServers":{"s":{"mcpx":{"toolViews":{
			"v":{"command":"cat"}}}}}}`, "description is required"},
		{"bad timeout", `{"mcpServers":{"s":{"mcpx":{"toolViews":{
			"v":{"command":"cat","description":"d","timeout":"soon"}}}}}}`, "not a positive duration"},
		{"zero timeout", `{"mcpServers":{"s":{"mcpx":{"toolViews":{
			"v":{"command":"cat","description":"d","timeout":"0s"}}}}}}`, "not a positive duration"},
		{"inputSchema does not compile", `{"mcpServers":{"s":{"mcpx":{"toolViews":{
			"v":{"command":"cat","description":"d","inputSchema":{"$ref":"#/nowhere"}}}}}}}`, "inputSchema does not compile"},
		{"inputSchema not an object", `{"mcpServers":{"s":{"mcpx":{"toolViews":{
			"v":{"command":"cat","description":"d","inputSchema":true,"pin":{}}}}}}}`, "inputSchema must be a JSON object"},
		{"pin not in schema", `{"mcpServers":{"s":{"mcpx":{"toolViews":{
			"v":{"command":"cat","description":"d","inputSchema":{"type":"object","properties":{"a":{"type":"string"}}},"pin":{"b":1}}}}}}}`, `pin "b" is not a property`},
		{"hide not pinned", `{"mcpServers":{"s":{"mcpx":{"toolViews":{
			"v":{"command":"cat","description":"d","hide":["a"]}}}}}}`, "hide lists \"a\", which is not pinned"},
		{"hide not pinned on a renamed view", `{"mcpServers":{"base":{"command":"x"},"s":{"aliasOf":"base","mcpx":{"toolViews":{
			"v":{"from":"t","hide":["a"]}}}}}}`, "hide lists \"a\", which is not pinned"},
		{"command fields on a renamed view", `{"mcpServers":{"base":{"command":"x"},"s":{"aliasOf":"base","mcpx":{"toolViews":{
			"v":{"from":"t","args":["-x"]}}}}}}`, "apply only to a command-backed view"},
		{"empty view name", `{"mcpServers":{"s":{"mcpx":{"toolViews":{
			"":{"command":"cat","description":"d"}}}}}}`, "toolView name must not be empty"},
		{"alias of a script-only server", `{"mcpServers":{"scripts":{"mcpx":{"toolViews":{
			"echo":{"command":"cat","description":"d"}}}},"view":{"aliasOf":"scripts","mcpx":{"toolViews":{
			"peek":{"from":"echo"}}}}}}`, "from needs aliasOf on a server with a process"},
	} {
		t.Run(tc.name, func(t *testing.T) {
			c, err := parse([]byte(tc.doc))
			if err != nil {
				t.Fatalf("parse: %v", err)
			}
			// Every server in the document is resolved: the broken view
			// may sit on the alias, not on the server a case names first.
			names := make([]string, 0, len(c.MCPServers))
			for name := range c.MCPServers {
				names = append(names, name)
			}
			sort.Strings(names)
			for _, name := range names {
				if _, rerr := c.Resolve(name); rerr != nil {
					err = rerr
					break
				}
			}
			if err == nil {
				t.Fatalf("want an error containing %q", tc.want)
			}
			if !strings.Contains(err.Error(), tc.want) {
				t.Errorf("error %q should contain %q", err, tc.want)
			}
		})
	}
}

func TestScriptBackedViewValidAcceptsDeclaredPinsAndHides(t *testing.T) {
	c, err := parse([]byte(`{"mcpServers":{"s":{"mcpx":{"toolViews":{
		"v":{"command":"cat","args":["-u"],"description":"d","timeout":"5s",
			"inputSchema":{"type":"object","properties":{"tailnet":{"type":"string"},"limit":{"type":"integer"}}},
			"pin":{"tailnet":"-"},"hide":["tailnet"]}}}}}}`))
	if err != nil {
		t.Fatalf("parse: %v", err)
	}
	if _, err := c.Resolve("s"); err != nil {
		t.Fatalf("a valid script view must resolve: %v", err)
	}
}
