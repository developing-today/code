package cli

import (
	"path/filepath"

	"github.com/dezren39/mcpx/internal/deps"
	"github.com/dezren39/mcpx/internal/execsvc"
)

// depsOptions is the dependency set to send with a script, or nil when the
// caller asked for nothing beyond whatever the runtime finds for itself.
//
// Nil is the default and it is load-bearing: the dependency chain costs a
// manifest search and a config read, and a run that never mentioned packages
// should behave exactly as it did before #1913.
//
// The wire carries the manifest's TEXT rather than its path, because a daemon
// on another machine can only open its own filesystem -- the same reason
// phase lines travel as source.
func (a *App) depsOptions(cwd string) *execsvc.DepsOptions {
	cfg := deps.From(a.Settings())
	if cfg.Source == deps.SourceHost && cfg.Manifest == "" && len(cfg.Packages) == 0 {
		return nil
	}
	set, err := deps.Resolve(cfg, cwd)
	if err != nil {
		return nil
	}
	out := &execsvc.DepsOptions{
		Packages:    cfg.Packages,
		Source:      string(cfg.Source),
		AutoInstall: string(cfg.AutoInstall),
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
		if name, text := m.LockfileText(); name != "" {
			out.LockfileName, out.Lockfile = name, string(text)
		}
	}
	return out
}
