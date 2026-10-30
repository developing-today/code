package settings_test

import (
	"fmt"
	"os"
	"strings"
	"testing"

	"github.com/dezren39/mcpx/internal/settings"
)

// update is declared in scriptenv_test.go, which regenerates
// docs/environment.md the same way.

const pluginReadmePath = "../../plugin/opencode/README.md"

// The markers separate the prose, which is written, from the table, which is
// not. Spelled the way docs/configuration.md spells them, so the generated
// documents look the same to a reader who opens both.
const (
	pluginReadmeBegin = "<!-- BEGIN GENERATED: settings.Registry() plugin.*, plus the two daemon options; `go test ./internal/settings -run TestThePluginReadmeListsEveryOption -update` -->"
	pluginReadmeEnd   = "<!-- END GENERATED -->"
)

// TestThePluginReadmeListsEveryOption keeps the opencode plugin's options table
// honest.
//
// The table was written by hand and read by nobody: discoveryRetry was missing
// from it while the registry declared it, and a wrong variable name or default
// in it would have sent people to a knob that does nothing. The plugin cannot
// ask Go to resolve a setting either -- it reads process.env directly -- so
// this table is the only place a plugin user sees the whole surface.
//
// So it is generated now, the way docs/configuration.md is: this test rewrites
// the region between the markers from settings.Registry() and fails when the
// README differs.
func TestThePluginReadmeListsEveryOption(t *testing.T) {
	doc := readPluginReadme(t)
	want, err := pluginReadmeWithOptions(doc, mustSchema(t))
	if err != nil {
		t.Fatal(err)
	}
	if doc == want {
		return
	}
	if *update {
		if err := os.WriteFile(pluginReadmePath, []byte(want), 0o644); err != nil {
			t.Fatal(err)
		}
		return
	}
	t.Errorf("plugin/opencode/README.md's options table no longer matches the "+
		"settings registry: %s\nregenerate it with\n"+
		"  go test ./internal/settings -run TestThePluginReadmeListsEveryOption -update",
		pluginReadmeDrift(doc, want))
}

// pluginReadmeWithOptions returns doc with the table between the markers
// replaced by the registry's rendering.
//
// The drift test and the test that proves the drift test bites both go through
// here, so what the negative test exercises is the comparison that runs.
func pluginReadmeWithOptions(doc string, s *settings.Schema) (string, error) {
	i := strings.Index(doc, pluginReadmeBegin)
	j := strings.Index(doc, pluginReadmeEnd)
	if i < 0 || j < i {
		return "", fmt.Errorf("plugin/opencode/README.md has lost the markers that "+
			"separate its prose from its generated options table (%s ... %s)",
			pluginReadmeBegin, pluginReadmeEnd)
	}
	table, err := pluginReadmeTable(s)
	if err != nil {
		return "", err
	}
	return doc[:i+len(pluginReadmeBegin)] + "\n\n" + table + doc[j:], nil
}

// pluginReadmeTable renders the options table: a header, then the two options
// that name a daemon, then every plugin.* setting.
func pluginReadmeTable(s *settings.Schema) (string, error) {
	rows, err := pluginReadmeRows(s)
	if err != nil {
		return "", err
	}
	var b strings.Builder
	b.WriteString("| option | variable | default | effect |\n")
	b.WriteString("| --- | --- | --- | --- |\n")
	for _, row := range rows {
		fmt.Fprintf(&b, "| `%s` | %s | %s | %s |\n",
			row.option, row.variable, row.def, cell(row.effect))
	}
	b.WriteString("\n")
	return b.String(), nil
}

// pluginReadmeOption is one row.
type pluginReadmeOption struct {
	option   string
	variable string
	def      string
	effect   string
}

