package preflight

import (
	"context"
	"encoding/json"
	"errors"
	"fmt"
	"net/http"
	"net/url"
	"os"
	"path/filepath"
	"sort"
	"strings"

	"github.com/developing-today/code/pkgs/mcpx/internal/defaults"
	"github.com/developing-today/code/pkgs/mcpx/internal/deps"
)

// ImportEdge is a successfully resolved edge from the static graph.
type ImportEdge struct {
	From      string
	Specifier string
	Resolved  string
	Span      *SourceSpan
	TypeOnly  bool
}

// PackageManifest is one project boundary discovered while walking the graph.
// Package maps contain the declared version/range, not a materialised install.
type PackageManifest struct {
	Path     string
	Dir      string
	Packages map[string]string
}

type ResolvedPackage struct {
	Name    string
	Version string
	Source  string
}

// DependencySubset keeps a foreign project's package declarations separate
// from the entry project's set. The prepare stage can materialise each subset
// beside the files that use it without arbitrating version conflicts here.
type DependencySubset struct {
	Manifest PackageManifest
	Packages []ResolvedPackage
}

// ResolvedSet is the stage-2 result. It is deliberately data-only: creating
// node_modules, changing a manifest, or writing a lockfile is stage 3.
type ResolvedSet struct {
	Packages []ResolvedPackage
	Subsets  []DependencySubset
}

type MissingMode string

const (
	MissingFail    MissingMode = "fail"
	MissingWarn    MissingMode = "warn"
	MissingInstall MissingMode = "install"
)

type CrossProjectMode string

const (
	CrossProjectIsolate CrossProjectMode = "isolate"
	CrossProjectMerge   CrossProjectMode = "merge"
	CrossProjectIgnore  CrossProjectMode = "ignore"
)

// ResolveOptions controls the pure Resolve stage. LatestVersion is injectable
// so tests can use a deterministic oracle; nil means the public npm registry.
type ResolveOptions struct {
	Explicit      []string
	Manifests     []PackageManifest
	Missing       MissingMode
	CrossProject  CrossProjectMode
	Network       bool
	LatestVersion func(context.Context, string) (string, error)
}

// DiscoverPackageManifests finds the nearest package.json/package.jsonc or
// deno.json for every local file in the graph. It reads but never modifies
// project files. The returned slice is ordered by path for reproducibility.
func DiscoverPackageManifests(g *Graph) ([]PackageManifest, error) {
	if g == nil {
		return nil, nil
	}
	files := append([]string(nil), g.LocalFiles...)
	files = append(files, g.Roots...)
	for _, edge := range g.Edges {
		files = append(files, edge.From, edge.Resolved)
	}
	seen := map[string]bool{}
	var manifests []PackageManifest
	for _, file := range files {
		if !filepath.IsAbs(file) {
			continue
		}
		for dir := filepath.Dir(file); ; dir = filepath.Dir(dir) {
			for _, name := range []string{"package.json", "package.jsonc", "deno.json"} {
				path := filepath.Join(dir, name)
				if seen[path] {
					continue
				}
				m, err := loadImportManifest(path)
				if errors.Is(err, os.ErrNotExist) {
					continue
				}
				if err != nil {
					return nil, err
				}
				seen[path] = true
				manifests = append(manifests, m)
			}
			if dir == filepath.Dir(dir) {
				break
			}
		}
	}
	sort.Slice(manifests, func(i, j int) bool { return manifests[i].Path < manifests[j].Path })
	return manifests, nil
}

