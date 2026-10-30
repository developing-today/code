package mcpserver

import (
	"context"
	"encoding/json"
	"errors"

	"github.com/dezren39/mcpx/internal/recipes"
)

// RecipeRunner runs one saved recipe with the values a caller supplied.
//
// Supplied rather than implemented here: a recipe runs through the daemon's
// existing /v1/recipes/{name}/run path, and a second runner in the protocol
// package would be a second set of bugs. The package that owns the client
// wires the two together.
type RecipeRunner func(ctx context.Context, recipe string, values map[string]any) (string, error)

// RecipeTools presents exposed recipes as tools.
//
// A recipe already declares a typed schema in its header, and this is the
// place it becomes visible to a host that will never read the file: one
// tool per recipe, named recipe_<name>, with the @param schema unchanged.
func RecipeTools(tools []recipes.Tool, run RecipeRunner) []Extra {
	out := make([]Extra, 0, len(tools))
	for _, t := range tools {
		t := t
		out = append(out, Extra{
			Tool: Tool{
				Name:        t.Name,
				Description: t.Description,
				InputSchema: t.InputSchema,
			},
			Call: func(ctx context.Context, raw json.RawMessage) (string, error) {
				if run == nil {
					return "", errors.New("this recipe tool has no runner")
				}
				values := map[string]any{}
				if len(raw) > 0 {
					if err := json.Unmarshal(raw, &values); err != nil {
						return "", err
					}
				}
				return run(ctx, t.Recipe.Name, values)
			},
		})
	}
	return out
}
