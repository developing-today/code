package cli

import (
	"encoding/json"
	"fmt"
	"sort"
	"strings"

	"github.com/developing-today/code/pkgs/mcpx/internal/api"
	"github.com/developing-today/code/pkgs/mcpx/internal/mcpserver"
	"github.com/developing-today/code/pkgs/mcpx/internal/refdoc"
	"github.com/developing-today/code/pkgs/mcpx/internal/settings"
)

// This file is the Markdown half of the manual: `mcpx man --format
// markdown` (#4909, the remainder of #1846 slices 2–4).
//
// The roff page (manual.go) and these pages are generated from the same
// declarations — the command table, the hand-flag index, the settings
// registry, the /v1 operation table, the MCP tool list and the protocol
// revision tables — so the two formats cannot disagree about what
// exists. Nothing here reads a running daemon or the network: the
// whole tree is a pure function of the binary's declarations, and two
// runs produce identical bytes.

// GlobalFlag is one flag mcpx accepts before the subcommand, declared
// in main() rather than on a command's flag set.
type GlobalFlag struct {
	Name  string
	Value string // "" for a boolean flag
	Usage string
}

// GlobalFlags is the hand-declared half of the global flags: the ones
// main() switches on before dispatch. The other half is every
// unrestricted setting (len(Commands) == 0), which
// ApplyGlobalSettingFlags binds as a global flag too; those are listed
// on the global-flags page from the registry, not repeated here.
//
// A test holds this list against cmd/mcpx/main.go, so a flag main()
// stops accepting — or starts accepting without being declared here —
// fails the build rather than drifting the page.
func GlobalFlags() []GlobalFlag {
	return []GlobalFlag{
		{Name: "json", Usage: "print JSON instead of a table, for every command"},
		{Name: "tui", Usage: "open the browser interface instead of running a command"},
		{Name: "profile", Value: "names", Usage: "apply these configuration profiles (comma or space separated); repeatable"},
		{Name: "skip-default", Usage: "do not apply the default profile"},
		{Name: "all-profiles", Usage: "apply every profile the configuration declares"},
		{Name: "config", Value: "path", Usage: "read this configuration file instead of searching for one"},
		{Name: "paths-config", Value: "dirs", Usage: "seed the configuration search path (bootstrap: decides which files the settings are read from)"},
		{Name: "version", Usage: "print the version and exit (also -v)"},
		{Name: "help", Usage: "print the help and exit (also -h)"},
	}
}

// ManualMarkdown renders the whole generated reference as a tree of
// Markdown pages, keyed by path relative to the output directory:
//
//	cli/index.md          every command, grouped, with the SUMMARY fragment
//	cli/global-flags.md   flags accepted before the subcommand
//	cli/<command>.md      one page per command
//	api/v1.md             every /v1 operation
//	api/openapi.json      the static /v1 specification (api.OpenAPI)
//	api/openapi-serve.json the serve-surface specification (cli OpenAPI)
//	mcp/tools.md          every MCP tool mcpx serves
//	protocol/tables.md    the protocol feature and capability tables
func ManualMarkdown(version string) map[string]string {
	out := map[string]string{}
	out["cli/index.md"] = cliIndexMarkdown(version)
	out["cli/global-flags.md"] = globalFlagsMarkdown()
	for _, c := range Commands() {
		out["cli/"+c.Name+".md"] = commandMarkdown(c)
	}
	out["api/v1.md"] = api.Markdown(version)
	out["api/openapi.json"] = marshalSpec(api.OpenAPI(version))
	out["api/openapi-serve.json"] = marshalSpec(OpenAPI(version))
	out["mcp/tools.md"] = mcpToolsMarkdown(version)
	out["protocol/tables.md"] = protocolTablesMarkdown()
	return out
}

// marshalSpec renders an OpenAPI document deterministically: Go's JSON
// encoder sorts map keys, and the indent is fixed, so two runs of the
// generator over the same declarations are byte-identical.
func marshalSpec(doc map[string]any) string {
	b, err := json.MarshalIndent(doc, "", "  ")
	if err != nil {
		return "{}\n"
	}
	return string(b) + "\n"
}

