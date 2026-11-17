package config

import (
	"encoding/json"
	"fmt"
	"os"
	"path/filepath"
	"strings"
	"sync"

	"github.com/developing-today/code/pkgs/mcpx/internal/settings"
)

// Two-pass resolution of the config chain (issue #1842, Approach C).
//
// paths.config is declared in the settings registry, but a resolved Set
// cannot decide which files the Set itself is read from — that is the
// circularity that left the setting inert. The chain is therefore resolved
// here, in two passes with the boundary stated rather than implied:
//
//  1. Discovery pass. Walk the default locations (SearchPathFrom), seeded
//     by MCPX_PATHS_CONFIG / --paths-config. Read only paths.config from
//     whatever is found, honouring the null splice.
//  2. Load pass. Resolve that list and load everything on it, nearest
//     last (LoadFrom merges nearest-first, which is the same order seen
//     from the other end).
//
// A file's paths.config governs the rest of the chain from that point —
// the locations farther than the file that named them — not the search
// that found the file itself. A list with no null entry replaces the
// farther remainder outright; a null entry stands for that remainder, so
// entries after the null land farther than everything already there.
//
// Rules pinned by the issue:
//   - A paths.config found in pass 2 that differs from pass 1 is
//     reported (ChainInfo.Divergent), not silently applied. Applying it
//     would mean re-running discovery with different roots, which need
//     not terminate.
//   - A cycle (A adds B, B adds A) is refused by identity, naming the
//     path that closed it.
//   - MCPX_CONFIG bypasses both passes: "this file, nothing else" has
//     to stay expressible.

// ChainInfo describes one two-pass resolution, so callers (notably
// `mcpx config --sources`) can show the passes distinctly instead of
// presenting one merged list as if it had been searched directly.
type ChainInfo struct {
	// Bypass is true when MCPX_CONFIG short-circuited both passes.
	Bypass bool
	// Discovery lists the pass-1 locations, nearest first: the seed
	// expansions spliced around the default walk at the seed's null.
	Discovery []string
	// Load lists the pass-2 locations, nearest first — the chain
	// LoadFrom actually merges.
	Load []string
	// Seeded holds the raw seed entries (environment/flag), if any.
	Seeded []string
	// Divergent reports paths.config declarations found in pass 2 that
	// were not applied: files the load pass added declaring entries of
	// their own, and declarations dropped when a nearer list replaced
	// the rest of the chain.
	Divergent []string
}

var (
	seedMu   sync.Mutex
	flagSeed []string
)

// SetPathsConfigSeed records the --paths-config flag value (a global
// flag, consumed in main alongside --config) as the pass-1 seed, in
// place of MCPX_PATHS_CONFIG. The flag has to be captured before any
// settings exist, for the same reason --config is: it decides which
// files the settings are read from.
func SetPathsConfigSeed(entries []string) {
	seedMu.Lock()
	defer seedMu.Unlock()
	flagSeed = append([]string(nil), entries...)
}

// AddPathsConfigSeed appends one --paths-config occurrence to the
// pass-1 seed. The setting is Repeatable, and the normal binding
// accumulates repeated occurrences; the bootstrap flag must do the
// same (PR #2477 finding) — replacing on each call left only the
// last value in discovery. main calls this once per occurrence;
// SetPathsConfigSeed remains the replace/reset form.
func AddPathsConfigSeed(entries []string) {
	seedMu.Lock()
	defer seedMu.Unlock()
	for _, e := range entries {
		// On the command line the splice marker is spelled "-"
		// (the settings schema's contract for a repeatable flag,
		// and docs/proposals.md); decode it to the marker the seed
		// list itself carries, or it would be read as a directory
		// named "-" and the seed would silently lose its splice.
		if e == "-" {
			e = settings.NullMarker
		}
		flagSeed = append(flagSeed, e)
	}
}

// PathsConfigSeedEnv returns the flag seed in MCPX_PATHS_CONFIG form
// (a JSON array, with null for splice markers), so a caller starting
// a child process — an auto-started daemon — can forward it (PR
// #2477 finding): the flag seed otherwise lives only in this
// process's memory, and the child would resolve a different chain.
// ok is false when no flag seed was given; an environment seed
// already travels in the environment itself.
func PathsConfigSeedEnv() (string, bool) {
	seedMu.Lock()
	fs := append([]string(nil), flagSeed...)
	seedMu.Unlock()
	if len(fs) == 0 {
		return "", false
	}
	arr := make([]any, len(fs))
	for i, e := range fs {
		if e == settings.NullMarker {
			arr[i] = nil
		} else {
			arr[i] = e
		}
	}
	b, err := json.Marshal(arr)
	if err != nil {
		return "", false
	}
	return string(b), true
}

