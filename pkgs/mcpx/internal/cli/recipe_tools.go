package cli

import (
	"context"
	"errors"
	"fmt"
	"strings"

	"github.com/developing-today/code/pkgs/mcpx/internal/mcpserver"
	"github.com/developing-today/code/pkgs/mcpx/internal/recipes"
)

// recipeToolSet is what the recipes on the search path contribute to
// tools/list, given the names the other producers already expose.
//
// The list comes from the daemon, which already resolved the script search
// path for this project. Nothing here reads a directory a second time.
func (a *App) recipeToolSet(ctx context.Context, c *Client, taken map[string]bool) (recipes.ToolSet, error) {
	list, err := c.Recipes(ctx)
	if err != nil {
		return recipes.ToolSet{}, err
	}
	return recipes.Tools(list.Recipes, taken, recipes.DefaultToolBudget), nil
}

// recipeToolExtras is recipeToolSet as MCP tools.
//
// Calling one posts to /v1/recipes/{name}/run, the path `mcpx recipes run`
// already uses: rendering, diagnostics, autonomy and the runner are the
// daemon's, not a copy here.
func (a *App) recipeToolExtras(ctx context.Context, c *Client, taken map[string]bool) ([]mcpserver.Extra, recipes.ToolSet, error) {
	set, err := a.recipeToolSet(ctx, c, taken)
	if err != nil {
		return nil, set, err
	}
	run := func(ctx context.Context, name string, values map[string]any) (string, error) {
		c, cerr := a.ensure(ctx)
		if cerr != nil {
			return "", cerr
		}
		// The caller's own session, not this process's: recipes may call
		// session-scoped namespaces, and two MCP clients must not share
		// one upstream session by accident (see mcpCaller).
		res, rerr := c.RunRecipe(ctx, name, values, "run", a.mcpCaller(ctx).SessionID)
		if rerr != nil {
			return "", rerr
		}
		return renderRecipeResult(res)
	}
	return mcpserver.RecipeTools(set.Tools, run), set, nil
}

// renderRecipeResult favours the recipe's stdout, as the adapter path does:
// that is what the script has to say. Diagnostics and a non-zero exit are
// appended rather than substituted, and a run that produced nothing is
// reported as the message the daemon gave instead of an empty string.
func renderRecipeResult(res *Resolution) (string, error) {
	if res == nil {
		return "", errors.New("the recipe produced no answer")
	}
	if res.Result == nil {
		if res.Message != "" {
			return "", errors.New(res.Message)
		}
		return res.Source, nil
	}
	var b strings.Builder
	b.WriteString(strings.TrimRight(res.Result.Stdout, "\n"))
	if s := strings.TrimSpace(res.Result.Stderr); s != "" {
		if b.Len() > 0 {
			b.WriteString("\n\n")
		}
		b.WriteString("stderr:\n" + s)
	}
	if res.Result.ExitCode != 0 {
		fmt.Fprintf(&b, "\n\n(exit %d)", res.Result.ExitCode)
	}
	if res.Result.Error != "" {
		return b.String(), errors.New(res.Result.Error)
	}
	if res.Result.ExitCode != 0 {
		// A failed recipe is a failed tool call: the output above stays
		// attached, but the MCP result must carry an error or callers
		// read the failure as success.
		return b.String(), fmt.Errorf("recipe exited with code %d", res.Result.ExitCode)
	}
	if b.Len() == 0 {
		return "(no output)", nil
	}
	return b.String(), nil
}

// printRecipeToolState reports what recipes add to tools/list, against what
// budget, and which ones did not make it.
//
// `mcpx catalog` is where somebody looks to find out what is exposed, so a
// recipe skipped for a name collision is named here rather than silently
// absent from the only list that would have shown it.
func (a *App) printRecipeToolState(ctx context.Context, c *Client) {
	set, err := a.recipeToolSet(ctx, c, a.takenToolNames())
	fmt.Print(formatRecipeToolState(set, err))
}

// formatRecipeToolState renders the recipe block `mcpx catalog` ends with.
//
// Split from the listing so it can be checked without a daemon: the point of
// the block is that a skip is named, and that is a string contract.
func formatRecipeToolState(set recipes.ToolSet, err error) string {
	if err != nil {
		return fmt.Sprintf("\nrecipe tools: unavailable (%v)\n", err)
	}
	var b strings.Builder
	hidden := ""
	if set.Hidden > 0 {
		hidden = fmt.Sprintf(", %d opted out", set.Hidden)
	}
	fmt.Fprintf(&b, "\nrecipe tools: %d exposed (budget %d%s)\n",
		len(set.Tools), set.Budget, hidden)
	for _, s := range set.Skipped {
		fmt.Fprintf(&b, "  %s: %s\n", s.Name, s.Reason)
	}
	return b.String()
}

// takenToolNames is every name the other producers already claim, as far as
// they can be known without starting anything: adapter tools and the /v1
// operation tools. OpenAPI operations are read over the network and are not
// included here; building the MCP surface itself checks against them.
func (a *App) takenToolNames() map[string]bool {
	taken := map[string]bool{}
	if specs, err := a.loadAdapters(); err == nil {
		for _, s := range specs {
			for _, t := range s.Tools {
				taken[s.Name+"_"+t.Name] = true
			}
		}
	}
	for _, e := range a.v1OpTools() {
		taken[e.Tool.Name] = true
	}
	return taken
}
