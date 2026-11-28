package runner

import (
	"encoding/json"
	"fmt"
	"net/url"
	"os"
	"path/filepath"
	"strings"

	"github.com/developing-today/code/pkgs/mcpx/internal/cache"
	"github.com/developing-today/code/pkgs/mcpx/internal/preflight"
)

// preparedImportMapName is the file a Deno run's prepared-environment
// import map is rendered to inside the run's workdir. It is not named
// deno.json: the declared-deps wiring owns that name in the workdir,
// and an import map passed with --import-map layers over whatever
// config the run already has instead of replacing it.
const preparedImportMapName = "import-map-prepared.json"

// pointDenoRunAtPreparedEnv points a Deno file run at the environment
// stage 3 of #1915 prepared for its resolved imports, in the form
// Deno consumes: an import map rendered from the prepared entry and
// passed with --import-map. Where Node and Bun resolve a beside-script
// node_modules, Deno's own form is the map: it names the entry's
// installed packages directly, and because --import-map layers over
// the project's deno configuration (unlike --config, which replaces
// it), the project's own aliases and any declared-deps config for the
// same run keep working, with the map's pins senior for the specifiers
// it names.
//
// The map covers every package installed at the top level of the
// entry's node_modules -- the resolved pins and the hoisted transitive
// packages the installer left beside them. Each bare specifier maps to
// the package's entry point (its package.json exports/main, resolved
// on disk) and each "name/" prefix to the package directory, so
// subpath imports resolve as file paths under it. A package copy
// nested inside another package, not hoisted, is outside the map's
// reach; that is the same boundary a flat node_modules link has.
//
// A prepared entry whose node_modules is gone, a resolved pin the
// entry does not install, or a pin with no entry point Deno can
// import is an error, not a fall-back: prepare reported success, so
// running on without the environment would surface the same missing
// packages as a runtime module error -- the failure shape the
// pipeline exists to replace. The returned function releases the
// cache lease the wiring takes on the entry (#1908), held until the
// process exits as the Node/Bun wiring holds it.
//
// When the run prepared no environment, a map an earlier prepare run
// left in this workdir is removed instead, so a later run of the same
// script that prepared nothing does not silently keep importing the
// earlier set: the same discipline the node_modules form applies to
// its beside-script link.
func pointDenoRunAtPreparedEnv(prepared *preflight.PreparedSet, workDir string) (func(), []string, error) {
	mapPath := filepath.Join(workDir, preparedImportMapName)
	if prepared == nil || prepared.Dir == "" {
		_ = os.Remove(mapPath)
		return nil, nil, nil
	}
	modules := filepath.Join(prepared.Dir, "node_modules")
	if st, serr := os.Stat(modules); serr != nil || !st.IsDir() {
		return nil, nil, fmt.Errorf("imports: the prepared dependency environment at %s has no node_modules to run against", prepared.Dir)
	}
	imports, err := preparedImportMap(prepared)
	if err != nil {
		return nil, nil, err
	}
	text, err := json.MarshalIndent(map[string]any{"imports": imports}, "", "  ")
	if err != nil {
		return nil, nil, fmt.Errorf("imports: render the prepared import map: %w", err)
	}
	if err := writeIfChanged(mapPath, string(text)+"\n"); err != nil {
		return nil, nil, fmt.Errorf("imports: write the prepared import map: %w", err)
	}
	// Lease the cache entry for the run's lifetime so pruning cannot
	// delete it from under a live exec (#1908) -- the same lease the
	// Node/Bun wiring takes.
	var lease *cache.Lease
	if l, lerr := cache.Acquire(prepared.Dir); lerr == nil {
		lease = l
	}
	release := func() {
		if lease != nil {
			lease.Release()
		}
	}
	return release, []string{"--import-map", mapPath}, nil
}

