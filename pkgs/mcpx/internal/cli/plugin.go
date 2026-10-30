package cli

import (
	"context"
	"fmt"
	"os"
	"path/filepath"
	"strings"

	"github.com/dezren39/mcpx/internal/config"
	"github.com/dezren39/mcpx/internal/plugin"
)

// loadPlugins discovers plugins from the configured paths.plugins and
// the process PATH (zero-manifest mcpx-plugin-<name> executables).
func (a *App) loadPlugins() ([]plugin.Plugin, error) {
	return plugin.Discover(splitPathList(a.Settings().String("paths.plugins")), os.Getenv("PATH"))
}

// augmentPlugins adds every discovered plugin to cfg as a server of its
// own, under the plugin's name.
//
// That is the whole of making a plugin first-class (#1870, Approach C).
// Attached as mcpserver extras, a plugin's tools would reach mcpx's own
// tools/list and nothing else -- not the pool, so not `mcpx ls`, `types`,
// `search`, `catalog`, /v1 or the generated client a script calls. As a
// server it gets all of those by the same path a configured server does,
// which is also where live add/remove lives: a reload re-runs discovery,
// so a plugin directory added or removed under paths.plugins appears or
// disappears with the reload, exactly as a servers edit does.
//
// The plugin process's environment carries the back-channel: MCPX_SOCKET
// (this daemon's socket) and MCPX_SESSION_ID, so a plugin can call /v1
// the same way plugin/opencode and an exec'd script do. The session id
// is the plugin's own: a pooled plugin is shared between callers, so the
// honest session identity for its callbacks is the plugin itself, not
// any one caller.
func (a *App) augmentPlugins(cfg *config.Config) error {
	plugins, err := a.loadPlugins()
	if err != nil || len(plugins) == 0 {
		return err
	}
	if cfg.MCPServers == nil {
		cfg.MCPServers = map[string]*config.Server{}
	}
	for _, p := range plugins {
		if _, dup := cfg.MCPServers[p.Name]; dup {
			return fmt.Errorf("plugin %q (from %s) has the same name as a configured server; rename one", p.Name, p.Source)
		}
		cfg.MCPServers[p.Name] = p.Server(a.Paths.Socket, "plugin-"+p.Name)
	}
	return nil
}

// augmentConfig is the daemon's config augmentation: adapters and
// plugins, each added as first-class servers before pools are built.
// daemon.Options.Augment takes one function, so the two live here as one.
func (a *App) augmentConfig(cfg *config.Config) error {
	if err := a.augmentAdapters(cfg); err != nil {
		return err
	}
	if err := a.augmentAPIs(cfg); err != nil {
		return err
	}
	return a.augmentPlugins(cfg)
}

// augmentAPIs adds every declared API to cfg as a server of its own:
// `mcpx api serve <name>`, under the API's name.
//
// That is the whole of making an OpenAPI-described service first-class
// (#2118, split of #1875). Attached only as mcpserver extras, an API's
// operations reached mcpx's own tools/list and nothing else -- not the
// pool, so not `mcpx ls`, `types`, `search`, `catalog`, /v1 or the
// generated client a script calls. As a server they get all of those by
// the same path a configured server does, which is also where live
// reload lives: a reload re-runs this, so an API declaration added to a
// paths.apis file appears with the reload, exactly as an adapter does.
func (a *App) augmentAPIs(cfg *config.Config) error {
	specs, err := a.loadAPIs()
	if err != nil || len(specs) == 0 {
		return err
	}
	self, err := os.Executable()
	if err != nil {
		return fmt.Errorf("apis need mcpx's own path to serve them: %w", err)
	}
	// The child reads the same files whatever its working directory.
	var files []string
	for _, p := range splitPathList(a.Settings().String("paths.apis")) {
		if p == "" {
			continue
		}
		if abs, aerr := filepath.Abs(p); aerr == nil {
			p = abs
		}
		files = append(files, p)
	}
	if cfg.MCPServers == nil {
		cfg.MCPServers = map[string]*config.Server{}
	}
	for _, s := range specs {
		if s.Name == "" {
			return fmt.Errorf("an api declaration in %s has no name; name it so it can be a namespace", strings.Join(files, ", "))
		}
		if _, dup := cfg.MCPServers[s.Name]; dup {
			return fmt.Errorf("api %q has the same name as a configured server or adapter; rename one", s.Name)
		}
		cfg.MCPServers[s.Name] = &config.Server{
			Name:    s.Name,
			Command: self,
			Args:    append([]string{"api", "serve", s.Name}, files...),
			Mcpx:    &config.Extras{Description: s.Description},
		}
	}
	return nil
}

// CmdPlugin inspects discovered plugins.
func (a *App) CmdPlugin(ctx context.Context, args []string) error {
	sub := ""
	if len(args) > 0 && !strings.HasPrefix(args[0], "-") {
		sub, args = args[0], args[1:]
	}
	fs := newFlagSet("plugin")
	if err := parseFlags(a, fs, args); err != nil {
		return err
	}
	switch sub {
	case "", "list":
		plugins, err := a.loadPlugins()
		if err != nil {
			return err
		}
		if a.JSON {
			return a.out(plugins)
		}
		if len(plugins) == 0 {
			fmt.Println("No plugins discovered. Point paths.plugins at a directory holding " +
				"a mcpx-plugin.json, or put an mcpx-plugin-<name> executable on PATH.")
			return nil
		}
		for _, p := range plugins {
			kind := "manifest"
			if p.ZeroManifest {
				kind = "zero-manifest"
			}
			target := p.Command
			if target == "" {
				target = p.URL
			}
			fmt.Printf("%-16s %-13s %s\n", p.Name, kind, target)
			if p.Description != "" {
				fmt.Printf("    %s\n", firstLine(p.Description))
			}
		}
		return nil
	}
	return fmt.Errorf("no plugin subcommand %q; list", sub)
}
