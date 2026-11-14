package codegen_test

import (
	"encoding/json"
	"os"
	"os/exec"
	"path/filepath"
	"reflect"
	"testing"

	"github.com/developing-today/code/pkgs/mcpx/internal/codegen"
)

// The behavioral counterpart to TestGeneratedSearchSupportsStableOffsetPages
// (#9598, CodeRabbit on PR #9205): instead of asserting emitted source
// strings, this executes the generated search() in deno and checks the
// actual ordering and paging. It fails if the score comparison is
// reversed, the tie-breaker is dropped, or count is always zero — the
// three regressions the source-string test cannot see.
//
// Fixture design for the query word "alpha" (a word in a namespace or
// tool name scores 10, in prose only 1; ties break by sortKey, where a
// recipe's key is "/"+name and so wins ties against tools):
//
//	score 10: recipe alpharecipe, tool alpha/zed (word in namespace),
//	          tool mid/alphaprime, tool zeta/alphabeta
//	score  1: recipe plainrecipe (prose), tool zeta/beta (prose)
//	no match: tool zeta/gamma
func searchFixtureModule() string {
	tool := func(name, desc string) codegen.Tool {
		return codegen.Tool{Name: name, Description: desc, InputSchema: json.RawMessage(`{"type":"object","properties":{}}`)}
	}
	nss := []codegen.Namespace{
		{Name: "alpha", Server: "alpha-server", Tools: []codegen.Tool{tool("zed", "a plain tool")}},
		{Name: "mid", Server: "mid-server", Tools: []codegen.Tool{tool("alphaprime", "a plain tool")}},
		{Name: "zeta", Server: "zeta-server", Tools: []codegen.Tool{
			tool("alphabeta", "a plain tool"),
			tool("beta", "alpha in prose only"),
			tool("gamma", "unrelated words entirely"),
		}},
	}
	recipes := []codegen.RecipeMeta{
		{Name: "alpharecipe", Description: "a plain recipe"},
		{Name: "plainrecipe", Description: "alpha in prose"},
	}
	return codegen.ModuleWithRecipes(nss, "http://127.0.0.1:1", "session", recipes)
}

func TestGeneratedSearchBehavior(t *testing.T) {
	deno, err := exec.LookPath("deno")
	if err != nil {
		t.Skip("deno not on PATH")
	}
	dir := t.TempDir()
	if err := os.WriteFile(filepath.Join(dir, "client.ts"), []byte(searchFixtureModule()), 0o644); err != nil {
		t.Fatal(err)
	}
	driver := `
import { search } from "./client.ts";
const id = (m) => m.kind === "tool" ? "tool:" + m.namespace + "/" + m.tool : "recipe:" + m.name;
const cases = {
  all:            search("alpha"),
  page1:          search("alpha", 2, 0),
  page2:          search("alpha", 2, 2),
  page3:          search("alpha", 2, 4),
  partialPage:    search("alpha", 4, 4),
  beyondResults:  search("alpha", 2, 99),
  negativeOffset: search("alpha", 2, -3),
  negativeLimit:  search("alpha", -1, 0),
  fracOffset:     search("alpha", 2, 1.9),
  fracLimit:      search("alpha", 2.9, 0),
  multiWord:      search("alpha prime"),
  emptyQuery:     search(""),
};
const out = {};
for (const [k, v] of Object.entries(cases)) out[k] = v.map(id);
console.log(JSON.stringify(out));
`
	if err := os.WriteFile(filepath.Join(dir, "driver.ts"), []byte(driver), 0o644); err != nil {
		t.Fatal(err)
	}
	cmd := exec.Command(deno, "run", "--quiet", "driver.ts")
	cmd.Dir = dir
	cmd.Env = append(os.Environ(), "DENO_NO_UPDATE_CHECK=1", "NO_COLOR=1")
	out, err := cmd.Output()
	if err != nil {
		if ee, ok := err.(*exec.ExitError); ok {
			t.Fatalf("deno run failed: %v\n%s", err, ee.Stderr)
		}
		t.Fatalf("deno run failed: %v", err)
	}
	var got map[string][]string
	if err := json.Unmarshal(out, &got); err != nil {
		t.Fatalf("driver output is not JSON: %v\n%s", err, out)
	}

	full := []string{
		"recipe:alpharecipe",
		"tool:alpha/zed",
		"tool:mid/alphaprime",
		"tool:zeta/alphabeta",
		"recipe:plainrecipe",
		"tool:zeta/beta",
	}
	want := map[string][]string{
		"all":            full,
		"page1":          full[0:2],
		"page2":          full[2:4],
		"page3":          full[4:6],
		"partialPage":    full[4:6],
		"beyondResults":  {},
		"negativeOffset": full[0:2],
		"negativeLimit":  {},
		"fracOffset":     full[1:3],
		"fracLimit":      full[0:2],
		"multiWord":      {"tool:mid/alphaprime"},
		"emptyQuery":     {},
	}
	for name, w := range want {
		g, ok := got[name]
		if !ok {
			t.Errorf("case %s missing from driver output", name)
			continue
		}
		if g == nil {
			g = []string{}
		}
		if !reflect.DeepEqual(g, w) {
			t.Errorf("case %s: got %v, want %v", name, g, w)
		}
	}
}
