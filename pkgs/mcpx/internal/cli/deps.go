package cli

import (
	"fmt"
	"path/filepath"
	"strings"

	"github.com/dezren39/mcpx/internal/deps"
	"github.com/dezren39/mcpx/internal/execsvc"
)

// depsOptions is the dependency set to send with a script, or nil when the
// caller asked for nothing beyond whatever the runtime finds for itself.
//
// Nil is the default and it is load-bearing: the dependency chain costs a
// manifest search and a config read, and a run that never mentioned packages
// should behave exactly as it did before #1913. The fast path therefore
// requires EVERY dependency setting at its default: an explicitly configured
// autoInstall policy or paths.deps search path is a request even when the
// source stays host and no package was named.
//
// The wire carries the manifest's TEXT rather than its path, because a daemon
// on another machine can only open its own filesystem -- the same reason
// phase lines travel as source.
//
// Discovery starts at the script's directory when the script is a file: the
// deps.manifest setting promises discovery from the script's directory, and a
// caller running /repoB/scripts/report.ts from /repoA means repoB's project.
// Inline scripts have no directory of their own, so cwd is used for them.
//
// A resolution failure is an error, not an absent set: a manifest that cannot
// be read or parsed means explicitly requested packages and policies would
// otherwise be discarded silently.
func (a *App) depsOptions(cwd string, scriptFile string) (*execsvc.DepsOptions, error) {
	cfg := deps.From(a.Settings())
	if cfg.Source == deps.SourceHost && cfg.Manifest == "" && len(cfg.Packages) == 0 &&
		cfg.AutoInstall == deps.AutoInstallOff && len(cfg.Paths.Entries) == 0 {
		return nil, nil
	}
	startDir := cwd
	if scriptFile != "" {
		startDir = filepath.Dir(scriptFile)
	}
	set, err := deps.Resolve(cfg, startDir)
	if err != nil {
		return nil, err
	}
	if len(set.Conflicts) > 0 {
		// A conflict is reported, never silently carried: the payload
		// has no field for resolution metadata, so a contradictory pin
		// that reached the wire would read as agreement. Failing here
		// names both versions and the manifest they came from.
		var parts []string
		for _, c := range set.Conflicts {
			parts = append(parts, fmt.Sprintf("%s: explicit %s contradicts manifest %s",
				c.Name, c.Explicit, c.Manifest))
		}
		return nil, fmt.Errorf("deps: %s", strings.Join(parts, "; "))
	}
	out := &execsvc.DepsOptions{
		Source:      string(cfg.Source),
		AutoInstall: string(cfg.AutoInstall),
	}
	// The payload carries the RESOLVED set, not just the explicit layer:
	// with deps.source=config the configuration directory's packages live in
	// set.Packages, and the manifest text serialized below is the project's,
	// so copying only cfg.Packages would discard them -- a remote daemon
	// cannot reconstruct them from the caller's filesystem.
	for _, p := range set.Packages {
		if p.Version != "" {
			out.Packages = append(out.Packages, p.Name+"@"+p.Version)
		} else {
			out.Packages = append(out.Packages, p.Name)
		}
	}
	if m := set.Manifest; m != nil {
		if text, err := m.JSON(); err == nil {
			out.Manifest = string(text)
			out.ManifestName = filepath.Base(m.Path)
			// A runtime is never handed a package.jsonc: the comments are
			// stripped on the way out, so it is plain JSON by then.
			if m.JSONC {
				out.ManifestName = "package.json"
			}
		}
		name, text, err := m.LockfileText()
		if err != nil {
			return nil, err
		}
		if name != "" {
			out.LockfileName, out.Lockfile = name, text
		}
	}
	return out, nil
}
