package cli_test

import (
	"os"
	"strings"
	"testing"

	"github.com/developing-today/code/pkgs/mcpx/internal/api"
	"github.com/developing-today/code/pkgs/mcpx/internal/cli"
	"github.com/developing-today/code/pkgs/mcpx/internal/mcpserver"
)

// TestManualMarkdownCoversEverySurface is the completeness half of
// #4909's acceptance: every command, /v1 operation and served MCP tool
// appears in the generated reference, and the tree is deterministic.
func TestManualMarkdownCoversEverySurface(t *testing.T) {
	tree := cli.ManualMarkdown("test-version")
	again := cli.ManualMarkdown("test-version")
	if len(tree) != len(again) {
		t.Fatalf("tree size differs between runs: %d vs %d", len(tree), len(again))
	}
	for path, body := range tree {
		if again[path] != body {
			t.Errorf("%s differs between runs; the reference must be deterministic", path)
		}
	}

	for _, c := range cli.Commands() {
		page, ok := tree["cli/"+c.Name+".md"]
		if !ok {
			t.Errorf("no page for command %s", c.Name)
			continue
		}
		if !strings.Contains(page, c.Summary) {
			t.Errorf("page for %s does not carry its summary", c.Name)
		}
	}
	index := tree["cli/index.md"]
	for _, c := range cli.Commands() {
		if !strings.Contains(index, "("+c.Name+".md)") {
			t.Errorf("index does not link command %s", c.Name)
		}
	}

	v1 := tree["api/v1.md"]
	for _, op := range api.Ops() {
		if !strings.Contains(v1, "`"+op.Name+"`") {
			t.Errorf("/v1 page omits operation %s", op.Name)
		}
		if !strings.Contains(v1, op.Method+" "+op.Path) {
			t.Errorf("/v1 page omits route for %s", op.Name)
		}
	}

	srv := mcpserver.New(nil, "mcpx", "test-version").WithExtras(mcpserver.OpTools(nil))
	tools := tree["mcp/tools.md"]
	for _, tool := range srv.Tools() {
		if !strings.Contains(tools, "`"+tool.Name+"`") {
			t.Errorf("MCP tools page omits tool %s", tool.Name)
		}
	}

	proto := tree["protocol/tables.md"]
	for _, v := range mcpserver.Supported {
		if !strings.Contains(proto, v) {
			t.Errorf("protocol tables omit revision %s", v)
		}
	}
	for _, f := range mcpserver.Features {
		if !strings.Contains(proto, string(f)) {
			t.Errorf("protocol tables omit feature %s", f)
		}
	}

	if got := tree["api/openapi.json"]; !strings.Contains(got, "\"openapi\": \"3.1.0\"") {
		t.Errorf("static openapi.json is not the api.OpenAPI document")
	}
	if got := tree["api/openapi-serve.json"]; !strings.Contains(got, "mcpx_namespaces") {
		t.Errorf("serve openapi document omits the MCP tool paths")
	}
}

// TestGlobalFlagsAreDeclaredInMain holds GlobalFlags() against the
// switch in cli.Run (run.go): since #1980 stage 1 the entrypoint's
// global-flag switch lives in Run, not cmd/mcpx/main.go -- a flag Run
// accepts must be on the list, and a flag on the list must be one Run
// accepts.
func TestGlobalFlagsAreDeclaredInMain(t *testing.T) {
	b, err := os.ReadFile("run.go")
	if err != nil {
		t.Fatal(err)
	}
	src := string(b)
	for _, f := range cli.GlobalFlags() {
		if !strings.Contains(src, "\"--"+f.Name+"\"") && !strings.Contains(src, "\"-"+f.Name+"\"") {
			t.Errorf("global flag --%s is declared in GlobalFlags but cli.Run never switches on it", f.Name)
		}
	}
}