func cliIndexMarkdown(version string) string {
	var b strings.Builder
	b.WriteString("# The mcpx commands\n\n")
	fmt.Fprintf(&b, "mcpx %s. Every command the binary dispatches, generated "+
		"from the command table help and the man page are generated from. "+
		"Flags accepted before the subcommand are on the [global flags](global-flags.md) page.\n\n", version)
	byGroup := CommandsByGroup()
	for _, g := range commandGroups {
		cmds := byGroup[g]
		if len(cmds) == 0 {
			continue
		}
		fmt.Fprintf(&b, "## %s\n\n", commandGroupTitles[g])
		var rows [][]string
		for _, c := range cmds {
			rows = append(rows, []string{
				"[" + c.Name + "](" + c.Name + ".md)",
				c.Summary,
				"`mcpx " + c.Name + strings.TrimPrefix(" "+c.Usage, "  ") + "`",
			})
			// TrimPrefix above guards the empty-usage case; rebuild the
			// cell plainly so the shape is obvious.
			usage := "mcpx " + c.Name
			if c.Usage != "" {
				usage += " " + c.Usage
			}
			rows[len(rows)-1][2] = "`" + usage + "`"
		}
		b.WriteString(refdoc.Table([]string{"command", "summary", "usage"}, rows))
		b.WriteString("\n")
	}
	b.WriteString("## SUMMARY fragment\n\n")
	b.WriteString("For the book's `SUMMARY.md`, this reference is:\n\n")
	var frag strings.Builder
	frag.WriteString("- [Command reference](cli/index.md)\n")
	frag.WriteString("  - [Global flags](cli/global-flags.md)\n")
	names := make([]string, 0, len(Commands()))
	for _, c := range Commands() {
		names = append(names, c.Name)
	}
	sort.Strings(names)
	for _, n := range names {
		fmt.Fprintf(&frag, "  - [mcpx %s](cli/%s.md)\n", n, n)
	}
	frag.WriteString("- [The /v1 API](api/v1.md)\n")
	frag.WriteString("- [MCP tools](mcp/tools.md)\n")
	frag.WriteString("- [Protocol tables](protocol/tables.md)\n")
	b.WriteString(refdoc.Code(frag.String()))
	return b.String()
}

func globalFlagsMarkdown() string {
	var b strings.Builder
	b.WriteString("# Global flags\n\n")
	b.WriteString("Flags accepted before the subcommand: `mcpx --json ls` means " +
		"what `mcpx ls --json` does. Two kinds are declared in two places, and " +
		"both are listed here from those declarations.\n\n")
	b.WriteString("## Declared in main\n\n")
	var rows [][]string
	for _, f := range GlobalFlags() {
		name := "`--" + f.Name + "`"
		if f.Value != "" {
			name = "`--" + f.Name + " <" + f.Value + ">`"
		}
		rows = append(rows, []string{name, f.Usage})
	}
	b.WriteString(refdoc.Table([]string{"flag", "what it does"}, rows))
	b.WriteString("\n## Setting flags\n\n")
	b.WriteString("Every setting no command list restricts is also a global flag: " +
		"main binds the unrestricted registry before dispatch " +
		"(`ApplyGlobalSettingFlags`), so any of these may appear before the " +
		"subcommand as well as on it.\n\n")
	sch, err := settings.New(settings.Registry())
	if err != nil {
		return b.String()
	}
	rows = rows[:0]
	for _, set := range sch.All() {
		if len(set.Commands) != 0 || set.Bootstrap {
			continue
		}
		rows = append(rows, []string{
			"`--" + set.FlagName() + "`",
			"`" + set.Path + "`",
			set.Short,
			"`" + defaultOrEmpty(set.Default) + "`",
		})
	}
	b.WriteString(refdoc.Table([]string{"flag", "setting", "governs", "default"}, rows))
	return b.String()
}

// commandMarkdown renders one command's page: its declaration, its
// hand-declared flags (from the generated flag index the man page
// reads) and the settings declared for this command alone. Settings
// every command accepts are on the global-flags page instead of being
// repeated, identically, on every page.
func commandMarkdown(c Command) string {
	var b strings.Builder
	fmt.Fprintf(&b, "# mcpx %s\n\n%s\n\n", c.Name, c.Summary)
	usage := "mcpx " + c.Name
	if c.Usage != "" {
		usage += " " + c.Usage
	}
	b.WriteString(refdoc.Code(usage))
	if len(c.Aliases) > 0 {
		fmt.Fprintf(&b, "\nAlso spelled: %s\n", strings.Join(c.Aliases, ", "))
	}
	if c.Detail != "" {
		b.WriteString("\n" + c.Detail + "\n")
	}
	if c.Local != "" {
		b.WriteString("\nWhy it reaches no /v1 operation: " + c.Local + "\n")
	}
	for _, g := range handFlagGroups(c.Name) {
		fmt.Fprintf(&b, "\n## Flags (%s)\n\n", g.Scope)
		var rows [][]string
		for _, f := range g.Flags {
			rows = append(rows, []string{"`--" + f.Name + "`", f.Usage})
		}
		b.WriteString(refdoc.Table([]string{"flag", "what it does"}, rows))
	}
	if sch, err := settings.New(settings.Registry()); err == nil {
		var rows [][]string
		for _, set := range sch.ForCommand(c.Name) {
			if len(set.Commands) == 0 {
				continue // unrestricted: on the global-flags page
			}
			rows = append(rows, []string{
				"`--" + set.FlagName() + "`",
				"`" + set.Path + "`",
				set.Short,
				"`" + defaultOrEmpty(set.Default) + "`",
			})
		}
		if len(rows) > 0 {
			b.WriteString("\n## Settings declared for this command\n\n")
			b.WriteString(refdoc.Table([]string{"flag", "setting", "governs", "default"}, rows))
		}
	}
	if len(c.Examples) > 0 {
		b.WriteString("\n## Examples\n\n")
		b.WriteString(refdoc.Code(strings.Join(c.Examples, "\n")))
	}
	return b.String()
}

