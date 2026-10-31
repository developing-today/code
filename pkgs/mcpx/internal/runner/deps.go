package runner

import (
	"bytes"
	"context"
	"crypto/sha256"
	"encoding/hex"
	"encoding/json"
	"fmt"
	"os"
	"os/exec"
	"path/filepath"
	"sort"
	"strings"
	"sync"

	"github.com/dezren39/mcpx/internal/deps"
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
	// Lockfiles carries every lockfile found beside the manifest. The
	// one that governs is selected per runtime at materialisation
	// time (see selectedLockfile); the singular fields above stay the
	// transport's first entry for callers that predate the list.
	Lockfiles []Lockfile
	// Source is the resolved deps.source.
	Source string
	// AutoInstall is the resolved deps.autoInstall: off, fallback or
	// always.
	AutoInstall string
}

// Lockfile is one lockfile's name and content, as transported.
type Lockfile struct {
	Name    string
	Content []byte
}

// lockfilePreference names, per runtime kind, the lockfiles that
// runtime honours frozen, most specific first.
var lockfilePreference = map[string][]string{
	KindBun:  {"bun.lock", "bun.lockb"},
	KindNode: {"package-lock.json", "npm-shrinkwrap.json"},
	KindDeno: {"deno.lock"},
}

// selectedLockfile returns the lockfile that governs a run of kind.
// A project may carry several (bun.lock beside package-lock.json);
// picking the first transported regardless of runtime froze bun runs
// correctly and node runs not at all (Codex review, PR #2589). With
// no kind-preferred candidate the first transported file wins, which
// is the behaviour sets transported before the list existed had.
func (d *Deps) selectedLockfile(kind string) (string, []byte, bool) {
	type cand struct {
		name    string
		content []byte
	}
	var cands []cand
	seen := map[string]bool{}
	for _, l := range d.Lockfiles {
		if l.Name == "" || len(l.Content) == 0 || seen[l.Name] {
			continue
		}
		seen[l.Name] = true
		cands = append(cands, cand{l.Name, l.Content})
	}
	if len(d.Lockfile) > 0 && d.LockfileName != "" && !seen[d.LockfileName] {
		cands = append(cands, cand{d.LockfileName, d.Lockfile})
	}
	if len(cands) == 0 {
		return "", nil, false
	}
	for _, want := range lockfilePreference[kind] {
		for _, c := range cands {
			if c.name == want {
				return c.name, c.content, true
			}
		}
	}
	return cands[0].name, cands[0].content, true
}

// fingerprint identifies the set's content for the execution-directory
// key: two runs sharing a generated client but resolving different
// packages must not share a materialised environment (Codex review,
// PR #2589). Nil and empty sets fingerprint alike ("").
func (d *Deps) fingerprint() string {
	if d == nil {
		return ""
	}
	h := sha256.New()
	fmt.Fprintf(h, "packages:%q\nmanifest:%q:%s\nsource:%s auto:%s\n",
		d.Packages, d.ManifestName, d.Manifest, d.Source, d.AutoInstall)
	writeLock := func(name string, content []byte) {
		sum := sha256.Sum256(content)
		fmt.Fprintf(h, "lock:%s:%s\n", name, hex.EncodeToString(sum[:]))
	}
	if len(d.Lockfile) > 0 {
		writeLock(d.LockfileName, d.Lockfile)
	}
	for _, l := range d.Lockfiles {
		writeLock(l.Name, l.Content)
	}
	return hex.EncodeToString(h.Sum(nil))
}

// splitSpecifier splits "name@version" (or "@scope/name@version") into
// name and version; the version is "" for a bare name.
//
// The split is at the first non-leading "@", the same rule deps.Parse
// applies at the resolution boundary: an npm alias specifier such as
// "alias@npm:@scope/pkg@2.0.0" has version "npm:@scope/pkg@2.0.0",
// and splitting at the LAST "@" corrupted it into a package literally
// named "alias@npm:@scope/pkg" (Codex review, PR #2589).
func splitSpecifier(spec string) (name, version string) {
	for i := 1; i < len(spec); i++ {
		if spec[i] == '@' {
			return spec[:i], spec[i+1:]
		}
	}
	return spec, ""
}

