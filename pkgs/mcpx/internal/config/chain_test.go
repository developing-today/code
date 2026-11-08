package config_test

import (
	"os"
	"path/filepath"
	"strings"
	"testing"

	"github.com/developing-today/code/pkgs/mcpx/internal/config"
)

func writeChainFile(t *testing.T, path, body string) {
	t.Helper()
	if err := os.MkdirAll(filepath.Dir(path), 0o755); err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(path, []byte(body), 0o644); err != nil {
		t.Fatal(err)
	}
}

// isolateChain points HOME/XDG at a scratch tree and clears the
// bootstrap variables, so the walk and the fallbacks see only the
// fixture. It returns the home directory inside the scratch tree.
func isolateChain(t *testing.T) string {
	t.Helper()
	root := t.TempDir()
	home := filepath.Join(root, "home")
	if err := os.MkdirAll(home, 0o755); err != nil {
		t.Fatal(err)
	}
	t.Setenv("HOME", home)
	t.Setenv("XDG_CONFIG_HOME", filepath.Join(root, "xdg"))
	t.Setenv("MCPX_CONFIG", "")
	t.Setenv("MCPX_PATHS_CONFIG", "")
	config.SetPathsConfigSeed(nil)
	t.Cleanup(func() { config.SetPathsConfigSeed(nil) })
	return home
}

func TestPathsConfigExtendsChain(t *testing.T) {
	home := isolateChain(t)
	proj := filepath.Join(home, "proj")
	shared := filepath.Join(home, "shared")
	writeChainFile(t, filepath.Join(proj, ".mcpx.json"),
		`{"paths":{"config":["../shared", null]}, "mcpServers":{"near":{"command":"x"}}}`)
	writeChainFile(t, filepath.Join(shared, ".mcpx.json"),
		`{"mcpServers":{"far":{"command":"y"}}}`)

	cfg, err := config.LoadFrom("", proj)
	if err != nil {
		t.Fatal(err)
	}
	if _, ok := cfg.MCPServers["far"]; !ok {
		t.Fatalf("paths.config extension did not load: sources %v", cfg.Sources)
	}
	if cfg.Chain == nil {
		t.Fatal("load did not record the chain resolution")
	}
	sharedFile := filepath.Join(shared, ".mcpx.json")
	inLoad, inDiscovery := false, false
	for _, p := range cfg.Chain.Load {
		if p == sharedFile {
			inLoad = true
		}
	}
	for _, p := range cfg.Chain.Discovery {
		if p == sharedFile {
			inDiscovery = true
		}
	}
	if !inLoad {
		t.Errorf("added file missing from the load pass: %v", cfg.Chain.Load)
	}
	if inDiscovery {
		t.Errorf("added file should not appear in the discovery pass: %v", cfg.Chain.Discovery)
	}
	if len(cfg.Chain.Divergent) != 0 {
		t.Errorf("unexpected divergence reports: %v", cfg.Chain.Divergent)
	}
}

func TestPathsConfigCycleRefused(t *testing.T) {
	isolateChain(t)
	root := t.TempDir()
	dirA := filepath.Join(root, "a")
	dirB := filepath.Join(root, "b")
	writeChainFile(t, filepath.Join(dirA, ".mcpx.json"),
		`{"paths":{"config":[`+quoteJSON(dirB)+`]}, "mcpServers":{"a":{"command":"x"}}}`)
	writeChainFile(t, filepath.Join(dirB, ".mcpx.json"),
		`{"paths":{"config":[`+quoteJSON(dirA)+`]}, "mcpServers":{"b":{"command":"y"}}}`)
	t.Setenv("MCPX_PATHS_CONFIG", dirA+","+dirB)

	_, _, err := config.ResolveChainFrom(root)
	if err == nil {
		t.Fatal("a paths.config cycle must be refused")
	}
	if !strings.Contains(err.Error(), "cycle") {
		t.Errorf("error should name the cycle: %v", err)
	}
	if !strings.Contains(err.Error(), ".mcpx.json") {
		t.Errorf("error should name the path that closed the cycle: %v", err)
	}
}