// mcpToolsMarkdown renders the served MCP tool reference: the base
// tools plus one tool per generated /v1 operation, from the same
// construction `mcpx openapi` uses — a server with the operation tools
// as extras and no backend. Proxied tools name the /v1 operation they
// reach; tools mcpx serves itself say so.
func mcpToolsMarkdown(version string) string {
	srv := mcpserver.New(nil, "mcpx", version).WithExtras(mcpserver.OpTools(nil))
	tools := srv.Tools()
	byTool := map[string]api.Op{}
	for _, op := range api.Ops() {
		byTool[op.ToolName()] = op
	}
	var b strings.Builder
	b.WriteString("# MCP tools\n\n")
	fmt.Fprintf(&b, "The %d tools mcpx %s serves, generated from the tool list "+
		"a server built with the operation tools carries. A tool that proxies "+
		"a /v1 operation links to it; the rest are mcpx's own.\n\n", len(tools), version)
	var rows [][]string
	for _, t := range tools {
		link := "mcpx's own"
		if op, ok := byTool[t.Name]; ok {
			link = fmt.Sprintf("[`%s`](../api/v1.md#%s) (`%s %s`)", op.Name, op.Name, op.Method, op.Path)
		}
		rows = append(rows, []string{"`" + t.Name + "`", t.Description, link})
	}
	b.WriteString(refdoc.Table([]string{"tool", "description", "reaches"}, rows))
	for _, t := range tools {
		fmt.Fprintf(&b, "\n## `%s`\n\n%s\n", t.Name, t.Description)
		if len(t.Annotations) > 0 {
			var ann map[string]any
			if json.Unmarshal(t.Annotations, &ann) == nil {
				keys := make([]string, 0, len(ann))
				for k := range ann {
					keys = append(keys, k)
				}
				sort.Strings(keys)
				var parts []string
				for _, k := range keys {
					parts = append(parts, fmt.Sprintf("%s: %v", k, ann[k]))
				}
				b.WriteString("\nAnnotations: " + strings.Join(parts, ", ") + "\n")
			}
		}
		var schema struct {
			Properties map[string]json.RawMessage `json:"properties"`
			Required   []string                   `json:"required"`
		}
		if json.Unmarshal(t.InputSchema, &schema) == nil && len(schema.Properties) > 0 {
			names := make([]string, 0, len(schema.Properties))
			for n := range schema.Properties {
				names = append(names, n)
			}
			sort.Strings(names)
			required := map[string]bool{}
			for _, r := range schema.Required {
				required[r] = true
			}
			b.WriteString("\n")
			var rows [][]string
			for _, n := range names {
				var prop struct {
					Type        string `json:"type"`
					Description string `json:"description"`
				}
				_ = json.Unmarshal(schema.Properties[n], &prop)
				req := "no"
				if required[n] {
					req = "yes"
				}
				rows = append(rows, []string{"`" + n + "`", prop.Type, req, prop.Description})
			}
			b.WriteString(refdoc.Table([]string{"parameter", "type", "required", "description"}, rows))
		}
	}
	return b.String()
}

// ProtocolFeaturesTable is the features × revisions matrix of
// docs/protocol.md §1, rendered from mcpserver.FeatureMatrix.
func ProtocolFeaturesTable() string {
	headers := []string{"feature"}
	for _, v := range mcpserver.Supported {
		headers = append(headers, "`"+v+"`")
	}
	matrix := mcpserver.FeatureMatrix()
	var rows [][]string
	for _, f := range mcpserver.Features {
		row := []string{"`" + string(f) + "`"}
		for _, v := range mcpserver.Supported {
			if matrix[string(f)][v] {
				row = append(row, "✓")
			} else {
				row = append(row, "—")
			}
		}
		rows = append(rows, row)
	}
	return refdoc.Table(headers, rows)
}