// effectiveDeps merges the explicit packages into the manifest's
// dependencies. A contradiction was already reported as a conflict by
// the caller (internal/deps), so a name seen twice here is agreement,
// not a second opinion -- but agreement has a seniority: a compatible
// exact pin from the caller governs over the manifest's range, because
// the pin is the resolved version and the range is only its envelope.
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
		if manifestVersion, taken := out[name]; taken {
			// Agreement is not identity: a compatible exact pin is
			// the version the caller resolved and asked for, so it
			// governs over the manifest's range (Codex review,
			// PR #2589). A contradicting pin was already reported
			// as a conflict by internal/deps and never reaches
			// here as agreement; the manifest keeps governing it.
			if version != "" && deps.CompatiblePin(manifestVersion, version) {
				out[name] = version
			}
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
func materialiseDeps(dir, kind string, d *Deps, fileRun bool, projectDir string) ([]string, error) {
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
		// The materialised copy is always package.json, whatever
		// name resolution read: deps.manifest may select an
		// alt-manifest.json, but npm ci and the runtimes look for
		// package.json beside the lockfile, so preserving the
		// selected basename would strand the frozen install
		// (Codex review, PR #2589). The transport already does
		// the same for JSONC manifests.
		name := "package.json"
		// The materialised copy is an installation + resolution
		// manifest for THIS runner's workdir, not the project:
		// - its lifecycle scripts must not travel with it. Only the
		//   manifest and lockfile were transported, so a project's
		//   prepare/preinstall would run in the workdir against
		//   files that are not there (and is a code-execution
		//   surface the caller never asked mcpx to run). Dependency
		//   packages keep their own hooks; only the root project's
		//   scripts are dropped (Codex review, PR #2589).
		// - a project "type": "commonjs" must not reclassify the
		//   generated execution files, which are ESM by
		//   construction (the launcher's imports); Node would fail
		//   them with "Cannot use import statement outside a
		//   module" (Codex review, PR #2589).
		delete(manifest, "scripts")
		if manifest["type"] == "commonjs" {
			manifest["type"] = "module"
		}
		text, err := json.MarshalIndent(manifest, "", "  ")
		if err != nil {
			return nil, fmt.Errorf("deps: render manifest: %w", err)
		}
		if err := writeIfChanged(filepath.Join(dir, name), string(text)+"\n"); err != nil {
			return nil, fmt.Errorf("deps: write manifest: %w", err)
		}
	}

	if lockName, lockContent, ok := d.selectedLockfile(kind); ok {
		name := filepath.Base(lockName)
		if name == "" || name == "." || name == ".." {
			return nil, fmt.Errorf("deps: lockfile name %q is not a file name", lockName)
		}
		if err := writeBytesIfChanged(filepath.Join(dir, name), lockContent); err != nil {
			return nil, fmt.Errorf("deps: write lockfile: %w", err)
		}
	}

	if kind == KindDeno && len(deps) == 0 {
		// An imports map is written only by this function, so one
		// sitting in a shared workdir belongs to an earlier, different
		// set; leaving it would keep that set's pins in force for a
		// run that resolved none.
		_ = os.Remove(filepath.Join(dir, "deno.json"))
	}
	if kind == KindDeno && len(deps) > 0 {
		imports := map[string]string{}
		var scopes map[string]map[string]string
		// The generated config replaces the project's own deno.json
		// for this run (--config selects ONE file; deno does not
		// merge configs), so the project's import mappings must be
		// carried into it or a script using project aliases or JSR
		// mappings loses them the moment dependency pins are added
		// (Codex review, PR #2589). The resolved pins are senior:
		// they overwrite a project mapping for the same specifier.
		if projectDir != "" {
			if pImports, pScopes, cfgDir, ok := loadProjectDenoConfig(projectDir); ok {
				for k, v := range pImports {
					imports[k] = rebaseDenoMapping(v, cfgDir)
				}
				if len(pScopes) > 0 {
					scopes = map[string]map[string]string{}
					for scope, m := range pScopes {
						rm := map[string]string{}
						for k, v := range m {
							rm[k] = rebaseDenoMapping(v, cfgDir)
						}
						// The scope KEY is a path prefix too: left
						// unrebased it would point into the workdir
						// instead of the project's directory, and the
						// scoped mappings under it would never match
						// an import from the project (Codex review,
						// PR #2589).
						scopes[rebaseDenoMapping(scope, cfgDir)] = rm
					}
				}
			}
		}
		for name, version := range deps {
			if version == "*" || version == "" {
				imports[name] = "npm:" + name
			} else {
				imports[name] = "npm:" + name + "@" + version
			}
		}
		rendered := map[string]any{"imports": imports}
		if len(scopes) > 0 {
			rendered["scopes"] = scopes
		}
		text, err := json.MarshalIndent(rendered, "", "  ")
		if err != nil {
			return nil, fmt.Errorf("deps: render deno.json: %w", err)
		}
		if err := writeIfChanged(filepath.Join(dir, "deno.json"), string(text)+"\n"); err != nil {
			return nil, fmt.Errorf("deps: write deno.json: %w", err)
		}
	}

	flags := depsFlags(kind, d)
	if fileRun && kind == KindDeno && len(deps) > 0 {
		// A file script runs from its own directory, and deno
		// discovers configuration from the working directory and
		// its ancestors \u2014 never from this workdir. Without an
		// explicit --config the imports map above governs
		// nothing for file runs: their imports resolve against
		// the project (or fail), not the resolved set (Codex
		// review, PR #2589). Inline scripts live in the workdir,
		// so discovery already finds the map for them.
		flags = append(flags, "--config", filepath.Join(dir, "deno.json"))
	}
	return flags, nil
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
	_, _, frozen := d.selectedLockfile(kind)
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
		if name, _, ok := d.selectedLockfile(kind); ok && name == "deno.lock" {
			return []string{"--frozen"}
		}
	}
	return nil
}