func loadImportManifest(path string) (PackageManifest, error) {
	b, err := os.ReadFile(path)
	if err != nil {
		return PackageManifest{}, err
	}
	if filepath.Ext(path) == ".jsonc" {
		b = deps.StripJSONC(b)
	}
	m := PackageManifest{Path: path, Dir: filepath.Dir(path), Packages: map[string]string{}}
	if filepath.Base(path) == "deno.json" {
		var raw struct {
			Imports map[string]string `json:"imports"`
		}
		if err := json.Unmarshal(b, &raw); err != nil {
			return PackageManifest{}, fmt.Errorf("parse %s: %w", path, err)
		}
		for _, target := range raw.Imports {
			if name, version, ok := npmPackage(target); ok {
				if version == "" {
					version = "latest"
				}
				m.Packages[name] = version
			}
		}
		return m, nil
	}
	var raw struct {
		Dependencies         map[string]string `json:"dependencies"`
		DevDependencies      map[string]string `json:"devDependencies"`
		OptionalDependencies map[string]string `json:"optionalDependencies"`
		PeerDependencies     map[string]string `json:"peerDependencies"`
	}
	if err := json.Unmarshal(b, &raw); err != nil {
		return PackageManifest{}, fmt.Errorf("parse %s: %w", path, err)
	}
	for _, section := range []map[string]string{raw.Dependencies, raw.DevDependencies, raw.OptionalDependencies, raw.PeerDependencies} {
		for name, version := range section {
			m.Packages[name] = version
		}
	}
	return m, nil
}

