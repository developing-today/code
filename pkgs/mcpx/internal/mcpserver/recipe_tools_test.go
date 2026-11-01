package mcpserver_test

import (
	"context"
	"encoding/json"
	"errors"
	"strings"
	"testing"

	"github.com/developing-today/code/pkgs/mcpx/internal/mcpserver"
	"github.com/developing-today/code/pkgs/mcpx/internal/recipes"
)

const recipeSource = `// Close stale issues in a repository.
// @param repo:string        which repository, as owner/name
// @param days:number = 30   how old counts as stale
const found = await demo.search({ repo: @repo, olderThanDays: @days });
`

func parseRecipe(name, src string) recipes.Recipe {
	return recipes.Parse(name, "/tmp/"+name+".ts", src)
}

// recipeSurface is a server whose whole surface is the recipe tools, as
// `mcpx serve` builds it: the tools a host reads and calls.
func recipeSurface(t *testing.T, all []recipes.Recipe,
	run mcpserver.RecipeRunner) *mcpserver.Server {
	t.Helper()
	set := recipes.Tools(all, nil, 0)
	if len(set.Tools) == 0 {
		t.Fatalf("no recipe tools to expose: %+v", set)
	}
	srv := mcpserver.New(nil, "mcpx", "test")
	srv.ExtrasOnly = true
	return srv.WithExtras(mcpserver.RecipeTools(set.Tools, run))
}

func TestARecipeAppearsInToolsListWithItsParamSchema(t *testing.T) {
	srv := recipeSurface(t,
		[]recipes.Recipe{parseRecipe("close-stale", recipeSource)},
		func(context.Context, string, map[string]any) (string, error) { return "ok", nil })

	var found *mcpserver.Tool
	for _, tool := range srv.Tools() {
		if tool.Name == "recipe_close-stale" {
			found = &tool
		}
	}
	if found == nil {
		t.Fatalf("recipe_close-stale is not in %v", names(srv.Tools()))
	}
	var schema struct {
		Properties map[string]struct {
			Type string `json:"type"`
		} `json:"properties"`
		Required []string
	}
	if err := json.Unmarshal(found.InputSchema, &schema); err != nil {
		t.Fatalf("schema: %v", err)
	}
	if schema.Properties["repo"].Type != "string" ||
		schema.Properties["days"].Type != "number" {
		t.Errorf("the @param types should survive: %+v", schema.Properties)
	}
}

func TestARecipeToolIsCallableThroughTheMCPToolPath(t *testing.T) {
	var gotName string
	var gotValues map[string]any
	srv := recipeSurface(t,
		[]recipes.Recipe{parseRecipe("close-stale", recipeSource)},
		func(_ context.Context, name string, values map[string]any) (string, error) {
			gotName, gotValues = name, values
			return "closed 3 issues", nil
		})
	out, err := srv.InvokeTool(context.Background(), "recipe_close-stale",
		json.RawMessage(`{"repo":"acme/web","days":7}`))
	if err != nil {
		t.Fatal(err)
	}
	if out != "closed 3 issues" {
		t.Errorf("out = %q", out)
	}
	if gotName != "close-stale" {
		t.Errorf("the runner should be told the recipe, got %q", gotName)
	}
	if gotValues["repo"] != "acme/web" || gotValues["days"] != 7.0 {
		t.Errorf("values = %+v", gotValues)
	}
	// A runtime failure is an error, not a result.
	failing := recipeSurface(t,
		[]recipes.Recipe{parseRecipe("close-stale", recipeSource)},
		func(context.Context, string, map[string]any) (string, error) {
			return "", errors.New("needs repo")
		})
	if _, err := failing.InvokeTool(context.Background(), "recipe_close-stale",
		json.RawMessage(`{}`)); err == nil {
		t.Error("a failed run should be an error")
	}
}

func TestAnOptedOutRecipeIsAbsentFromToolsList(t *testing.T) {
	all := []recipes.Recipe{
		parseRecipe("shown", recipeSource),
		parseRecipe("hidden", recipeSource+"// @tool false\n"),
	}
	srv := recipeSurface(t, all,
		func(context.Context, string, map[string]any) (string, error) { return "ok", nil })
	listed := strings.Join(names(srv.Tools()), ",")
	if !strings.Contains(listed, "recipe_shown") {
		t.Errorf("the default recipe should be listed: %v", listed)
	}
	if strings.Contains(listed, "recipe_hidden") {
		t.Errorf("@tool false should keep the recipe out: %v", listed)
	}
}

func TestARecipeToolNameThatCollidesIsNotExposed(t *testing.T) {
	// The documented rule is one tool name per surface; a producer that
	// cannot have its name does not shadow the one that has it.
	set := recipes.Tools(
		[]recipes.Recipe{parseRecipe("close-stale", recipeSource)},
		map[string]bool{"recipe_close-stale": true}, 0)
	if len(set.Tools) != 0 || len(set.Skipped) != 1 {
		t.Fatalf("collision should skip, not shadow: %+v", set)
	}
}

func TestReListingDoesNotDuplicateRecipeTools(t *testing.T) {
	srv := recipeSurface(t,
		[]recipes.Recipe{parseRecipe("close-stale", recipeSource)},
		func(context.Context, string, map[string]any) (string, error) { return "ok", nil })
	first := names(srv.Tools())
	second := names(srv.Tools())
	if strings.Join(first, ",") != strings.Join(second, ",") {
		t.Errorf("two listings differ: %v then %v", first, second)
	}
	seen := map[string]bool{}
	for _, n := range second {
		if seen[n] {
			t.Errorf("%s appears twice in %v", n, second)
		}
		seen[n] = true
	}
}

func names(tools []mcpserver.Tool) []string {
	out := make([]string, 0, len(tools))
	for _, t := range tools {
		out = append(out, t.Name)
	}
	return out
}

func TestAFailedRecipeCallKeepsItsOutputInTheErrorResult(t *testing.T) {
	// Regression (Codex on #2504): a recipe that printed diagnostics and
	// exited nonzero had that output dropped by the tools/call handler,
	// which sent only the error text, so the caller saw "recipe exited
	// with code 1" and none of what the recipe actually said.
	srv := recipeSurface(t,
		[]recipes.Recipe{parseRecipe("close-stale", recipeSource)},
		func(context.Context, string, map[string]any) (string, error) {
			return "boom: the repo is gone\n\n(exit 1)", errors.New("recipe exited with code 1")
		})
	text, isErr := call(t, srv, "recipe_close-stale", map[string]any{})
	if !isErr {
		t.Fatalf("a failed recipe call should be an isError result, got %q", text)
	}
	if !strings.Contains(text, "boom: the repo is gone") {
		t.Errorf("the recipe output should survive into the result: %q", text)
	}
	if !strings.Contains(text, "recipe exited with code 1") {
		t.Errorf("the error should still be named: %q", text)
	}
}