func TestPathsConfigPass2DivergenceReportedNotApplied(t *testing.T) {
	home := isolateChain(t)
	proj := filepath.Join(home, "proj")
	added := filepath.Join(home, "added")
	other := filepath.Join(home, "other")
	writeChainFile(t, filepath.Join(proj, ".mcpx.json"),
		`{"paths":{"config":["../added", null]}, "mcpServers":{"near":{"command":"x"}}}`)
	writeChainFile(t, filepath.Join(added, ".mcpx.json"),
		`{"paths":{"config":["../other", null]}, "mcpServers":{"added":{"command":"y"}}}`)
	writeChainFile(t, filepath.Join(other, ".mcpx.json"),
		`{"mcpServers":{"other":{"command":"z"}}}`)

	cfg, err := config.LoadFrom("", proj)
	if err != nil {
		t.Fatal(err)
	}
	if _, ok := cfg.MCPServers["added"]; !ok {
		t.Fatalf("pass-1 addition did not load: %v", cfg.Sources)
	}
	if _, ok := cfg.MCPServers["other"]; ok {
		t.Errorf("pass-2 paths.config was applied; it must only be reported")
	}
	if len(cfg.Chain.Divergent) == 0 {
		t.Errorf("pass-2 divergence was not reported: %+v", cfg.Chain)
	}
}

func TestMCPXConfigBypassesBothPasses(t *testing.T) {
	home := isolateChain(t)
	proj := filepath.Join(home, "proj")
	single := filepath.Join(home, "single.json")
	writeChainFile(t, filepath.Join(proj, ".mcpx.json"),
		`{"paths":{"config":["../shared", null]}, "mcpServers":{"near":{"command":"x"}}}`)
	writeChainFile(t, filepath.Join(home, "shared", ".mcpx.json"),
		`{"mcpServers":{"far":{"command":"y"}}}`)
	writeChainFile(t, single, `{"mcpServers":{"only":{"command":"z"}}}`)
	t.Setenv("MCPX_CONFIG", single)

	cfg, err := config.LoadFrom("", proj)
	if err != nil {
		t.Fatal(err)
	}
	if len(cfg.MCPServers) != 1 {
		t.Fatalf("MCPX_CONFIG should load exactly one file, got %v", cfg.Sources)
	}
	if _, ok := cfg.MCPServers["only"]; !ok {
		t.Errorf("the MCPX_CONFIG file did not load: %v", cfg.Sources)
	}
	if cfg.Chain == nil || !cfg.Chain.Bypass {
		t.Errorf("bypass not recorded: %+v", cfg.Chain)
	}
}

func TestPathsConfigEnvSeedExtendsDiscovery(t *testing.T) {
	isolateChain(t)
	root := t.TempDir()
	seedDir := filepath.Join(root, "seeded")
	writeChainFile(t, filepath.Join(seedDir, ".mcpx.json"),
		`{"mcpServers":{"seeded":{"command":"x"}}}`)
	t.Setenv("MCPX_PATHS_CONFIG", seedDir)

	cfg, err := config.LoadFrom("", root)
	if err != nil {
		t.Fatal(err)
	}
	if _, ok := cfg.MCPServers["seeded"]; !ok {
		t.Errorf("env-seeded directory did not load: %v", cfg.Sources)
	}
	if cfg.Chain == nil || len(cfg.Chain.Seeded) != 1 {
		t.Errorf("seed not recorded: %+v", cfg.Chain)
	}
}

func TestPathsConfigWithoutNullReplacesRemainder(t *testing.T) {
	home := isolateChain(t)
	proj := filepath.Join(home, "proj")
	only := filepath.Join(home, "only")
	writeChainFile(t, filepath.Join(home, ".mcpx.json"),
		`{"mcpServers":{"homeSrv":{"command":"h"}}}`)
	writeChainFile(t, filepath.Join(proj, ".mcpx.json"),
		`{"paths":{"config":["../only"]}, "mcpServers":{"near":{"command":"x"}}}`)
	writeChainFile(t, filepath.Join(only, ".mcpx.json"),
		`{"mcpServers":{"onlySrv":{"command":"y"}}}`)

	cfg, err := config.LoadFrom("", proj)
	if err != nil {
		t.Fatal(err)
	}
	if _, ok := cfg.MCPServers["onlySrv"]; !ok {
		t.Errorf("replacement directory did not load: %v", cfg.Sources)
	}
	if _, ok := cfg.MCPServers["homeSrv"]; ok {
		t.Errorf("a list without null must replace the farther remainder, but the home file still loaded: %v", cfg.Sources)
	}
}

func quoteJSON(s string) string {
	return `"` + strings.ReplaceAll(s, `\`, `\\`) + `"`
}
