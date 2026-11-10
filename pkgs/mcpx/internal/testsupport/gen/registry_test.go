package gen_test

import (
	"bytes"
	"crypto/sha256"
	"encoding/hex"
	"fmt"
	"os"
	"path/filepath"
	"sort"
	"strings"
	"testing"

	"github.com/developing-today/code/pkgs/mcpx/internal/config"
	"github.com/developing-today/code/pkgs/mcpx/internal/launcher"
	"github.com/developing-today/code/pkgs/mcpx/internal/testsupport/gen"
)

// TestRegistryWellFormed: All is the one registration table, and
// every entry in it is complete and deterministic.
func TestRegistryWellFormed(t *testing.T) {
	seen := map[string]bool{}
	prev := ""
	for _, g := range gen.All {
		if g.Name == "" || g.Version < 1 || g.Origin == "" || g.Draw == nil {
			t.Fatalf("incomplete registry entry: %+v", g)
		}
		if seen[g.Key()] {
			t.Fatalf("duplicate registry key %s", g.Key())
		}
		seen[g.Key()] = true
		if prev != "" && g.Name <= prev {
			t.Fatalf("gen.All not sorted by name: %s after %s", g.Name, prev)
		}
		prev = g.Name
		a := g.Draw(gen.Stream(goldenSeed, "wellformed", g.Name, g.Version), 3)
		b := g.Draw(gen.Stream(goldenSeed, "wellformed", g.Name, g.Version), 3)
		if a != b || a == "" {
			t.Fatalf("%s: Draw not deterministic/non-empty: %q vs %q", g.Key(), a, b)
		}
		if _, ok := gen.Lookup(g.Name); !ok {
			t.Fatalf("%s: Lookup misses a registered generator", g.Name)
		}
	}
}

// TestRegistryCoversTypedGenerators is the unregistered-generator
// guard for the generators that exist as typed code: the typed
// draw functions in generators.go are exactly the registered set.
// A new typed generator without a registry entry fails here; a
// registry entry with no typed draw fails TestRegistryWellFormed's
// Draw check or the golden guard. (The four remaining #1958 first
// shapes are follow-on work, registered when they land.)
func TestRegistryCoversTypedGenerators(t *testing.T) {
	want := []string{"config.layers", "launcher.template"}
	var got []string
	for _, g := range gen.All {
		got = append(got, g.Name)
	}
	sort.Strings(got)
	if fmt.Sprint(got) != fmt.Sprint(want) {
		t.Fatalf("registered generators = %v, want %v (update this guard and gen.All together)", got, want)
	}
}

// TestManifestHash: the manifest changes iff a name or version
// changes, and does not depend on entry order.
func TestManifestHash(t *testing.T) {
	base := gen.ManifestHash(gen.All)
	if len(base) != 64 {
		t.Fatalf("manifest %q is not a sha256 hex", base)
	}
	if gen.Manifest() != base {
		t.Fatal("Manifest() != ManifestHash(All)")
	}
	reversed := append([]gen.Generator{}, gen.All...)
	for i, j := 0, len(reversed)-1; i < j; i, j = i+1, j-1 {
		reversed[i], reversed[j] = reversed[j], reversed[i]
	}
	if gen.ManifestHash(reversed) != base {
		t.Fatal("manifest depends on entry order")
	}
	bumped := append([]gen.Generator{}, gen.All...)
	bumped[0].Version++
	if gen.ManifestHash(bumped) == base {
		t.Fatal("manifest did not change on a version bump")
	}
	renamed := append([]gen.Generator{}, gen.All...)
	renamed[0].Name += "x"
	if gen.ManifestHash(renamed) == base {
		t.Fatal("manifest did not change on a rename")
	}
}