// frozenNPM reports whether this run must install a frozen npm set
// before executing: a node run carrying a package-lock.json or an
// npm-shrinkwrap.json -- npm ci accepts either name for a frozen
// installation, and a project carrying only the shrinkwrap is frozen
// too (Codex review, PR #2589). npm ci installs exactly the lockfile
// into the workdir's node_modules and fails when the lockfile and
// manifest disagree, which is what frozen means; it runs in the exec
// workdir, never a user's project.
func (d *Deps) frozenNPM(kind string) bool {
	if kind != KindNode {
		return false
	}
	name, _, ok := d.selectedLockfile(kind)
	return ok && (name == "package-lock.json" || name == "npm-shrinkwrap.json")
}

// frozenInstallLocks serialises first installation per workdir:
// concurrent runs sharing a content-addressed workdir (same client,
// same dep fingerprint) must not install underneath each other --
// npm ci removes the existing node_modules before installing, so a
// second run's install can delete the set a live first run is still
// resolving imports from (Codex review, PR #2589).
var frozenInstallLocks sync.Map // map[string]*sync.Mutex, keyed by workdir

// installFrozenNPM runs npm ci in dir exactly once per dep set: the
// first run for a fingerprint installs and records a marker; later
// runs with the same fingerprint and an intact node_modules skip the
// install entirely. A changed set (different fingerprint), a missing
// marker, or a pruned node_modules installs again.
func installFrozenNPM(ctx context.Context, dir string, d *Deps) error {
	mu, _ := frozenInstallLocks.LoadOrStore(dir, &sync.Mutex{})
	lock := mu.(*sync.Mutex)
	lock.Lock()
	defer lock.Unlock()

	marker := filepath.Join(dir, ".mcpx-deps-installed")
	want := d.fingerprint()
	if b, err := os.ReadFile(marker); err == nil && string(b) == want {
		if st, err := os.Stat(filepath.Join(dir, "node_modules")); err == nil && st.IsDir() {
			return nil
		}
	}
	ci := exec.CommandContext(ctx, "npm", "ci", "--prefix", dir, "--no-audit", "--no-fund")
	out, cerr := ci.CombinedOutput()
	if cerr != nil {
		return fmt.Errorf("deps: npm ci (frozen lockfile): %w: %s",
			cerr, strings.TrimSpace(string(out)))
	}
	_ = os.WriteFile(marker, []byte(want), 0o600)
	return nil
}

