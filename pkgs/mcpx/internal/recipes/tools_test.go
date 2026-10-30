package recipes_test

import (
	"encoding/json"
	"strings"
	"testing"

	"github.com/dezren39/mcpx/internal/recipes"
)

const exposed = `// Close stale issues in a repository.
// @param repo:string        which repository, as owner/name
// @param days:number = 30   how old counts as stale
// @param dryRun:boolean?    report without closing
const found = await demo.search({ repo: @repo, olderThanDays: @days });
`

func recipe(name, src string) recipes.Recipe {
	return recipes.Parse(name, "/tmp/"+name+".ts", src)
}

func TestARecipeBecomesAToolWithItsParamSchema(t *testing.T) {
	set := recipes.Tools([]recipes.Recipe{recipe("close-stale", exposed)}, nil, 0)
	if len(set.Tools) != 1 {
		t.Fatalf("expected one tool, got %+v", set)
	}
	tool := set.Tools[0]
	if tool.Name != "recipe_close-stale" {
		t.Errorf("name = %q; recipe tools live under one namespace", tool.Name)
	}
	var schema struct {
		Type       string `json:"type"`
		Required   []string
		Properties map[string]struct {
			Type        string `json:"type"`
			Description string `json:"description"`
			Default     any    `json:"default"`
		} `json:"properties"`
	}
	if err := json.Unmarshal(tool.InputSchema, &schema); err != nil {
		t.Fatalf("schema: %v in %s", err, tool.InputSchema)
	}
	if schema.Type != "object" {
		t.Errorf("schema type = %q", schema.Type)
	}
	if p := schema.Properties["repo"]; p.Type != "string" || p.Description == "" {
		t.Errorf("repo = %+v", p)
	}
	if p := schema.Properties["days"]; p.Type != "number" || p.Default != 30.0 {
		t.Errorf("days should carry its type and default: %+v", p)
	}
	if p := schema.Properties["dryRun"]; p.Type != "boolean" {
		t.Errorf("dryRun = %+v", p)
	}
	if strings.Join(schema.Required, ",") != "repo" {
		t.Errorf("required = %v; only a hole with no default is required", schema.Required)
	}
}

func TestOptOutMarkerRemovesTheRecipeFromTheToolSet(t *testing.T) {
	off := recipe("secret", exposed+"// @tool false\n")
	if off.ExposesTool() {
		t.Error("@tool false should opt the recipe out")
	}
	set := recipes.Tools([]recipes.Recipe{off}, nil, 0)
	if len(set.Tools) != 0 || set.Hidden != 1 {
		t.Errorf("an opted-out recipe must not be exposed: %+v", set)
	}
	// The default is on, which is what makes the marker the exception.
	if !recipe("normal", exposed).ExposesTool() {
		t.Error("a recipe without a marker should be exposed")
	}
}

func TestACollidingRecipeIsSkippedAndReported(t *testing.T) {
	// Adapter and v1-op tools are already in tools/list; a recipe never
	// shadows one, and `mcpx catalog` says which one it was.
	set := recipes.Tools(
		[]recipes.Recipe{recipe("close-stale", exposed)},
		map[string]bool{"recipe_close-stale": true}, 0)
	if len(set.Tools) != 0 {
		t.Fatalf("a colliding recipe must not be exposed: %+v", set.Tools)
	}
	if set.Collisions != 1 || len(set.Skipped) != 1 {
		t.Fatalf("the collision should be reported: %+v", set)
	}
	if !strings.Contains(set.Skipped[0].Reason, "already exposed") {
		t.Errorf("reason = %q", set.Skipped[0].Reason)
	}
}

func TestTheBudgetIsAppliedAndReported(t *testing.T) {
	all := []recipes.Recipe{
		recipe("a", exposed), recipe("b", exposed), recipe("c", exposed),
	}
	set := recipes.Tools(all, nil, 2)
	if set.Budget != 2 || len(set.Tools) != 2 || len(set.Skipped) != 1 {
		t.Fatalf("budget 2 of 3: %+v", set)
	}
	if !strings.Contains(set.Skipped[0].Reason, "budget") {
		t.Errorf("reason = %q", set.Skipped[0].Reason)
	}
	// Unspecified is the built-in, not unlimited.
	if got := recipes.Tools(all, nil, 0).Budget; got != recipes.DefaultToolBudget {
		t.Errorf("budget = %d", got)
	}
}

func TestExposureIsIdempotent(t *testing.T) {
	all := []recipes.Recipe{recipe("a", exposed), recipe("b", exposed)}
	first := recipes.Tools(all, nil, 0)
	second := recipes.Tools(all, nil, 0)
	if len(first.Tools) != len(second.Tools) {
		t.Fatalf("two runs differ: %d then %d", len(first.Tools), len(second.Tools))
	}
	for i := range first.Tools {
		if first.Tools[i].Name != second.Tools[i].Name {
			t.Errorf("tool %d: %q then %q", i, first.Tools[i].Name, second.Tools[i].Name)
		}
	}
}

func TestToolNamesAreSafeForAHost(t *testing.T) {
	if got := recipes.ToolName("my recipe"); got != "recipe_my_recipe" {
		t.Errorf("got %q", got)
	}
	if got := recipes.ToolName("close-stale"); got != "recipe_close-stale" {
		t.Errorf("dots and dashes are legal and should survive: %q", got)
	}
}