// TestParseBudget: the MCPX_TEST_FUZZ knob parses exactly the
// four named budgets, fails loudly on anything else, and counts
// grow with the budget.
func TestParseBudget(t *testing.T) {
	for _, s := range []string{"off", "quick", "ci", "soak", " CI "} {
		if _, err := gen.ParseBudget(s); err != nil {
			t.Fatalf("ParseBudget(%q): %v", s, err)
		}
	}
	for _, s := range []string{"", "fast", "1", "quiick"} {
		if _, err := gen.ParseBudget(s); err == nil {
			t.Fatalf("ParseBudget(%q) succeeded, want loud failure", s)
		}
	}
	if gen.BudgetOff.PureCount() > gen.BudgetQuick.PureCount() ||
		gen.BudgetQuick.PureCount() > gen.BudgetCI.PureCount() ||
		gen.BudgetCI.PureCount() > gen.BudgetSoak.PureCount() {
		t.Fatal("pure counts do not grow with budget")
	}
	if gen.BudgetQuick.E2ECount() >= gen.BudgetQuick.PureCount() {
		t.Fatal("e2e count should stay far below pure count (each case costs a subprocess)")
	}
}

// goldenPins pins each registered generator's canonical Draw at a
// fixed seed and size: the golden-output guard over gen.All
// (#1958, following #1949 section 3). If a pin fails, the
// generator's output changed — that is a new Version: bump it,
// re-pin under the new version, and keep the old stream
// reproducible. Never just re-pin at the same version.
var goldenPins = map[string]string{
	"config.layers@1":     "73fbc0d18e2d95cb8dd8d06133d37aa037483f058c8b0cba46760ac26f98310e",
	"launcher.template@1": "18cba9a2a651ce69a7de19d035abbb9c97f40f219f400421c296a6513af69998",
}

func TestGoldenGuardCoversAll(t *testing.T) {
	if len(goldenPins) != len(gen.All) {
		t.Fatalf("golden pins cover %d generators, gen.All has %d — a registered generator has no golden pin", len(goldenPins), len(gen.All))
	}
	for _, g := range gen.All {
		pin, ok := goldenPins[g.Key()]
		if !ok {
			t.Fatalf("%s: no golden pin", g.Key())
		}
		out := g.Draw(gen.Stream(goldenSeed, "golden-guard", g.Name, g.Version), 3)
		sum := sha256.Sum256([]byte(out))
		if got := hex.EncodeToString(sum[:]); got != pin {
			t.Fatalf("%s: golden output changed: sha256 %s, pinned %s — a changed output is a new generator version, not an edit (draw was %s)", g.Key(), got, pin, out)
		}
	}
}

func budgetCount(t *testing.T) int {
	t.Helper()
	b, err := gen.BudgetFromEnv()
	if err != nil {
		t.Fatal(err)
	}
	return b.PureCount()
}

// TestConfigLayersProperty: nearest wins per server; adding an
// unrelated far layer never changes a near value; StripJSONC of
// comment-free JSON is identity. Draws at the MCPX_TEST_FUZZ
// budget count from the run-seed stream.
func TestConfigLayersProperty(t *testing.T) {
	count := budgetCount(t)
	src := gen.Stream(goldenSeed, t.Name(), "config.layers", 1)
	for i := 0; i < count; i++ {
		stack := gen.DrawConfigLayers(src, 3)
		// StripJSONC identity on every comment-free rendering.
		for _, text := range stack.Render(false) {
			if got := config.StripJSONC([]byte(text)); !bytes.Equal(got, []byte(text)) {
				t.Fatalf("case %d: StripJSONC not identity on comment-free JSON: %q -> %q", i, text, got)
			}
		}
		got := loadStack(t, stack)
		assertWinners(t, i, stack, got)
		// An unrelated far layer changes nothing near.
		extra := gen.ConfigLayers{Layers: append(
			[]gen.ConfigLayer{{Servers: []gen.LayerServer{{Name: "omega", Command: "cmd-omega"}}}},
			stack.Layers...)}
		got2 := loadStack(t, extra)
		for _, name := range stack.Names() {
			want, _ := stack.Winner(name)
			if got2[name] != want {
				t.Fatalf("case %d: unrelated far layer changed near value: server %s = %q, want %q", i, name, got2[name], want)
			}
		}
		if s, ok := got2["omega"]; !ok || s != "cmd-omega" {
			t.Fatalf("case %d: unrelated far server omega = %q, %v", i, s, ok)
		}
	}
}