// pathsConfigSeed returns the pass-1 seed entries: the flag when given,
// otherwise MCPX_PATHS_CONFIG. Both spell the same list a config file
// holds — a JSON array, or entries split on commas and newlines — with
// null entries preserved as splice markers.
func pathsConfigSeed() []string {
	seedMu.Lock()
	fs := append([]string(nil), flagSeed...)
	seedMu.Unlock()
	if len(fs) > 0 {
		return fs
	}
	v := strings.TrimSpace(os.Getenv("MCPX_PATHS_CONFIG"))
	if v == "" {
		return nil
	}
	if strings.HasPrefix(v, "[") {
		var arr []any
		if json.Unmarshal([]byte(v), &arr) == nil {
			out := make([]string, 0, len(arr))
			for _, e := range arr {
				if e == nil {
					out = append(out, settings.NullMarker)
					continue
				}
				out = append(out, fmt.Sprint(e))
			}
			return out
		}
	}
	fields := strings.FieldsFunc(v, func(r rune) bool { return r == ',' || r == '\n' })
	out := make([]string, 0, len(fields))
	for _, f := range fields {
		if f = strings.TrimSpace(f); f != "" {
			out = append(out, f)
		}
	}
	return out
}

// configSpellings are the candidate files one directory level
// contributes to the chain, in the order SearchPathFrom tries them.
func configSpellings(dir string) []string {
	return []string{
		filepath.Join(dir, ".config", "mcpx", "config.json"),
		filepath.Join(dir, ".mcpx.json"),
		filepath.Join(dir, ".mcpx", "config.json"),
		filepath.Join(dir, ".mcp.json"),
	}
}

// expandEntry resolves one paths.config entry to candidate chain
// locations. An entry naming an existing file means exactly that file
// (the registry's spelling); anything else is a directory, searched
// with the usual per-level spellings. Relative entries resolve against
// base — the declaring file's directory, or the working directory for
// the seed.
func expandEntry(entry, base string) []string {
	p := entry
	if !filepath.IsAbs(p) {
		p = filepath.Join(base, p)
	}
	p = filepath.Clean(p)
	if st, err := os.Stat(p); err == nil && !st.IsDir() {
		return []string{p}
	}
	return configSpellings(p)
}

// expandEntries expands a raw entry list, splitting at the null splice
// marker: entries before it land nearer than the inherited remainder,
// entries after it farther.
func expandEntries(entries []string, base string) (before, after []string, hasNull bool) {
	dst := &before
	for _, e := range entries {
		if e == settings.NullMarker {
			hasNull = true
			dst = &after
			continue
		}
		*dst = append(*dst, expandEntry(e, base)...)
	}
	return before, after, hasNull
}

func expandAll(entries []string, base string) []string {
	before, after, _ := expandEntries(entries, base)
	return append(before, after...)
}

// readPathsConfig reads only the paths.config key from one file, as raw
// entries with nulls preserved. ok is false when the file is absent or
// declares no paths.config.
func readPathsConfig(path string) (entries []string, ok bool, err error) {
	b, rerr := os.ReadFile(path)
	if rerr != nil {
		if os.IsNotExist(rerr) {
			return nil, false, nil
		}
		return nil, false, rerr
	}
	var doc struct {
		Paths struct {
			Config any `json:"config"`
		} `json:"paths"`
		Flat any `json:"paths.config"`
	}
	if uerr := json.Unmarshal(StripJSONC(b), &doc); uerr != nil {
		return nil, false, fmt.Errorf("reading paths.config in %s: %w", path, uerr)
	}
	raw := doc.Paths.Config
	if raw == nil {
		raw = doc.Flat
	}
	if raw == nil {
		return nil, false, nil
	}
	// A string value is the comma/newline list spelling the settings
	// layer itself accepts for this key (ApplyFile / Set.List), and
	// existing configs use it. Discovery runs before dispatch, so
	// refusing it here would make a previously valid file block every
	// command (PR #2477 finding); accept it like the loader does.
	if str, isStr := raw.(string); isStr {
		out := make([]string, 0, 4)
		for _, f := range strings.FieldsFunc(str, func(r rune) bool { return r == ',' || r == '\n' }) {
			if f = strings.TrimSpace(f); f != "" {
				out = append(out, f)
			}
		}
		return out, true, nil
	}
	arr, isArr := raw.([]any)
	if !isArr {
		return nil, false, fmt.Errorf("reading paths.config in %s: paths.config is not a list", path)
	}
	out := make([]string, 0, len(arr))
	for _, e := range arr {
		if e == nil {
			out = append(out, settings.NullMarker)
			continue
		}
		s, isStr := e.(string)
		if !isStr {
			return nil, false, fmt.Errorf("reading paths.config in %s: entry %v is not a path string", path, e)
		}
		out = append(out, s)
	}
	return out, true, nil
}