func pluginReadmeRows(s *settings.Schema) ([]pluginReadmeOption, error) {
	rows, err := daemonOptionRows(s)
	if err != nil {
		return nil, err
	}
	// settings.Registry() order, which is the declaration order: bin, binArgs
	// and backend first, then the switches. Every plugin.* setting is here, and
	// TestThePluginOptionTableCoversTheRegistry says so.
	for _, set := range settings.Registry() {
		if !strings.HasPrefix(set.Path, "plugin.") {
			continue
		}
		if set.Short == "" {
			return nil, fmt.Errorf("%s has no Short, so the table has no effect to print", set.Path)
		}
		rows = append(rows, pluginReadmeOption{
			option:   strings.TrimPrefix(set.Path, "plugin."),
			variable: ticked(envNames(set)),
			def:      defaultCell(set.Default),
			effect:   sentence(set.Short),
		})
	}
	return rows, nil
}

// daemonOptions are the two rows the table puts first, because they are read
// before anything else and either one stops the ladder.
//
// Only one of them is a setting. daemon.endpoint is in the registry -- the
// plugin option and the CLI flag are the same knob -- so its variable and
// default come from there. daemon.socket is not a setting at all: MCPX_SOCKET
// names a socket for the daemon and for the plugin, and scriptenv.go is where
// it is declared. The effects are the existing prose rather than the registry's
// Short: daemon.endpoint's Short is the CLI's view of it ("a daemon somewhere
// else, instead of the local socket") and cannot say what a reader of this
// ladder needs, and a socket has no setting to take a Short from.
var daemonOptions = []struct {
	option  string
	setting string
	env     []string // spellings beyond the setting's, checked against ScriptEnv
	effect  string
}{
	{
		option:  "endpoint",
		setting: "daemon.endpoint",
		// MCPX_ENDPOINT is the plugin's fallback for MCPX_DAEMON_ENDPOINT
		// (mcpx/daemon.ts) and a script-env variable, not an alias the
		// registry declares: mcpx itself does not read it as that setting.
		env:    []string{"MCPX_ENDPOINT"},
		effect: "a daemon named outright: `http://host:port` or `unix:///path`. Stops the ladder.",
	},
	{
		option: "socket",
		env:    []string{"MCPX_SOCKET"},
		effect: "a socket named outright. Stops the ladder.",
	},
}

func daemonOptionRows(s *settings.Schema) ([]pluginReadmeOption, error) {
	declared := scriptEnvNames()
	var rows []pluginReadmeOption
	for _, o := range daemonOptions {
		names := append([]string(nil), o.env...)
		def := emptyDefault
		if o.setting != "" {
			set, ok := s.Lookup(o.setting)
			if !ok {
				return nil, fmt.Errorf("%s is in the plugin README and not in the registry", o.setting)
			}
			names = append(envNames(*set), names...)
			def = defaultCell(set.Default)
		}
		for _, name := range names {
			if !declared[name] {
				return nil, fmt.Errorf("%s is in the plugin README's options table and "+
					"scriptenv.go does not declare it", name)
			}
		}
		rows = append(rows, pluginReadmeOption{
			option:   o.option,
			variable: ticked(names),
			def:      def,
			effect:   o.effect,
		})
	}
	return rows, nil
}

// emptyDefault is how a setting with no default is shown. An empty cell reads
// as an omission, which is a different thing.
const emptyDefault = "—"

func envNames(set settings.Setting) []string {
	names := []string{set.EnvName()}
	return append(names, set.EnvAliases...)
}

func ticked(names []string) string {
	out := make([]string, 0, len(names))
	for _, n := range names {
		out = append(out, "`"+n+"`")
	}
	return strings.Join(out, ", ")
}

func defaultCell(def string) string {
	if def == "" {
		return emptyDefault
	}
	return "`" + def + "`"
}

// sentence is the effect column: the registry's one-line Short with the full
// stop every other row in this table ends on.
func sentence(short string) string {
	if strings.HasSuffix(short, ".") || strings.HasSuffix(short, "!") || strings.HasSuffix(short, "?") {
		return short
	}
	return short + "."
}

func scriptEnvNames() map[string]bool {
	out := map[string]bool{}
	for _, v := range settings.ScriptEnv() {
		out[v.Name] = true
	}
	return out
}