// loadStack writes a drawn stack as nested .mcpx.json layers
// (farthest outermost) under an isolated HOME and loads exactly
// those files.
//
// Hermeticity (#7686): the load must never consult the ambient
// machine. Setting HOME/XDG_CONFIG_HOME alone is NOT enough:
// config.SearchPathFrom walks up from the working directory to
// the filesystem root adding config spellings at every ancestor,
// so when TMPDIR lives under the real home (the fleet's Go
// convention, TMPDIR=~/tmp), t.TempDir() is under the real home
// too and the walk picks up the host's real
// ~/.config/mcpx/config.json on its way past — the merged result
// then contains every server the host happens to define.
// Instead the fixture files are passed as the paths.config seed:
// a seed with no null splice replaces the default walk outright
// (config.ResolveChainFrom pass 1), so the chain is exactly the
// fixture layers, nearest first, on any machine.
func loadStack(t *testing.T, stack gen.ConfigLayers) map[string]string {
	return loadStackSeeded(t, stack, false)
}

// loadStackSeeded is loadStack with an optional hostile host:
// hostile plants host-style configs where a non-hermetic load
// would find them — the isolated HOME's user config (the home
// fallback) and an ancestor directory of the stack (the upward
// walk) — each naming a server that is not in the stack.
func loadStackSeeded(t *testing.T, stack gen.ConfigLayers, hostile bool) map[string]string {
	t.Helper()
	root := t.TempDir()
	home := filepath.Join(root, "home")
	if err := os.MkdirAll(home, 0o755); err != nil {
		t.Fatal(err)
	}
	t.Setenv("HOME", home)
	t.Setenv("XDG_CONFIG_HOME", filepath.Join(root, "xdg"))
	t.Setenv("MCPX_CONFIG", "")
	t.Setenv("MCPX_PATHS_CONFIG", "")
	config.SetPathsConfigSeed(nil)
	t.Cleanup(func() { config.SetPathsConfigSeed(nil) })

	texts := stack.Render(true)
	dir := filepath.Join(root, "l0")
	var files []string // farthest first, in write order
	for li, text := range texts {
		if li > 0 {
			dir = filepath.Join(dir, fmt.Sprintf("l%d", li))
		}
		if err := os.MkdirAll(dir, 0o755); err != nil {
			t.Fatal(err)
		}
		f := filepath.Join(dir, ".mcpx.json")
		if err := os.WriteFile(f, []byte(text), 0o644); err != nil {
			t.Fatal(err)
		}
		files = append(files, f)
	}
	if hostile {
		hostileCfg := `{"mcpServers":{"host-extra":{"command":"npx"}}}`
		homeCfg := filepath.Join(home, ".config", "mcpx", "config.json")
		if err := os.MkdirAll(filepath.Dir(homeCfg), 0o755); err != nil {
			t.Fatal(err)
		}
		if err := os.WriteFile(homeCfg, []byte(hostileCfg), 0o644); err != nil {
			t.Fatal(err)
		}
		ancestorCfg := filepath.Join(root, ".config", "mcpx", "config.json")
		if err := os.MkdirAll(filepath.Dir(ancestorCfg), 0o755); err != nil {
			t.Fatal(err)
		}
		if err := os.WriteFile(ancestorCfg, []byte(hostileCfg), 0o644); err != nil {
			t.Fatal(err)
		}
	}
	// Seed the chain with exactly the fixture files, nearest
	// (deepest) first: a no-null seed replaces the default
	// ancestor/home walk outright, so no host file can enter.
	seed := make([]string, 0, len(files))
	for i := len(files) - 1; i >= 0; i-- {
		seed = append(seed, files[i])
	}
	config.SetPathsConfigSeed(seed)
	cfg, err := config.LoadFrom("", dir)
	if err != nil {
		t.Fatalf("LoadFrom: %v (stack %s)", err, stack.Canonical())
	}
	out := map[string]string{}
	for name, s := range cfg.MCPServers {
		out[name] = s.Command
	}
	return out
}