// writeBytesIfChanged is writeIfChanged for byte content (a lockfile
// may be binary: bun.lockb).
func writeBytesIfChanged(path string, content []byte) error {
	if existing, err := os.ReadFile(path); err == nil && bytes.Equal(existing, content) {
		return nil
	}
	return os.WriteFile(path, content, 0o600)
}

// loadProjectDenoConfig finds the deno configuration a script's
// project already lives under: deno discovers it from the script's
// directory upward, nearest first, deno.json before deno.jsonc in
// each directory. It returns the config's import mappings and
// scopes, plus the directory the config was found in (relative
// mappings are relative to THAT directory). ok is false when no
// project config exists or it cannot be parsed -- a config mcpx
// cannot read must not fail a run it was only enriching.
func loadProjectDenoConfig(dir string) (imports map[string]string, scopes map[string]map[string]string, cfgDir string, ok bool) {
	for d := dir; ; d = filepath.Dir(d) {
		for _, name := range []string{"deno.json", "deno.jsonc"} {
			raw, err := os.ReadFile(filepath.Join(d, name))
			if err != nil {
				continue
			}
			var parsed struct {
				Imports map[string]string            `json:"imports"`
				Scopes  map[string]map[string]string `json:"scopes"`
			}
			if err := json.Unmarshal(deps.StripJSONC(raw), &parsed); err != nil {
				return nil, nil, "", false
			}
			return parsed.Imports, parsed.Scopes, d, true
		}
		if filepath.Dir(d) == d {
			return nil, nil, "", false
		}
	}
}

// projectDenoConfigIdentity identifies the project Deno configuration
// a run's generated deno.json will carry -- the config's content plus
// the directory its relative mappings resolve against. Two runs whose
// dependency payloads are identical but whose projects contribute
// different configurations must not share a materialised workdir:
// the generated config is project-specific (aliases, absolute rebased
// paths), and Deno runs take no script-directory lock, so a shared
// directory would let one execution overwrite the config another is
// about to read (Codex review, PR #2589). "" when the project carries
// no readable config, matching the generated config carrying none.
func projectDenoConfigIdentity(dir string) string {
	imports, scopes, cfgDir, ok := loadProjectDenoConfig(dir)
	if !ok {
		return ""
	}
	h := sha256.New()
	fmt.Fprintf(h, "cfgdir:%s\n", cfgDir)
	if b, err := json.Marshal(struct {
		Imports map[string]string            `json:"imports"`
		Scopes  map[string]map[string]string `json:"scopes"`
	}{imports, scopes}); err == nil {
		h.Write(b)
	}
	return hex.EncodeToString(h.Sum(nil))
}

// depsProjectDir is the directory whose project a run's dependency
// set resolves against: a file script's own directory, or the
// execution directory for an inline script. It is the directory
// materialiseDeps loads the project Deno config from, so the workdir
// identity and the materialisation must agree on it.
func depsProjectDir(opts Options) string {
	if opts.File != "" {
		if abs, err := filepath.Abs(opts.File); err == nil {
			return filepath.Dir(abs)
		}
		return filepath.Dir(opts.File)
	}
	return opts.Dir
}

// rebaseDenoMapping rewrites a project config mapping for a
// generated config living in a different directory: relative
// mappings are resolved against the project config's directory and
// spelled as file URLs; everything else (npm:, jsr:, http(s):, bare
// specifiers, absolute paths) travels unchanged. A trailing slash
// is load-bearing -- it is what makes a mapping a prefix mapping.
func rebaseDenoMapping(v, cfgDir string) string {
	if !strings.HasPrefix(v, "./") && !strings.HasPrefix(v, "../") {
		return v
	}
	abs := filepath.Join(cfgDir, v)
	if strings.HasSuffix(v, "/") {
		abs += "/"
	}
	return "file://" + filepath.ToSlash(abs)
}
