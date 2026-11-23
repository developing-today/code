package preflight

import (
	"context"
	"encoding/json"
	"errors"
	"fmt"
	"os"
	"os/exec"
	"path/filepath"
	"sort"
	"strings"

	"github.com/developing-today/code/pkgs/mcpx/internal/cache"
	"github.com/developing-today/code/pkgs/mcpx/internal/defaults"
	"github.com/developing-today/code/pkgs/mcpx/internal/homeutil"
)

// Stage 3 (Prepare) of #1915: materialise a resolved dependency set into
// a prepared environment in the content-addressed cache (#1908).
//
// Stages 1 and 2 compute the import graph and resolve it into a data-only
// set; until this stage, prepare mode ended at a report saying the set
// "was resolved, but materialising it is stage 3". PrepareSet is that
// materialisation for the entry project's set: the exact recorded pins
// become a package.json in a cache entry keyed by the canonical set, and
// an installer populates its node_modules. The same set always keys the
// same entry -- a warm cache is a hit and installs nothing again -- and a
// different set can never collide with it, the same discipline the
// runner already applies to its exec workdirs.
//
// The recorded version is installed as declared: an exact pin installs
// exactly, and a range a manifest declared is written as declared and
// resolved by the installer at install time; the install's lockfile is
// stored inside the entry, so what actually materialised stays auditable
// and a later entry for the same key is the cached one, never a
// re-resolution.
//
// One part of the stage-3 design is deliberately not here yet, and the
// report says so rather than implying coverage: cross-project subsets
// are resolved and counted but not materialised (their design puts a
// node_modules beside the foreign project's files, which is a write
// into a user's project and needs its own opt-in). The runner consumes
// the prepared environment for file runs
// (runner.pointRunAtPreparedEnv): node and bun take the entry's
// node_modules form, linked beside the script; deno takes an
// import-map form rendered from the same entry.

// prepareKind is the cache kind prepared environments live under, so
// `mcpx cache` lists and prunes them with everything else.
const prepareKind = "imports"

// preparedManifestName is the package.json name every prepared
// environment carries. It is the marker IsPreparedNodeModules
// recognises a prepared environment by, so a consumer deciding what
// left a node_modules directory behind does not have to know the
// cache's layout.
const preparedManifestName = "mcpx-prepared-imports"

// Installer populates a prepared environment directory whose
// package.json already carries the resolved pins.
type Installer interface {
	Install(ctx context.Context, dir string, pkgs []ResolvedPackage) error
}

// NPMInstaller is the production Installer: npm installs the pinned
// manifest into the entry. Scripts stay off -- a prepared environment is
// dependency code mcpx fetched because a script imports it, and running
// its install scripts would be executing it before the run starts.
type NPMInstaller struct {
	// Bin is the npm executable; empty means "npm" from PATH.
	Bin string
}

func (n NPMInstaller) Install(ctx context.Context, dir string, _ []ResolvedPackage) error {
	bin := n.Bin
	if bin == "" {
		bin = "npm"
	}
	cmd := exec.CommandContext(ctx, bin, "install",
		"--ignore-scripts", "--no-audit", "--no-fund", "--loglevel=error")
	cmd.Dir = dir
	out, err := cmd.CombinedOutput()
	if err != nil {
		return fmt.Errorf("npm install in the prepared environment: %w: %s",
			err, strings.TrimSpace(string(out)))
	}
	return nil
}

// PreparedSet is the outcome of stage 3 for one resolved set.
type PreparedSet struct {
	// Dir is the prepared environment's directory in the cache. It is
	// empty when the resolved set had no entry-project packages: there
	// was nothing to materialise and no entry was created.
	Dir string
	// Key is the cache key Dir lives under, derived from the canonical
	// resolved set.
	Key string
	// Hit reports that the entry already existed and nothing installed.
	Hit bool
	// Packages are the canonical pins that were materialised, sorted by
	// name.
	Packages []ResolvedPackage
	// Subsets counts the cross-project subsets the resolved set kept
	// separate. They are not materialised by this stage.
	Subsets int
}

// PrepareSet materialises set's entry-project packages into the cache
// rooted at cacheRoot. An empty set is a successful no-op. A failure
// leaves no cache entry behind: the cache stores an entry only when its
// fill completes, so a failed install is retried by the next prepare
// rather than served half-built forever.
func PrepareSet(ctx context.Context, cacheRoot string, set ResolvedSet, inst Installer) (PreparedSet, error) {
	pkgs, err := canonicalPins(set.Packages)
	if err != nil {
		return PreparedSet{}, err
	}
	out := PreparedSet{Packages: pkgs, Subsets: len(set.Subsets)}
	if len(pkgs) == 0 {
		return out, nil
	}
	if cacheRoot == "" {
		return PreparedSet{}, errors.New("no cache directory is available for the prepared environment (MCPX_CACHE_DIR is unset and no home directory could be determined)")
	}
	if inst == nil {
		inst = NPMInstaller{}
	}
	parts := make([]string, 0, len(pkgs)+1)
	parts = append(parts, "mcpx-prepare-v1")
	for _, p := range pkgs {
		parts = append(parts, p.Name+"@"+p.Version)
	}
	key := cache.KeyString(parts...)
	c := cache.Open(cacheRoot)
	lim := cache.Limits{
		MaxBytes:      defaults.CacheMaxBytes,
		MaxAge:        defaults.CacheMaxAge,
		LeaseStaleAge: defaults.CacheLeaseStaleAge,
	}
	dir, hit, err := c.Put(prepareKind, key, lim, func(dir string) error {
		if err := writePreparedManifest(dir, pkgs); err != nil {
			return err
		}
		return inst.Install(ctx, dir, pkgs)
	})
	if err != nil {
		return PreparedSet{}, fmt.Errorf("materialise the resolved dependency set: %w", err)
	}
	out.Dir, out.Key, out.Hit = dir, key, hit
	return out, nil
}

