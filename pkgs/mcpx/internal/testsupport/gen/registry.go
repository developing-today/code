package gen

import (
	"crypto/sha256"
	"encoding/hex"
	"fmt"
	"os"
	"sort"
	"strings"
)

// This file is the registry half of #1958 (Approach C): every
// generator mcpx ships is one entry in All, so guards can iterate
// the whole set — a generator that is not in All does not exist as
// far as the golden guard, the manifest in stored seed records
// (#122), and the budget knob are concerned. Generators register
// here and ONLY here; the typed draw functions in generators.go
// are each named by exactly one entry (TestRegistryCoversTypedGenerators
// pins that correspondence).

// Generator is one registered bug-shape generator.
//
// Draw renders one generated input in canonical form (a string) at
// the given size. The canonical form is what the golden-output
// guard pins: same stream, same size, same string, forever — a
// change to it is a new Version, never a silent edit, exactly like
// the stream-stability rule in gen.go. Typed views of the same draw
// (for property tests that need structure, not text) live beside
// the entry and must render to the same canonical form.
type Generator struct {
	Name    string // <area>.<shape>, e.g. "config.layers"
	Version int    // bump on ANY change to Draw's output for the same stream
	Origin  string // the issue/PR/bug that motivated the shape ("#5, #78")
	Draw    func(src Source, size int) string
}

// Key is the generator's substream key: name@version (#1949
// section 2). Bumping one generator's version never shifts
// another's inputs, because the key is per-generator.
func (g Generator) Key() string { return fmt.Sprintf("%s@%d", g.Name, g.Version) }

// All is the registry: every generator, sorted by name. The four
// remaining first shapes from #1958 (exec.flagset,
// codegen.jsonschema, api.v1request, settings.precedence) are
// named follow-on work on #1958 — not registered yet, not dropped.
var All = []Generator{
	{
		Name:    "config.layers",
		Version: 1,
		Origin:  "#5, #78 (via #1958)",
		Draw: func(src Source, size int) string {
			return DrawConfigLayers(src, size).Canonical()
		},
	},
	{
		Name:    "launcher.template",
		Version: 1,
		Origin:  "#7 (via #1958)",
		Draw: func(src Source, size int) string {
			return DrawLauncherTemplate(src, size).Canonical()
		},
	},
}

// Lookup returns the registered generator with the given name.
func Lookup(name string) (Generator, bool) {
	for _, g := range All {
		if g.Name == name {
			return g, true
		}
	}
	return Generator{}, false
}

// ManifestHash is the sha256 over the sorted name@version entries
// of a registry — the manifest that goes into every stored seed
// record (#122), so a stored seed names exactly which generator
// versions produced its inputs. The hash changes iff a name or a
// version in the set changes; order of the input slice does not
// matter.
func ManifestHash(gs []Generator) string {
	keys := make([]string, 0, len(gs))
	for _, g := range gs {
		keys = append(keys, g.Key())
	}
	sort.Strings(keys)
	h := sha256.New()
	for _, k := range keys {
		h.Write([]byte(k))
		h.Write([]byte("\n"))
	}
	return hex.EncodeToString(h.Sum(nil))
}

// Manifest is ManifestHash over All.
func Manifest() string { return ManifestHash(All) }

// Budget is the MCPX_TEST_FUZZ knob (#1958, named in #1949):
// off|quick|ci|soak. Budgets are COUNTS, not time, so the same
// seed generates the same inputs on every machine.
type Budget string

const (
	BudgetOff   Budget = "off"   // regression inputs only (nix checkPhase)
	BudgetQuick Budget = "quick" // local default
	BudgetCI    Budget = "ci"    // matrix default
	BudgetSoak  Budget = "soak"  // scheduled long run
)

// ParseBudget parses one MCPX_TEST_FUZZ value. An invalid value
// fails loudly: a typo that silently fell back to a default would
// change which inputs a run generates while looking like the
// requested run.
func ParseBudget(s string) (Budget, error) {
	switch Budget(strings.ToLower(strings.TrimSpace(s))) {
	case BudgetOff, BudgetQuick, BudgetCI, BudgetSoak:
		return Budget(strings.ToLower(strings.TrimSpace(s))), nil
	default:
		return "", fmt.Errorf("MCPX_TEST_FUZZ: invalid value %q: want off, quick, ci or soak", s)
	}
}

// BudgetFromEnv reads MCPX_TEST_FUZZ; unset means the local
// default, quick.
func BudgetFromEnv() (Budget, error) {
	s, ok := os.LookupEnv("MCPX_TEST_FUZZ")
	if !ok || strings.TrimSpace(s) == "" {
		return BudgetQuick, nil
	}
	return ParseBudget(s)
}

// PureCount is the per-generator draw count for pure-function
// generators at this budget (the starting counts #1958 proposes;
// #95 owns the final numbers).
func (b Budget) PureCount() int {
	switch b {
	case BudgetOff:
		return 1 // the fixed-seed regression case only
	case BudgetCI:
		return 2000
	case BudgetSoak:
		return 20000
	default: // quick
		return 200
	}
}

// E2ECount is the per-generator draw count for generators whose
// cases cost a subprocess.
func (b Budget) E2ECount() int {
	switch b {
	case BudgetOff:
		return 1
	case BudgetCI:
		return 10
	case BudgetSoak:
		return 100
	default: // quick
		return 2
	}
}