// ProtocolCapabilitiesTable is the declared-capabilities table of
// docs/protocol.md §2.2, rendered from
// mcpserver.DeclaredServerCapabilities — the server's own capabilities()
// evaluated for a connection that can push, per revision.
func ProtocolCapabilitiesTable() string {
	headers := []string{"capability"}
	for _, v := range mcpserver.Supported {
		headers = append(headers, "`"+v+"`")
	}
	type capRow struct {
		name string
		get  func(caps map[string]any) string
	}
	// pushNote marks a value that is declared only when the connection
	// can push; the table is evaluated with push available, and §2.2's
	// prose below the table is the footnote that says so.
	pushNote := func(yes string) string { return yes + " when the connection can push" }
	rows := []capRow{
		{"`tools.listChanged`", func(caps map[string]any) string {
			if capFlag(caps, "tools", "listChanged") {
				return pushNote("true")
			}
			return "false for mcpx's own list (fixed when the server is built); " +
				"true in pass-through mode when the connection can push"
		}},
		{"`resources.subscribe`", func(caps map[string]any) string {
			if capFlag(caps, "resources", "subscribe") {
				return pushNote("true")
			}
			return "—"
		}},
		{"`resources.listChanged`", func(caps map[string]any) string {
			if capFlag(caps, "resources", "listChanged") {
				return pushNote("true")
			}
			return "—"
		}},
		{"`prompts.listChanged`", func(caps map[string]any) string {
			if promptsFlag(caps) {
				return pushNote("true")
			}
			return "—"
		}},
		{"`completions`", func(caps map[string]any) string {
			if _, ok := caps["completions"]; ok {
				return "✓"
			}
			return "— (the revision has no such capability; the method is answered anyway)"
		}},
		{"`logging`", func(caps map[string]any) string {
			if _, ok := caps["logging"]; ok {
				return "✓"
			}
			return "— (the level travels in each request's `_meta`)"
		}},
		{"`tasks` (core)", func(caps map[string]any) string {
			if _, ok := caps["tasks"]; ok {
				return "✓"
			}
			return "—"
		}},
		{"`extensions[\"io.modelcontextprotocol/tasks\"]`", func(caps map[string]any) string {
			if extHas(caps, mcpserver.ExtTasks) {
				return "✓"
			}
			return "—"
		}},
		{"`extensions[\"io.modelcontextprotocol/skills\"]`", func(caps map[string]any) string {
			if extHas(caps, mcpserver.ExtSkills) {
				return "`{\"directoryRead\": true}`, where skills are served"
			}
			return "— (declared where skills are served)"
		}},
	}
	perVersion := make([]map[string]any, len(mcpserver.Supported))
	for i, v := range mcpserver.Supported {
		perVersion[i] = mcpserver.DeclaredServerCapabilities(v)
	}
	var out [][]string
	for _, r := range rows {
		row := []string{r.name}
		for _, caps := range perVersion {
			row = append(row, r.get(caps))
		}
		out = append(out, row)
	}
	return refdoc.Table(headers, out)
}

// capFlag reads caps[<group>][<key>] as a bool: the capability groups
// are map[string]any with bool leaves.
func capFlag(caps map[string]any, group, key string) bool {
	m, ok := caps[group].(map[string]any)
	if !ok {
		return false
	}
	v, _ := m[key].(bool)
	return v
}

func promptsFlag(caps map[string]any) bool {
	m, ok := caps["prompts"].(map[string]any)
	if !ok {
		return false
	}
	v, _ := m["listChanged"].(bool)
	return v
}

func extHas(caps map[string]any, key string) bool {
	m, ok := caps["extensions"].(map[string]any)
	if !ok {
		return false
	}
	_, ok = m[key]
	return ok
}

func protocolTablesMarkdown() string {
	var b strings.Builder
	b.WriteString("# Protocol tables\n\n")
	b.WriteString("The generated tables of the protocol document: which revision " +
		"defines which feature, and which capabilities mcpx declares per " +
		"revision. The committed copies live in `docs/protocol.md` §1 and §2.2, " +
		"guarded by the shared golden test in `internal/refdoc`.\n\n")
	b.WriteString("## Features × revisions\n\n")
	b.WriteString(ProtocolFeaturesTable())
	b.WriteString("\n## Declared capabilities\n\n")
	b.WriteString(ProtocolCapabilitiesTable())
	b.WriteString("\nPush-dependent capabilities are declared only when the connection " +
		"can push; the table is evaluated with push available. \"Can push\" is a " +
		"property of the connection, not of the server: a stdio connection can, " +
		"a legacy HTTP session can through its GET stream, a legacy POST outside " +
		"a session cannot, and a modern client receives these only on a " +
		"`subscriptions/listen` stream it opened.\n")
	return b.String()
}