// ResolveGraph turns unresolved bare imports into entries in a data-only
// dependency set. Explicit package declarations win, then the nearest
// manifest, then an exact npm latest version (announced in the report).
func ResolveGraph(ctx context.Context, g *Graph, opts ResolveOptions, dynamic []DynamicImport, dynamicMode DynamicImportMode) (ResolvedSet, *Report) {
	set := ResolvedSet{}
	r := &Report{}
	if g == nil {
		r.mergeDynamics(dynamic, dynamicMode)
		return set, r
	}
	if opts.Missing == "" {
		opts.Missing = MissingFail
	}
	if opts.CrossProject == "" {
		opts.CrossProject = CrossProjectIsolate
	}
	if opts.LatestVersion == nil {
		opts.LatestVersion = npmLatestVersion
	}
	manifests := opts.Manifests
	if len(manifests) == 0 {
		var err error
		manifests, err = DiscoverPackageManifests(g)
		if err != nil {
			r.Add(Problem{What: "could not read a project manifest while resolving imports: " + err.Error(), Where: "script.imports", Fatal: true})
			return set, r
		}
	}
	rootDir := ""
	if len(g.Roots) > 0 {
		if m := nearestManifest(g.Roots[0], manifests); m != nil {
			rootDir = m.Dir
		}
	}

	explicit := map[string]string{}
	for _, spec := range opts.Explicit {
		if name, version, ok := packageName(spec); ok {
			explicit[name] = version
		}
	}
	rootPackages := map[string]ResolvedPackage{}
	subsets := map[string]*DependencySubset{}
	add := func(dst map[string]ResolvedPackage, p ResolvedPackage) {
		if current, ok := dst[p.Name]; !ok || (current.Version == "" && p.Version != "") {
			dst[p.Name] = p
		}
	}

	for _, edge := range g.Edges {
		fromManifest := nearestManifest(edge.From, manifests)
		toManifest := nearestManifest(edge.Resolved, manifests)
		if toManifest == nil || (fromManifest != nil && fromManifest.Dir == toManifest.Dir) || toManifest.Dir == rootDir {
			continue
		}
		switch opts.CrossProject {
		case CrossProjectIgnore:
			continue
		case CrossProjectMerge:
			for name, version := range toManifest.Packages {
				add(rootPackages, ResolvedPackage{Name: name, Version: version, Source: toManifest.Path})
			}
		default:
			subset := subsets[toManifest.Path]
			if subset == nil {
				subset = &DependencySubset{Manifest: *toManifest}
				subsets[toManifest.Path] = subset
			}
			for name, version := range toManifest.Packages {
				addMap := map[string]ResolvedPackage{}
				for _, old := range subset.Packages {
					addMap[old.Name] = old
				}
				add(addMap, ResolvedPackage{Name: name, Version: version, Source: toManifest.Path})
				subset.Packages = subset.Packages[:0]
				for _, p := range addMap {
					subset.Packages = append(subset.Packages, p)
				}
			}
		}
	}

	for _, missing := range g.Missing {
		name, _, bare := packageName(missing.Specifier)
		if !bare {
			addUnresolved(r, missing, opts.Missing)
			continue
		}
		version, source := explicit[name], "explicit --package"
		manifest := nearestManifest(missing.From, manifests)
		if manifest != nil && manifest.Dir != rootDir && opts.CrossProject == CrossProjectIgnore {
			addResolutionProblem(r, fmt.Sprintf("ignored import %q from foreign project %s by script.importsCrossProject=ignore", missing.Specifier, manifest.Dir), missing.Span, opts.Missing == MissingFail)
			continue
		}
		if version == "" && manifest != nil {
			version = manifest.Packages[name]
			if version != "" {
				source = manifest.Path
			}
		}
		if version == "" {
			if !opts.Network {
				addResolutionProblem(r, fmt.Sprintf("cannot pin missing import %q: script.importsNetwork is off and no explicit or manifest version exists", missing.Specifier), missing.Span, opts.Missing == MissingFail)
				continue
			}
			var err error
			version, err = opts.LatestVersion(ctx, name)
			if err != nil || version == "" {
				what := fmt.Sprintf("cannot pin missing import %q to latest", missing.Specifier)
				if err != nil {
					what += ": " + err.Error()
				}
				addResolutionProblem(r, what, missing.Span, opts.Missing == MissingFail)
				continue
			}
			source = "latest (announced)"
			addResolutionProblem(r, fmt.Sprintf("using latest %s@%s for import %q", name, version, missing.Specifier), missing.Span, false)
		}
		p := ResolvedPackage{Name: name, Version: version, Source: source}
		if manifest != nil && manifest.Dir != rootDir && opts.CrossProject == CrossProjectIsolate {
			subset := subsets[manifest.Path]
			if subset == nil {
				subset = &DependencySubset{Manifest: *manifest}
				subsets[manifest.Path] = subset
			}
			byName := map[string]ResolvedPackage{}
			for _, old := range subset.Packages {
				byName[old.Name] = old
			}
			add(byName, p)
			subset.Packages = subset.Packages[:0]
			for _, x := range byName {
				subset.Packages = append(subset.Packages, x)
			}
		} else {
			add(rootPackages, p)
		}
		if opts.Missing == MissingFail {
			addResolutionProblem(r, fmt.Sprintf("unresolved import %q resolved to %s@%s but is not installed", missing.Specifier, name, version), missing.Span, true)
		} else if opts.Missing == MissingWarn {
			addResolutionProblem(r, fmt.Sprintf("unresolved import %q resolved to %s@%s; dependency was not installed", missing.Specifier, name, version), missing.Span, false)
		} else {
			addResolutionProblem(r, fmt.Sprintf("resolved %s@%s for import %q; stage 2 records the set but does not install it", name, version, missing.Specifier), missing.Span, false)
		}
	}
	for _, p := range rootPackages {
		set.Packages = append(set.Packages, p)
	}
	for _, subset := range subsets {
		sort.Slice(subset.Packages, func(i, j int) bool { return subset.Packages[i].Name < subset.Packages[j].Name })
		set.Subsets = append(set.Subsets, *subset)
	}
	sort.Slice(set.Packages, func(i, j int) bool { return set.Packages[i].Name < set.Packages[j].Name })
	sort.Slice(set.Subsets, func(i, j int) bool { return set.Subsets[i].Manifest.Path < set.Subsets[j].Manifest.Path })
	for _, p := range set.Packages {
		r.Add(Problem{What: fmt.Sprintf("resolved dependency set includes %s@%s (%s)", p.Name, p.Version, p.Source), Where: "script.imports"})
	}
	for _, subset := range set.Subsets {
		r.Add(Problem{What: fmt.Sprintf("isolated dependency subset %s from %s", packageList(subset.Packages), subset.Manifest.Path), Where: "script.imports"})
	}
	r.mergeDynamics(dynamic, dynamicMode)
	return set, r
}

func packageList(packages []ResolvedPackage) string {
	parts := make([]string, 0, len(packages))
	for _, p := range packages {
		parts = append(parts, p.Name+"@"+p.Version)
	}
	return strings.Join(parts, ", ")
}

