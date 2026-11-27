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

// PreparedSubset is one foreign project's separately materialised package
// set. ManifestDir is the project boundary the runtime resolves imports from;
// Dir is an mcpx-owned cache entry, never a directory inside that project.
type PreparedSubset struct {
	ManifestPath string
	ManifestDir  string
	Dir          string
	Key          string
	Hit          bool
	Packages     []ResolvedPackage
}

// PreparedSet is the outcome of stage 3 for one resolved set.
type PreparedSet struct {
	// Dir is the prepared environment's directory in the cache. It is
	// empty when the resolved set had no entry-project packages.
	Dir string
	// Key is the cache key Dir lives under, derived from the canonical
	// entry-project set.
	Key string
	// Hit reports that the entry-project cache already existed.
	Hit bool
	// Packages are the canonical entry-project pins, sorted by name.
	Packages []ResolvedPackage
	// Subsets is the number of cross-project subsets retained by stage 2.
	Subsets int
	// PreparedSubsets contains one content-addressed cache environment per
	// non-empty foreign project. Each is keyed by project manifest identity
	// and canonical pins, so different projects never collide merely because
	// they happen to declare the same versions.
	PreparedSubsets []PreparedSubset
	// warmLeases pin entries between PrepareSet and runner wiring. Once the
	// runtime has acquired its own leases, it releases these temporary ones.
	warmLeases []*cache.Lease
}

// ReleaseWarmLeases releases the temporary leases held across the
// preflight-to-runner handoff. It is safe to call more than once.
func (p *PreparedSet) ReleaseWarmLeases() {
	if p == nil {
		return
	}
	for _, lease := range p.warmLeases {
		lease.Release()
	}
	p.warmLeases = nil
}

type preparedSubsetPlan struct {
	manifestPath string
	manifestDir  string
	packages     []ResolvedPackage
}

// canonicalSubsetPlans validates project identities and normalises the
// foreign package pins before any cache entry is written. Two distinct
// manifests resolving into one directory cannot be linked independently, so
// that ambiguous shape fails closed rather than selecting one project's pins.
func canonicalSubsetPlans(subsets []DependencySubset) ([]preparedSubsetPlan, error) {
	ordered := append([]DependencySubset(nil), subsets...)
	sort.Slice(ordered, func(i, j int) bool {
		return ordered[i].Manifest.Path < ordered[j].Manifest.Path
	})
	plans := make([]preparedSubsetPlan, 0, len(ordered))
	dirs := make(map[string]string, len(ordered))
	for _, subset := range ordered {
		pins, err := canonicalPins(subset.Packages)
		if err != nil {
			return nil, fmt.Errorf("cross-project subset %s: %w", subset.Manifest.Path, err)
		}
		if len(pins) == 0 {
			continue
		}
		manifestPath := filepath.Clean(subset.Manifest.Path)
		manifestDir := filepath.Clean(subset.Manifest.Dir)
		if !filepath.IsAbs(manifestPath) || !filepath.IsAbs(manifestDir) {
			return nil, fmt.Errorf("cross-project subset manifest path and directory must be absolute: %q in %q", subset.Manifest.Path, subset.Manifest.Dir)
		}
		rel, err := filepath.Rel(manifestDir, manifestPath)
		if err != nil || rel == "." || rel == ".." || strings.HasPrefix(rel, ".."+string(filepath.Separator)) {
			return nil, fmt.Errorf("cross-project subset manifest %q is not inside project directory %q", manifestPath, manifestDir)
		}
		if previous, ok := dirs[manifestDir]; ok && previous != manifestPath {
			return nil, fmt.Errorf("cross-project manifests %q and %q share directory %q; isolated node_modules cannot represent both", previous, manifestPath, manifestDir)
		}
		dirs[manifestDir] = manifestPath
		plans = append(plans, preparedSubsetPlan{manifestPath: manifestPath, manifestDir: manifestDir, packages: pins})
	}
	return plans, nil
}

