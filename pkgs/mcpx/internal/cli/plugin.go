package cli

import (
	"context"
	"crypto/sha256"
	"encoding/hex"
	"encoding/json"
	"fmt"
	"os"
	"path/filepath"
	"strings"

	"github.com/developing-today/code/pkgs/mcpx/internal/config"
	"github.com/developing-today/code/pkgs/mcpx/internal/plugin"
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
	// The socket the hosting daemon actually listens on, recorded by
	// CmdDaemon / inlineDaemon when they resolved it; Paths.Socket is
	// only the un-keyed default and would send a plugin's back-channel
	// calls to the wrong daemon (or none).
	socket := a.daemonSocket
	if socket == "" {
		socket = a.Paths.Socket
	}
	for _, p := range plugins {
		if _, dup := cfg.MCPServers[p.Name]; dup {
			return fmt.Errorf("plugin %q (from %s) has the same name as a configured server; rename one", p.Name, p.Source)
		}
		cfg.MCPServers[p.Name] = p.Server(socket, "plugin-"+p.Name)
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
			// The declaration's fingerprint rides in the environment
			// because PoolID hashes the environment: editing the
			// declaration (baseUrl, headers, spec, filters) and
			// reloading then replaces this API's pool instead of
			// silently keeping the child that was started for the old
			// declaration (Codex review, PR #2514). Adapters carry
			// the same fingerprint for the same reason since #2118
			// (augmentAdapters, MCPX_ADAPTER_DECLARATION_SHA256).
			Env:  map[string]string{"MCPX_API_DECLARATION_SHA256": apiDeclarationFingerprint(s)},
			Mcpx: &config.Extras{Description: s.Description},
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

// apiDeclarationFingerprint hashes the effective declaration of one API:
// its fields as written (headers carry ${VAR} references, never values),
// plus the spec document's bytes when the spec is a local file, so editing
// either replaces the API's pool on reload.
func apiDeclarationFingerprint(s APISpec) string {
	h := sha256.New()
	raw, err := json.Marshal(s)
	if err == nil {
		h.Write(raw)
	}
	if b, err := os.ReadFile(s.Spec); err == nil {
		h.Write([]byte{0})
		h.Write(b)
	}
	return hex.EncodeToString(h.Sum(nil))[:32]
}