// TestConfigLayersHostIsolation is the #7686 regression guard:
// with host-style configs planted in the isolated HOME and in an
// ancestor of the stack directory, the merged servers are still
// exactly the fixture stack's — the load exercises only its
// fixture layers, never live host configuration. (On the laptop
// this also covers the original failure shape, TMPDIR under the
// real home; the planted ancestor config reproduces that leak
// deterministically on any machine.)
func TestConfigLayersHostIsolation(t *testing.T) {
	stack := gen.ConfigLayers{Layers: []gen.ConfigLayer{
		{Servers: []gen.LayerServer{{Name: "alpha", Command: "cmd-alpha"}, {Name: "beta", Command: "cmd-beta"}}},
		{Servers: []gen.LayerServer{{Name: "beta", Command: "cmd-beta-near"}}},
	}}
	got := loadStackSeeded(t, stack, true)
	assertWinners(t, 0, stack, got)
	if _, ok := got["host-extra"]; ok {
		t.Fatalf("host config leaked into the merged stack: %v", got)
	}
}

func assertWinners(t *testing.T, i int, stack gen.ConfigLayers, got map[string]string) {
	t.Helper()
	names := stack.Names()
	if len(got) != len(names) {
		t.Fatalf("case %d: merged servers = %v, want names %v (stack %s)", i, got, names, stack.Canonical())
	}
	for _, name := range names {
		want, _ := stack.Winner(name)
		if got[name] != want {
			t.Fatalf("case %d: server %s = %q, want nearest %q (stack %s)", i, name, got[name], want, stack.Canonical())
		}
	}
}

// TestLauncherTemplateProperty: filling every Used placeholder
// never errors, no placeholder survives, and the output is
// exactly the literals with fills substituted in place (text
// outside placeholders preserved). Draws at budget count.
func TestLauncherTemplateProperty(t *testing.T) {
	count := budgetCount(t)
	src := gen.Stream(goldenSeed, t.Name(), "launcher.template", 1)
	for i := 0; i < count; i++ {
		draw := gen.DrawLauncherTemplate(src, 3)
		tmpl := launcher.Template{Text: draw.Text(), Name: "generated"}
		used := launcher.Used(tmpl)
		var wantUsed []string
		for j, s := range draw.Segments {
			if j%2 == 1 {
				wantUsed = append(wantUsed, s)
			}
		}
		sort.Strings(wantUsed)
		var gotUsed []string
		for _, p := range used {
			gotUsed = append(gotUsed, string(p))
		}
		if fmt.Sprint(gotUsed) != fmt.Sprint(wantUsed) {
			t.Fatalf("case %d: Used = %v, want %v (template %q)", i, gotUsed, wantUsed, draw.Text())
		}
		fill := launcher.Fill{}
		for name, v := range draw.Fill {
			fill[launcher.Placeholder(name)] = v
		}
		out, err := launcher.Expand(tmpl, fill, launcher.Options{})
		if err != nil {
			t.Fatalf("case %d: Expand with every Used placeholder filled errored: %v (template %q)", i, err, draw.Text())
		}
		if out != draw.Expected() {
			t.Fatalf("case %d: Expand = %q, want %q (literals preserved, fills in place)", i, out, draw.Expected())
		}
		for _, name := range wantUsed {
			if strings.Contains(out, "@"+name) {
				t.Fatalf("case %d: placeholder @%s survived expansion: %q", i, name, out)
			}
		}
	}
}
