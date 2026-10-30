package recipes

import (
	"encoding/json"
	"sort"
	"strconv"
	"strings"
)

// Recipe tools share one namespace, as adapter tools do: `<adapter>_<tool>`
// is the rule for a producer that is not a server of its own, and a recipe
// is the same shape -- one namespace, one tool per member. Recipes are not
// pooled servers, so the namespace is fixed rather than derived from a name
// somebody chose.
const ToolPrefix = "recipe_"

// DefaultToolBudget caps how many recipe tools tools/list carries.
//
// A repository with fifty saved scripts would otherwise turn every host's
// tool list into those fifty schemas, which is the problem mcpx exists to
// avoid. The cap is reported by `mcpx catalog` beside the tools it did
// expose, so a recipe that did not make it is visible rather than absent.
const DefaultToolBudget = 25

// Tool is one recipe presented as an MCP tool.
type Tool struct {
	// Recipe is the source recipe, with its body: not serialised, because a
	// listing that carried every script would be the directory again.
	Recipe Recipe `json:"-"`
	// Name is the prefixed, tool-safe name: recipe_<recipe>.
	Name string
	// Description is the summary a host shows, with the path, because two
	// projects can hold recipes of the same name.
	Description string
	// InputSchema is the recipe's @param schema, already typed.
	InputSchema json.RawMessage
}

// Skip is an exposure that did not happen, and why. Reported rather than
// dropped: a recipe that collides is a naming mistake somebody can fix.
type Skip struct {
	Recipe string `json:"recipe"`
	Name   string `json:"name"`
	Reason string `json:"reason"`
}

// ToolSet is what a set of recipes contributes to tools/list, plus the state
// needed to report it.
type ToolSet struct {
	Tools   []Tool `json:"tools"`
	Skipped []Skip `json:"skipped,omitempty"`
	// Budget is the cap that was applied; zero means DefaultToolBudget.
	Budget int `json:"budget"`
	// Hidden counts recipes that opted out with `@tool false`.
	Hidden int `json:"hidden,omitempty"`
	// Collisions counts skipped recipes whose name was already taken.
	Collisions int `json:"collisions,omitempty"`
}

// Tools builds the tool list from recipes.
//
// taken is every tool name the other producers already expose -- adapter
// tools, OpenAPI operations, mcpx's own. A recipe tool never shadows one:
// the documented rule for a collision is a loud report, and here the
// recipe is skipped and named in Skipped. Ordering is by recipe name so
// two runs of the same directory produce the same list.
func Tools(all []Recipe, taken map[string]bool, budget int) ToolSet {
	if budget <= 0 {
		budget = DefaultToolBudget
	}
	set := ToolSet{Budget: budget}
	sorted := append([]Recipe(nil), all...)
	sort.SliceStable(sorted, func(i, j int) bool { return sorted[i].Name < sorted[j].Name })

	names := map[string]bool{}
	for _, r := range sorted {
		if !r.ExposesTool() {
			set.Hidden++
			continue
		}
		name := ToolName(r.Name)
		switch {
		case name == ToolPrefix:
			set.Skipped = append(set.Skipped, Skip{Recipe: r.Name, Name: name,
				Reason: "the recipe has no usable name"})
			continue
		case taken[name] || names[name]:
			set.Collisions++
			set.Skipped = append(set.Skipped, Skip{Recipe: r.Name, Name: name,
				Reason: "a tool named " + name + " is already exposed; rename the recipe"})
			continue
		case len(set.Tools) >= budget:
			set.Skipped = append(set.Skipped, Skip{Recipe: r.Name, Name: name,
				Reason: "over the recipe tool budget of " + strconv.Itoa(budget)})
			continue
		}
		names[name] = true
		set.Tools = append(set.Tools, r.tool(name))
	}
	return set
}

// tool renders one recipe as a tool.
func (r Recipe) tool(name string) Tool {
	desc := strings.TrimSpace(r.Summary)
	if desc == "" {
		desc = "Saved mcpx recipe " + r.Name + "."
	}
	if r.Path != "" {
		desc += "\n\nRuns " + r.Path + "."
	}
	return Tool{
		Recipe:      r,
		Name:        name,
		Description: desc,
		InputSchema: FormSchema(r.Placeholders),
	}
}

// ToolName is the tool name a recipe gets: recipe_<name>, with anything that
// is not legal in an MCP tool name replaced by an underscore.
//
// Recipe names are filenames, so a dot or a dash is ordinary; a space or a
// slash is not, and a name a host refuses is worse than one that reads
// slightly differently.
func ToolName(recipe string) string {
	var b strings.Builder
	b.WriteString(ToolPrefix)
	for _, c := range recipe {
		switch {
		case c >= 'a' && c <= 'z', c >= 'A' && c <= 'Z',
			c >= '0' && c <= '9', c == '_', c == '-', c == '.':
			b.WriteRune(c)
		default:
			b.WriteByte('_')
		}
	}
	return b.String()
}