func chainIdentity(p string) string {
	abs, err := filepath.Abs(p)
	if err != nil {
		abs = p
	}
	if resolved, err := filepath.EvalSymlinks(abs); err == nil {
		abs = resolved
	}
	return filepath.Clean(abs)
}

// findCycle looks for a cycle in the pass-1 "adds" graph (file ->
// chain files its paths.config expands to). It returns the file whose
// declaration closed the cycle and the cycle itself.
func findCycle(edges map[string][]string, order []string) (string, []string, bool) {
	const (
		white = 0
		grey  = 1
		black = 2
	)
	colour := map[string]int{}
	var stack []string
	var visit func(n string) ([]string, bool)
	visit = func(n string) ([]string, bool) {
		colour[n] = grey
		stack = append(stack, n)
		for _, m := range edges[n] {
			switch colour[m] {
			case grey:
				i := 0
				for j, s := range stack {
					if s == m {
						i = j
						break
					}
				}
				return append(append([]string{}, stack[i:]...), m), true
			case white:
				if cyc, found := visit(m); found {
					return cyc, true
				}
			}
		}
		stack = stack[:len(stack)-1]
		colour[n] = black
		return nil, false
	}
	for _, n := range order {
		if _, isNode := edges[n]; !isNode {
			continue
		}
		if colour[n] == white {
			if cyc, found := visit(n); found {
				// The closing declaration belongs to the file whose
				// edge led back into the stack: the predecessor of
				// the repeated node in the reported cycle.
				closing := cyc[len(cyc)-2]
				return closing, cyc, true
			}
		}
	}
	return "", nil, false
}

// ResolveChain runs both passes for the process's working directory.
func ResolveChain() ([]string, *ChainInfo, error) {
	wd, err := os.Getwd()
	if err != nil {
		wd = ""
	}
	return ResolveChainFrom(wd)
}

