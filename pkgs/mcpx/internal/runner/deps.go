package runner

import (
	"bytes"
	"encoding/json"
	"fmt"
	"os"
	"path/filepath"
	"sort"
	"strings"
)

// Deps is a script's resolved dependency set, as the runner consumes it
// (#1913). It mirrors execsvc.DepsOptions; the runner cannot import
// execsvc (execsvc imports the runner), so execsvc converts at the
// RunWith boundary.
//
// Everything here is data, never paths: Manifest and Lockfile are the
// files' contents, because the daemon materialising them may be on
// another machine from the caller who resolved them.
type Deps struct {
	// Packages are explicit specifiers: name or name@version.
	Packages []string
	// Manifest is the manifest's text, comment-stripped when it came
	// from a package.jsonc.
	Manifest string
	// ManifestName is the filename to write it as: package.json by
	// default.
	ManifestName string
	// Lockfile is the lockfile's content, honoured frozen and never
	// rewritten.
	Lockfile []byte
	// LockfileName is the filename to write it as, such as bun.lock.
	LockfileName string
	// Source is the resolved deps.source.
	Source string
	// AutoInstall is the resolved deps.autoInstall: off, fallback or
	// always.
	AutoInstall string
}

// splitSpecifier splits "name@version" (or "@scope/name@version") into
// name and version; the version is "" for a bare name.
func splitSpecifier(spec string) (name, version string) {
	if i := strings.LastIndex(spec, "@"); i > 0 {
		return spec[:i], spec[i+1:]
	}
	return spec, ""
}

// effectiveDeps merges the explicit packages into the manifest's
// dependencies. The manifest wins a name both name: a contradiction was
// already reported as a conflict by the caller (internal/deps), so a
// name seen twice here is agreement, not a second opinion.
func (d *Deps) effectiveDeps(manifest map[string]any) map[string]string {
	out := map[string]string{}
	if manifest != nil {
		if deps, ok := manifest["dependencies"].(map[string]any); ok {
			for name, v := range deps {
				if s, ok := v.(string); ok {
					out[name] = s
				}
			}
		}
	}
	for _, spec := range d.Packages {
		name, version := splitSpecifier(spec)
		if _, taken := out[name]; taken {
			continue
		}
		if version == "" {
			version = "*"
		}
		out[name] = version
	}
	return out
}