// canonicalPins reduces a resolved set's packages to the sorted,
// name-unique pins a prepared environment is keyed and built by. Stage 2
// already dedupes by name; a conflicting or unpinned entry here means
// the set was built some other way, and preparing from it would key two
// different environments alike or install an unspecified version, so it
// is refused rather than guessed at.
func canonicalPins(in []ResolvedPackage) ([]ResolvedPackage, error) {
	byName := map[string]ResolvedPackage{}
	for _, p := range in {
		if p.Name == "" {
			return nil, errors.New("the resolved dependency set contains a package with no name")
		}
		if p.Version == "" {
			return nil, fmt.Errorf("the resolved dependency set leaves %s unpinned; there is no version to prepare", p.Name)
		}
		if cur, ok := byName[p.Name]; ok && cur.Version != p.Version {
			return nil, fmt.Errorf("the resolved dependency set pins %s to both %s and %s", p.Name, cur.Version, p.Version)
		}
		byName[p.Name] = p
	}
	out := make([]ResolvedPackage, 0, len(byName))
	for _, p := range byName {
		out = append(out, p)
	}
	sort.Slice(out, func(i, j int) bool { return out[i].Name < out[j].Name })
	return out, nil
}

// writePreparedManifest writes the entry's package.json: the resolved
// pins, as declared, and nothing else -- no scripts, no metadata that
// could drift from the set the entry is keyed by.
func writePreparedManifest(dir string, pkgs []ResolvedPackage) error {
	deps := make(map[string]string, len(pkgs))
	for _, p := range pkgs {
		deps[p.Name] = p.Version
	}
	manifest := map[string]any{
		"name":         preparedManifestName,
		"version":      "0.0.0",
		"private":      true,
		"dependencies": deps,
	}
	b, err := json.MarshalIndent(manifest, "", "  ")
	if err != nil {
		return err
	}
	return os.WriteFile(filepath.Join(dir, "package.json"), append(b, '\n'), 0o644)
}

// IsPreparedNodeModules reports whether a node_modules directory is a
// prepared environment's: the directory beside it is the cache entry
// stage 3 wrote, recognisable by the manifest name it carries. The
// runner uses it to tell a link it may refresh or remove (one a
// prepare run left) from a link the project put there itself.
func IsPreparedNodeModules(modulesDir string) bool {
	b, err := os.ReadFile(filepath.Join(filepath.Dir(modulesDir), "package.json"))
	if err != nil {
		return false
	}
	var manifest struct {
		Name string `json:"name"`
	}
	if err := json.Unmarshal(b, &manifest); err != nil {
		return false
	}
	return manifest.Name == preparedManifestName
}

// prepareResolved runs stage 3 for one resolved set and records the
// outcome on the report: the prepared environment structurally (for the
// consumers that come next) and as report lines (for the user reading
// this run's warnings today). A materialisation failure is fatal: the
// caller asked for prepare by name, and letting the run continue would
// surface the same missing packages later as a runtime module error --
// the failure shape this pipeline exists to replace.
func prepareResolved(ctx context.Context, r *Report, set ResolvedSet, opts ResolveOptions) {
	prepared, err := PrepareSet(ctx, opts.prepareCacheRoot(), set, opts.PrepareInstall)
	if err != nil {
		r.Add(Problem{
			What:  "the dependency set was resolved, but materialising it failed: " + err.Error(),
			Where: "script.imports",
			Fix:   "check the package installer and the cache directory, or set script.imports to report",
			Fatal: true,
		})
		return
	}
	r.Prepared = &prepared
	if len(prepared.Packages) > 0 {
		how := "installed"
		if prepared.Hit {
			how = "already in the cache"
		}
		r.Add(Problem{
			What:  fmt.Sprintf("prepared dependency environment (%s): %s at %s", how, packageList(prepared.Packages), prepared.Dir),
			Where: "script.imports",
		})
	}
	if prepared.Subsets > 0 {
		r.Add(Problem{
			What:  fmt.Sprintf("%d cross-project dependency subset(s) were resolved but are not materialised yet: prepare covers the entry project's set only", prepared.Subsets),
			Where: "script.imports",
		})
	}
}

// prepareCacheRoot resolves the cache root stage 3 writes to: the
// explicit option first, then the same derivation daemon.ResolvePaths
// applies to its cache leg (MCPX_CACHE_DIR, then XDG_CACHE_HOME, then
// the home directory). The daemon package cannot be imported here -- it
// reaches preflight through execsvc and the runner -- so the derivation
// is mirrored, and the two must be kept in step.
func (o ResolveOptions) prepareCacheRoot() string {
	if o.PrepareCache != "" {
		return o.PrepareCache
	}
	if v := os.Getenv("MCPX_CACHE_DIR"); v != "" {
		return v
	}
	if v := os.Getenv("XDG_CACHE_HOME"); v != "" {
		return filepath.Join(v, "mcpx")
	}
	if home, err := homeutil.Home(); err == nil {
		return filepath.Join(home, ".cache", "mcpx")
	}
	return ""
}