func addUnresolved(r *Report, m MissingSpecifier, mode MissingMode) {
	addResolutionProblem(r, fmt.Sprintf("unresolved import %q: %s", m.Specifier, m.Err), m.Span, mode == MissingFail)
}

func addResolutionProblem(r *Report, what string, span *SourceSpan, fatal bool) {
	where := "script.imports"
	if span != nil {
		where = span.String()
	}
	r.Add(Problem{What: what, Where: where, Fatal: fatal, Fix: "declare a pinned dependency or adjust the script.imports policy"})
}

func nearestManifest(file string, manifests []PackageManifest) *PackageManifest {
	path := fileURLToPath(file)
	if !filepath.IsAbs(path) {
		return nil
	}
	var best *PackageManifest
	for i := range manifests {
		m := &manifests[i]
		rel, err := filepath.Rel(m.Dir, path)
		if err == nil && rel != ".." && !strings.HasPrefix(rel, ".."+string(filepath.Separator)) && (best == nil || len(m.Dir) > len(best.Dir)) {
			best = m
		}
	}
	return best
}

// packageName accepts npm package names and package subpaths, and returns the
// package root plus an optional pin. Relative, URL, npm:, jsr: and node: imports
// are intentionally not interpreted as ordinary package names.
func packageName(spec string) (name, version string, ok bool) {
	if spec == "" || strings.HasPrefix(spec, ".") || strings.HasPrefix(spec, "/") || strings.HasPrefix(spec, "#") || strings.Contains(spec, ":") {
		return "", "", false
	}
	if strings.HasPrefix(spec, "@") {
		parts := strings.Split(spec, "/")
		if len(parts) < 2 || parts[1] == "" {
			return "", "", false
		}
		name = strings.Join(parts[:2], "/")
		rest := strings.TrimPrefix(spec, name)
		if strings.HasPrefix(rest, "@") {
			version = strings.SplitN(strings.TrimPrefix(rest, "@"), "/", 2)[0]
		}
		return name, version, !nodeBuiltin(name)
	}
	parts := strings.Split(spec, "/")
	name = parts[0]
	if i := strings.IndexByte(name, '@'); i >= 0 {
		version, name = name[i+1:], name[:i]
	}
	return name, version, name != "" && !nodeBuiltin(name)
}

func nodeBuiltin(name string) bool {
	switch name {
	case "assert", "async_hooks", "buffer", "child_process", "cluster", "console", "constants", "crypto", "dgram", "diagnostics_channel", "dns", "domain", "events", "fs", "http", "http2", "https", "module", "net", "os", "path", "perf_hooks", "process", "punycode", "querystring", "readline", "repl", "stream", "string_decoder", "sys", "timers", "tls", "trace_events", "tty", "url", "util", "v8", "vm", "wasi", "worker_threads", "zlib":
		return true
	}
	return false
}

func npmPackage(spec string) (name, version string, ok bool) {
	if !strings.HasPrefix(spec, "npm:") {
		return "", "", false
	}
	return packageName(strings.TrimPrefix(spec, "npm:"))
}

type npmVersion struct {
	Version string `json:"version"`
}

func npmLatestVersion(ctx context.Context, name string) (string, error) {
	u := "https://registry.npmjs.org/" + strings.ReplaceAll(url.PathEscape(name), "%2F", "%2f") + "/latest"
	req, err := http.NewRequestWithContext(ctx, http.MethodGet, u, nil)
	if err != nil {
		return "", err
	}
	client := &http.Client{Timeout: defaults.PreflightNPMTimeout}
	resp, err := client.Do(req)
	if err != nil {
		return "", err
	}
	defer resp.Body.Close()
	if resp.StatusCode != http.StatusOK {
		return "", fmt.Errorf("npm registry returned %s", resp.Status)
	}
	var result npmVersion
	if err := json.NewDecoder(resp.Body).Decode(&result); err != nil {
		return "", err
	}
	if result.Version == "" {
		return "", errors.New("npm registry response had no version")
	}
	return result.Version, nil
}