// materialiseDeps writes the dependency set into the exec workdir and
// returns the extra runtime flags the set implies for kind.
//
// The workdir is the only place written: it is mcpx's own directory, so
// materialising there never rewrites a user's project. A file script
// keeps resolving against its own project's manifest, exactly as the
// chain resolved it; the materialised copy governs inline scripts,
// whose only home is the workdir.
//
// Per runtime:
//   - bun/node: a package.json (the resolved manifest with the explicit
//     packages merged in, or a synthesised one naming only them).
//   - deno: the same package.json, plus a deno.json imports map pinning
//     each bare specifier to its npm: equivalent, so a script's imports
//     resolve to the resolved set rather than whatever deno would pick.
//   - a lockfile is written byte-for-byte and never rewritten by mcpx.
func materialiseDeps(dir, kind string, d *Deps) ([]string, error) {
	var manifest map[string]any
	if strings.TrimSpace(d.Manifest) != "" {
		if err := json.Unmarshal([]byte(d.Manifest), &manifest); err != nil {
			return nil, fmt.Errorf("deps: manifest is not valid JSON: %w", err)
		}
	}
	deps := d.effectiveDeps(manifest)

	if manifest != nil || len(deps) > 0 {
		if manifest == nil {
			manifest = map[string]any{
				"name":    "mcpx-exec",
				"version": "0.0.0",
				"private": true,
			}
		}
		if len(deps) > 0 {
			merged := map[string]any{}
			names := make([]string, 0, len(deps))
			for name := range deps {
				names = append(names, name)
			}
			sort.Strings(names)
			for _, name := range names {
				merged[name] = deps[name]
			}
			manifest["dependencies"] = merged
		}
		name := filepath.Base(d.ManifestName)
		if name == "" || name == "." || !strings.HasSuffix(name, ".json") {
			name = "package.json"
		}
		text, err := json.MarshalIndent(manifest, "", "  ")
		if err != nil {
			return nil, fmt.Errorf("deps: render manifest: %w", err)
		}
		if err := writeIfChanged(filepath.Join(dir, name), string(text)+"\n"); err != nil {
			return nil, fmt.Errorf("deps: write manifest: %w", err)
		}
	}

	if len(d.Lockfile) > 0 && d.LockfileName != "" {
		name := filepath.Base(d.LockfileName)
		if name == "" || name == "." || name == ".." {
			return nil, fmt.Errorf("deps: lockfile name %q is not a file name", d.LockfileName)
		}
		if err := writeBytesIfChanged(filepath.Join(dir, name), d.Lockfile); err != nil {
			return nil, fmt.Errorf("deps: write lockfile: %w", err)
		}
	}

	if kind == KindDeno && len(deps) > 0 {
		imports := map[string]string{}
		for name, version := range deps {
			if version == "*" || version == "" {
				imports[name] = "npm:" + name
			} else {
				imports[name] = "npm:" + name + "@" + version
			}
		}
		text, err := json.MarshalIndent(map[string]any{"imports": imports}, "", "  ")
		if err != nil {
			return nil, fmt.Errorf("deps: render deno.json: %w", err)
		}
		if err := writeIfChanged(filepath.Join(dir, "deno.json"), string(text)+"\n"); err != nil {
			return nil, fmt.Errorf("deps: write deno.json: %w", err)
		}
	}

	return depsFlags(kind, d), nil
}

// depsFlags maps the resolved set onto runtime flags.
//
// bun: --install= is bun run's own auto-install control, so the
// deps.autoInstall enum maps onto it directly; off is --no-install, so
// a missing package fails the run instead of being fetched. A lockfile
// has no --frozen-lockfile spelling under `bun run` (that flag belongs
// to `bun install`); frozen is honoured by materialising the lockfile
// and using fallback, which installs only what is missing and resolves
// it against the lockfile, never rewriting the manifest.
//
// deno: --frozen is deno run's lockfile mode, spelled when the set
// carries deno.lock; deno otherwise caches npm packages from the
// materialised package.json/imports map itself.
//
// node never installs from the runner; a frozen npm set is installed by
// the npm-ci step in Run, not by a flag.
func depsFlags(kind string, d *Deps) []string {
	frozen := len(d.Lockfile) > 0
	switch kind {
	case KindBun:
		if frozen {
			return []string{"--install=fallback"}
		}
		switch d.AutoInstall {
		case "fallback":
			return []string{"--install=fallback"}
		case "always":
			return []string{"--install=auto"}
		default: // off, and anything unrecognised: the loud default
			return []string{"--no-install"}
		}
	case KindDeno:
		if frozen && d.LockfileName == "deno.lock" {
			return []string{"--frozen"}
		}
	}
	return nil
}

// frozenNPM reports whether this run must install a frozen npm set
// before executing: a node run carrying a package-lock.json. npm ci
// installs exactly the lockfile into the workdir's node_modules and
// fails when the lockfile and manifest disagree, which is what frozen
// means; it runs in the exec workdir, never a user's project.
func (d *Deps) frozenNPM(kind string) bool {
	return kind == KindNode && len(d.Lockfile) > 0 && d.LockfileName == "package-lock.json"
}

// writeBytesIfChanged is writeIfChanged for byte content (a lockfile
// may be binary: bun.lockb).
func writeBytesIfChanged(path string, content []byte) error {
	if existing, err := os.ReadFile(path); err == nil && bytes.Equal(existing, content) {
		return nil
	}
	return os.WriteFile(path, content, 0o600)
}