// preparedImportMap renders the import map for a prepared
// environment: one exact and one prefix mapping per package installed
// at the top level of the entry's node_modules. A resolved pin the
// entry does not install, or whose package has no resolvable entry
// point, fails the render; an unpinned package with no entry point
// keeps only its prefix mapping, so its subpath file imports still
// resolve and Deno's own resolution owns its bare specifier.
func preparedImportMap(prepared *preflight.PreparedSet) (map[string]string, error) {
	modules := filepath.Join(prepared.Dir, "node_modules")
	installed, err := installedTopLevelPackages(modules)
	if err != nil {
		return nil, fmt.Errorf("imports: read the prepared environment at %s: %w", prepared.Dir, err)
	}
	pinned := map[string]bool{}
	for _, p := range prepared.Packages {
		pinned[p.Name] = true
		if !installed[p.Name] {
			return nil, fmt.Errorf("imports: the prepared dependency environment at %s does not install %s, which the resolved set pins", prepared.Dir, p.Name)
		}
	}
	imports := map[string]string{}
	for name := range installed {
		dir := filepath.Join(modules, filepath.FromSlash(name))
		imports[name+"/"] = fileURL(dir) + "/"
		entry, ok := packageEntryPoint(dir)
		if !ok {
			if pinned[name] {
				return nil, fmt.Errorf("imports: the prepared package %s at %s has no entry point deno can import", name, dir)
			}
			continue
		}
		imports[name] = fileURL(entry)
	}
	return imports, nil
}

// installedTopLevelPackages lists the packages a node_modules
// directory holds at its top level, expanding scope directories
// (@scope/name) one level. Dot entries (.bin and friends) are not
// packages and are skipped.
func installedTopLevelPackages(modules string) (map[string]bool, error) {
	out := map[string]bool{}
	entries, err := os.ReadDir(modules)
	if err != nil {
		return nil, err
	}
	for _, e := range entries {
		if strings.HasPrefix(e.Name(), ".") {
			continue
		}
		path := filepath.Join(modules, e.Name())
		if st, serr := os.Stat(path); serr != nil || !st.IsDir() {
			continue
		}
		if !strings.HasPrefix(e.Name(), "@") {
			out[e.Name()] = true
			continue
		}
		subs, rerr := os.ReadDir(path)
		if rerr != nil {
			continue
		}
		for _, s := range subs {
			if strings.HasPrefix(s.Name(), ".") {
				continue
			}
			if st, serr := os.Stat(filepath.Join(path, s.Name())); serr == nil && st.IsDir() {
				out[e.Name()+"/"+s.Name()] = true
			}
		}
	}
	return out, nil
}

// packageEntryPoint resolves the file a bare import of an installed
// package loads: the "." target of its package.json exports when it
// declares one, else its main, else its module, else index.js. A
// candidate counts only when it exists on disk as a file -- a
// manifest naming a file the install does not contain is no entry
// point at all.
func packageEntryPoint(dir string) (string, bool) {
	var manifest struct {
		Exports json.RawMessage `json:"exports"`
		Main    string          `json:"main"`
		Module  string          `json:"module"`
	}
	if data, err := os.ReadFile(filepath.Join(dir, "package.json")); err == nil {
		_ = json.Unmarshal(data, &manifest)
	}
	var candidates []string
	if len(manifest.Exports) > 0 {
		if rel := exportsEntryPoint(manifest.Exports); rel != "" {
			candidates = append(candidates, rel)
		}
	}
	if manifest.Main != "" {
		candidates = append(candidates, manifest.Main)
	}
	if manifest.Module != "" {
		candidates = append(candidates, manifest.Module)
	}
	candidates = append(candidates, "index.js")
	for _, rel := range candidates {
		p := filepath.Join(dir, filepath.FromSlash(rel))
		if st, err := os.Stat(p); err == nil && !st.IsDir() {
			return p, true
		}
	}
	return "", false
}

// exportsEntryPoint extracts the "." target from a package.json
// exports value: a string is the target itself; an object's "." key
// holds it; an object without one is itself the "." condition map,
// walked in Deno's preference order (deno, import, default, then the
// rest). Subpath-only maps and arrays yield "".
func exportsEntryPoint(raw json.RawMessage) string {
	var s string
	if err := json.Unmarshal(raw, &s); err == nil {
		return s
	}
	var obj map[string]json.RawMessage
	if err := json.Unmarshal(raw, &obj); err != nil {
		return ""
	}
	if dot, ok := obj["."]; ok {
		return exportsEntryPoint(dot)
	}
	for _, cond := range []string{"deno", "import", "default", "node", "require"} {
		if v, ok := obj[cond]; ok {
			if r := exportsEntryPoint(v); r != "" {
				return r
			}
		}
	}
	return ""
}

// fileURL spells a filesystem path as the file: URL an import map
// target needs.
func fileURL(path string) string {
	return (&url.URL{Scheme: "file", Path: filepath.ToSlash(path)}).String()
}