func preparedSetKey(prefix, identity string, pkgs []ResolvedPackage) string {
	parts := make([]string, 0, len(pkgs)+2)
	parts = append(parts, prefix)
	if identity != "" {
		parts = append(parts, identity)
	}
	for _, p := range pkgs {
		parts = append(parts, p.Name+"@"+p.Version)
	}
	return cache.KeyString(parts...)
}

func putPreparedSet(ctx context.Context, c *cache.Cache, key string, pkgs []ResolvedPackage, inst Installer, lim cache.Limits) (string, bool, error) {
	return c.Put(prepareKind, key, lim, func(dir string) error {
		if err := writePreparedManifest(dir, pkgs); err != nil {
			return err
		}
		return inst.Install(ctx, dir, pkgs)
	})
}

// PrepareSet materialises the entry project's set and each non-empty isolated
// subset into separate content-addressed cache entries. The source projects
// are read-only: stage 3 writes only under cacheRoot. A failed install never
// leaves a partial cache entry, and temporary leases keep completed entries
// from eviction until the runner takes over.
func PrepareSet(ctx context.Context, cacheRoot string, set ResolvedSet, inst Installer) (PreparedSet, error) {
	pkgs, err := canonicalPins(set.Packages)
	if err != nil {
		return PreparedSet{}, err
	}
	subsets, err := canonicalSubsetPlans(set.Subsets)
	if err != nil {
		return PreparedSet{}, err
	}
	out := PreparedSet{Packages: pkgs, Subsets: len(subsets)}
	if len(pkgs) == 0 && len(subsets) == 0 {
		return out, nil
	}
	if cacheRoot == "" {
		return PreparedSet{}, errors.New("no cache directory is available for the prepared environment (MCPX_CACHE_DIR is unset and no home directory could be determined)")
	}
	if inst == nil {
		inst = NPMInstaller{}
	}
	completed := false
	defer func() {
		if !completed {
			out.ReleaseWarmLeases()
		}
	}()
	c := cache.Open(cacheRoot)
	lim := cache.Limits{
		MaxBytes:      defaults.CacheMaxBytes,
		MaxAge:        defaults.CacheMaxAge,
		LeaseStaleAge: defaults.CacheLeaseStaleAge,
	}
	if len(pkgs) > 0 {
		key := preparedSetKey("mcpx-prepare-v1", "", pkgs)
		dir, hit, err := putPreparedSet(ctx, c, key, pkgs, inst, lim)
		if err != nil {
			return PreparedSet{}, fmt.Errorf("materialise the resolved dependency set: %w", err)
		}
		lease, err := cache.Acquire(dir)
		if err != nil {
			return PreparedSet{}, fmt.Errorf("lease the prepared dependency set at %s: %w", dir, err)
		}
		out.warmLeases = append(out.warmLeases, lease)
		out.Dir, out.Key, out.Hit = dir, key, hit
	}
	for _, subset := range subsets {
		key := preparedSetKey("mcpx-prepare-subset-v1", subset.manifestPath, subset.packages)
		dir, hit, err := putPreparedSet(ctx, c, key, subset.packages, inst, lim)
		if err != nil {
			return PreparedSet{}, fmt.Errorf("materialise the cross-project subset for %s: %w", subset.manifestPath, err)
		}
		lease, err := cache.Acquire(dir)
		if err != nil {
			return PreparedSet{}, fmt.Errorf("lease the cross-project subset for %s at %s: %w", subset.manifestPath, dir, err)
		}
		out.warmLeases = append(out.warmLeases, lease)
		out.PreparedSubsets = append(out.PreparedSubsets, PreparedSubset{
			ManifestPath: subset.manifestPath,
			ManifestDir:  subset.manifestDir,
			Dir:          dir,
			Key:          key,
			Hit:          hit,
			Packages:     subset.packages,
		})
	}
	completed = true
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
	if len(prepared.PreparedSubsets) > 0 {
		r.Add(Problem{
			What:  fmt.Sprintf("prepared %d isolated cross-project dependency subset(s) in the content-addressed cache", len(prepared.PreparedSubsets)),
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
