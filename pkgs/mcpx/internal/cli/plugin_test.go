package cli

import (
	"os"
	"path/filepath"
	"strings"
	"testing"

	"github.com/dezren39/mcpx/internal/config"
	"github.com/dezren39/mcpx/internal/daemon"
	"github.com/dezren39/mcpx/internal/testsupport"
)

// pluginApp builds an App whose paths.plugins names dir, with PATH
// emptied so zero-manifest discovery on the host's real PATH cannot leak
// into the test.
func pluginApp(t *testing.T, pluginDir string) *App {
	t.Helper()
	root := testsupport.TempDir(t)
	t.Setenv("HOME", root)
	t.Setenv("XDG_CONFIG_HOME", filepath.Join(root, "xdg"))
	t.Setenv("PATH", filepath.Join(root, "emptybin"))
	t.Setenv("MCPX_PATHS_PLUGINS", pluginDir)
	cfg := filepath.Join(root, "mcpx.json")
	if err := os.WriteFile(cfg, []byte(`{"mcpServers":{}}`), 0o600); err != nil {
		t.Fatal(err)
	}
	return &App{ConfigPath: cfg, Paths: daemon.PathsAt(root, "")}
}

func writePlugin(t *testing.T, dir string) {
	t.Helper()
	pd := filepath.Join(dir, "weather")
	if err := os.MkdirAll(pd, 0o755); err != nil {
		t.Fatal(err)
	}
	manifest := `{"name":"weather","description":"forecasts","command":"./weather-server"}`
	if err := os.WriteFile(filepath.Join(pd, "mcpx-plugin.json"), []byte(manifest), 0o644); err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(filepath.Join(pd, "weather-server"), []byte("#!/bin/sh\n"), 0o755); err != nil {
		t.Fatal(err)
	}
}

// A discovered plugin becomes a server in the config the daemon builds
// pools from -- the pool-routing requirement of #1870, not an extra.
func TestAugmentPluginsAddsServerAndPool(t *testing.T) {
	dir := t.TempDir()
	writePlugin(t, dir)
	a := pluginApp(t, dir)

	cfg := &config.Config{MCPServers: map[string]*config.Server{}}
	if err := a.augmentPlugins(cfg); err != nil {
		t.Fatal(err)
	}
	s, ok := cfg.MCPServers["weather"]
	if !ok {
		t.Fatalf("plugin not added as a server: %v", cfg.MCPServers)
	}
	if !strings.HasSuffix(s.Command, filepath.Join("weather", "weather-server")) {
		t.Fatalf("unexpected plugin command %q", s.Command)
	}
	if s.Env["MCPX_SOCKET"] != a.Paths.Socket {
		t.Fatalf("plugin env lacks the daemon socket: %v (socket %q)", s.Env, a.Paths.Socket)
	}
	if s.Env["MCPX_SESSION_ID"] != "plugin-weather" {
		t.Fatalf("plugin env lacks its session id: %v", s.Env)
	}

	reg, err := daemon.NewRegistry(cfg, a.Paths, nil)
	if err != nil {
		t.Fatal(err)
	}
	if _, ok := reg.Pool("weather"); !ok {
		t.Fatalf("plugin has no pool; registry names: %v", reg.Names())
	}
}

func TestAugmentPluginsNameCollisionWithServerIsAnError(t *testing.T) {
	dir := t.TempDir()
	writePlugin(t, dir)
	a := pluginApp(t, dir)

	cfg := &config.Config{MCPServers: map[string]*config.Server{
		"weather": {Name: "weather", Command: "/bin/true"},
	}}
	err := a.augmentPlugins(cfg)
	if err == nil || !strings.Contains(err.Error(), "same name as a configured server") {
		t.Fatalf("want collision error, got %v", err)
	}
}

func TestAugmentPluginsNoneConfiguredIsQuiet(t *testing.T) {
	a := pluginApp(t, "")
	cfg := &config.Config{MCPServers: map[string]*config.Server{}}
	if err := a.augmentPlugins(cfg); err != nil {
		t.Fatal(err)
	}
	if len(cfg.MCPServers) != 0 {
		t.Fatalf("no plugins should mean no servers, got %v", cfg.MCPServers)
	}
}

// Regression (Codex P1 on #2481): the socket handed to a plugin server
// is the one the hosting daemon resolved (config-keyed or inline
// private), not the un-keyed Paths default.
func TestAugmentPluginsUsesDaemonSocket(t *testing.T) {
	dir := t.TempDir()
	writePlugin(t, dir)
	a := pluginApp(t, dir)
	a.daemonSocket = filepath.Join(dir, "keyed.sock")

	cfg := &config.Config{MCPServers: map[string]*config.Server{}}
	if err := a.augmentPlugins(cfg); err != nil {
		t.Fatal(err)
	}
	s := cfg.MCPServers["weather"]
	if s == nil {
		t.Fatalf("plugin not added: %v", cfg.MCPServers)
	}
	if s.Env["MCPX_SOCKET"] != a.daemonSocket {
		t.Fatalf("plugin got socket %q, want the daemon's resolved %q", s.Env["MCPX_SOCKET"], a.daemonSocket)
	}
	if s.Env["MCPX_SOCKET"] == a.Paths.Socket {
		t.Fatal("plugin got the un-keyed Paths.Socket default")
	}
}