// ResolveChainFrom is ResolveChain for a directory other than this
// process's, matching SearchPathFrom.
func ResolveChainFrom(wd string) ([]string, *ChainInfo, error) {
	// MCPX_CONFIG wins outright and skips both passes.
	if p := os.Getenv("MCPX_CONFIG"); p != "" {
		one := []string{p}
		return one, &ChainInfo{Bypass: true, Discovery: one, Load: one}, nil
	}

	// Pass 1: the seed expands first (nearest), then the default walk.
	info := &ChainInfo{Seeded: pathsConfigSeed()}
	var chain0 []string
	seen0 := map[string]bool{}
	add0 := func(p string) {
		if id := chainIdentity(p); !seen0[id] {
			seen0[id] = true
			chain0 = append(chain0, p)
		}
	}
	// The seed obeys the same splice rule as a file's list (PR #2477
	// finding): entries before a null land nearer than the default
	// walk, entries after it land farther, and a seed with no null
	// replaces the default walk outright — which is what the
	// registry help for the setting promises ("Without a null the
	// list replaces outright"). No seed at all leaves the walk
	// untouched.
	if len(info.Seeded) > 0 {
		before, after, hasNull := expandEntries(info.Seeded, wd)
		for _, loc := range before {
			add0(loc)
		}
		if hasNull {
			for _, loc := range SearchPathFrom(wd) {
				add0(loc)
			}
		}
		for _, loc := range after {
			add0(loc)
		}
	} else {
		for _, loc := range SearchPathFrom(wd) {
			add0(loc)
		}
	}
	info.Discovery = chain0

	inChain0 := map[string]bool{}
	for _, p := range chain0 {
		inChain0[chainIdentity(p)] = true
	}

	// Discovery reads: only paths.config, from files that exist.
	declared := map[string][]string{}
	// The same declarations keyed by file identity, with the
	// discovery spelling kept for relative-path resolution: the
	// load chain may retain a different spelling of the same file
	// (a symlink alias a nearer list inserted), and the declaration
	// must still be found (PR #5165 review).
	declaredByID := map[string][]string{}
	declaredFrom := map[string]string{}
	for _, f := range chain0 {
		entries, ok, err := readPathsConfig(f)
		if err != nil {
			return nil, nil, err
		}
		if ok {
			declared[f] = entries
			id := chainIdentity(f)
			declaredByID[id] = entries
			declaredFrom[id] = f
		}
	}

	// A cycle among the discovered files is refused before any chain
	// is built from it: A adds B and B adds A has no honest order.
	// Graph endpoints are file identities, not raw spellings (PR
	// #2477 finding): the membership test above is by identity, so
	// the graph must be too — two spellings of one file (a symlink
	// alias) would otherwise be unrelated vertices, and a real cycle
	// through an alias would slip past.
	edges := map[string][]string{}
	for f, entries := range declared {
		from := chainIdentity(f)
		for _, loc := range expandAll(entries, filepath.Dir(f)) {
			// A file naming its own location is idempotent, not a
			// cycle: it is already in the chain at that point.
			to := chainIdentity(loc)
			if to == from {
				continue
			}
			if inChain0[to] {
				edges[from] = append(edges[from], to)
			}
		}
	}
	order := make([]string, 0, len(chain0))
	for _, p := range chain0 {
		order = append(order, chainIdentity(p))
	}
	if closing, cyc, found := findCycle(edges, order); found {
		return nil, nil, fmt.Errorf("paths.config cycle refused: %s closes the cycle (%s)",
			closing, strings.Join(cyc, " -> "))
	}

	// Build the load chain nearest-first. Each discovered file's list
	// governs the chain farther than itself: its locations are
	// inserted immediately after it (before the null) and after the
	// inherited remainder (after the null), or replace that remainder
	// outright when the list has no null.
	load := append([]string{}, chain0...)
	applied := map[string]bool{} // by file identity, like declaredByID
	indexOf := func(p string) int {
		id := chainIdentity(p)
		for i, q := range load {
			if chainIdentity(q) == id {
				return i
			}
		}
		return -1
	}
	// Declarations apply in the resolved chain's order, not in
	// discovery order (PR #2477 finding): when a nearer file's list
	// reorders the chain, the files it moved nearer govern before
	// the ones they displaced — applying in discovery order let a
	// displaced file's no-null list replace a remainder the new
	// order had already extended. Each application can reorder the
	// remainder, so rescan from the nearest end after each one. A
	// file a nearer replacement dropped from the chain is never
	// reached here; pass 2 reports it as divergent.
	for {
		progressed := false
		for _, f := range load {
			id := chainIdentity(f)
			entries, ok := declaredByID[id]
			if !ok || applied[id] {
				continue
			}
			idx := indexOf(f)
			if idx < 0 {
				continue
			}
			before, after, hasNull := expandEntries(entries, filepath.Dir(declaredFrom[id]))
			next := make([]string, 0, len(load)+len(before)+len(after))
			next = append(next, load[:idx+1]...)
			next = append(next, before...)
			if hasNull {
				next = append(next, load[idx+1:]...)
				next = append(next, after...)
			}
			// Dedupe by identity, first (nearest) occurrence wins — the
			// same rule SearchPathFrom applies to the default walk.
			deduped := next[:0]
			seen := map[string]bool{}
			for _, p := range next {
				if id := chainIdentity(p); !seen[id] {
					seen[id] = true
					deduped = append(deduped, p)
				}
			}
			load = deduped
			applied[id] = true
			progressed = true
			break
		}
		if !progressed {
			break
		}
	}
	info.Load = load

	// Pass 2: re-read the load chain. A declaration that pass 1 did not
	// apply — from a file pass 1 added, or one a nearer replacement
	// dropped — is reported, never applied.
	inLoad := map[string]bool{}
	for _, p := range load {
		inLoad[chainIdentity(p)] = true
	}
	for _, f := range load {
		if applied[chainIdentity(f)] {
			continue
		}
		entries, ok, err := readPathsConfig(f)
		if err != nil {
			return nil, nil, err
		}
		if !ok {
			continue
		}
		var extra []string
		for _, loc := range expandAll(entries, filepath.Dir(f)) {
			if !inLoad[chainIdentity(loc)] {
				extra = append(extra, loc)
			}
		}
		if len(extra) > 0 {
			info.Divergent = append(info.Divergent, fmt.Sprintf(
				"%s declares paths.config adding %s — reported, not applied (pass 2)",
				f, strings.Join(extra, ", ")))
		}
	}
	for f := range declared {
		if !applied[chainIdentity(f)] && indexOf(f) < 0 {
			info.Divergent = append(info.Divergent, fmt.Sprintf(
				"%s declares paths.config but a nearer file's list replaced the rest of the chain — reported, not applied (pass 2)",
				f))
		}
	}
	return load, info, nil
}