// pluginReadmeDrift describes the first line where doc and want disagree, so a
// failure says what drifted rather than "the files differ".
func pluginReadmeDrift(doc, want string) string {
	got, exp := strings.Split(doc, "\n"), strings.Split(want, "\n")
	for i := 0; i < len(got) && i < len(exp); i++ {
		if got[i] != exp[i] {
			return fmt.Sprintf("line %d:\n  README:   %q\n  registry: %q", i+1, got[i], exp[i])
		}
	}
	if len(got) != len(exp) {
		return fmt.Sprintf("the README has %d lines and the registry renders %d", len(got), len(exp))
	}
	return "no difference"
}

// TestThePluginOptionTableCoversTheRegistry pins the option set itself.
//
// The drift test compares bytes, which cannot notice a generator that silently
// stopped reading the registry -- a filter typo would render a stable, short
// table and pass. This says every plugin.* setting has exactly one row and the
// two daemon options are still there.
func TestThePluginOptionTableCoversTheRegistry(t *testing.T) {
	s := mustSchema(t)
	rows, err := pluginReadmeRows(s)
	if err != nil {
		t.Fatal(err)
	}
	seen := map[string]bool{}
	for _, row := range rows {
		if seen[row.option] {
			t.Errorf("%s appears twice in the options table", row.option)
		}
		seen[row.option] = true
	}
	plugin := 0
	for _, set := range settings.Registry() {
		if !strings.HasPrefix(set.Path, "plugin.") {
			continue
		}
		plugin++
		if !seen[strings.TrimPrefix(set.Path, "plugin.")] {
			t.Errorf("%s is a plugin setting and the options table has no row for it", set.Path)
		}
	}
	for _, o := range daemonOptions {
		if !seen[o.option] {
			t.Errorf("the %s row is missing from the options table", o.option)
		}
	}
	if want := plugin + len(daemonOptions); len(rows) != want {
		t.Errorf("the options table has %d rows, and the registry plus the daemon "+
			"options make %d", len(rows), want)
	}
}

// TestThePluginOptionTableDriftBites proves the comparison above can fail. A
// drift test that cannot fail is a comment: the hand-written table it replaced
// was wrong for a release while a Contains() check passed.
func TestThePluginOptionTableDriftBites(t *testing.T) {
	doc := readPluginReadme(t)
	s := mustSchema(t)
	want, err := pluginReadmeWithOptions(doc, s)
	if err != nil {
		t.Fatal(err)
	}
	if want != doc {
		t.Fatal("the committed README is already stale; regenerate it before " +
			"believing anything below")
	}
	mutations := []struct {
		what string
		do   func(string) string
	}{
		{"a changed default", func(d string) string {
			return strings.Replace(d, "| `mcpx` |", "| `mcpx-next` |", 1)
		}},
		{"a changed variable", func(d string) string {
			return strings.Replace(d, "`MCPX_PLUGIN_BIN`", "`MCPX_PLUGIN_BINARY`", 1)
		}},
		{"a renamed row", func(d string) string {
			return strings.Replace(d, "| `remember` |", "| `remembered` |", 1)
		}},
		{"an invented row", func(d string) string {
			return strings.Replace(d, "| `bin` |",
				"| `nope` | `MCPX_PLUGIN_NOPE` | — | invented. |\n| `bin` |", 1)
		}},
	}
	for _, m := range mutations {
		mutated := m.do(want)
		if mutated == want {
			t.Fatalf("%s: the mutation changed nothing, so the table no longer "+
				"contains what the mutation targets", m.what)
		}
		got, err := pluginReadmeWithOptions(mutated, s)
		if err != nil {
			t.Fatal(err)
		}
		if got == mutated {
			t.Errorf("%s went unnoticed by the drift check", m.what)
		}
	}
}

func readPluginReadme(t *testing.T) string {
	t.Helper()
	b, err := os.ReadFile(pluginReadmePath)
	if err != nil {
		t.Fatalf("the plugin README should exist: %v", err)
	}
	return string(b)
}
