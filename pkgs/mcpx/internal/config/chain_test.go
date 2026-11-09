package config_test

import (
	"os"
	"path/filepath"
	"strings"
	"testing"

	"github.com/developing-today/code/pkgs/mcpx/internal/config"
	"github.com/developing-today/code/pkgs/mcpx/internal/settings"
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

// PR #2477 finding: the seed's null splice was discarded — a seed of
// [null, custom] placed custom before every default location instead
// of after them.
func TestPathsConfigSeedNullSplicesAroundDefaults(t *testing.T) {
	home := isolateChain(t)
	custom := filepath.Join(t.TempDir(), "custom")
	writeChainFile(t, filepath.Join(home, ".mcpx.json"),
		`{"mcpServers":{"homeSrv":{"command":"h"}}}`)
	writeChainFile(t, filepath.Join(custom, ".mcpx.json"),
		`{"mcpServers":{"customSrv":{"command":"c"}}}`)
	t.Setenv("MCPX_PATHS_CONFIG", `[null,`+quoteJSON(custom)+`]`)

	cfg, err := config.LoadFrom("", t.TempDir())
	if err != nil {
		t.Fatal(err)
	}
	if _, ok := cfg.MCPServers["homeSrv"]; !ok {
		t.Errorf("default-walk file did not load: %v", cfg.Sources)
	}
	if _, ok := cfg.MCPServers["customSrv"]; !ok {
		t.Fatalf("seeded file did not load: %v", cfg.Sources)
	}
	homeIdx, customIdx := -1, -1
	for i, p := range cfg.Chain.Discovery {
		switch p {
		case filepath.Join(home, ".mcpx.json"):
			homeIdx = i
		case filepath.Join(custom, ".mcpx.json"):
			customIdx = i
		}
	}
	if homeIdx < 0 || customIdx < 0 {
		t.Fatalf("discovery missing an endpoint: %v", cfg.Chain.Discovery)
	}
	if customIdx < homeIdx {
		t.Errorf("seed entry after null must land FARTHER than the defaults: discovery %v", cfg.Chain.Discovery)
	}
}

// PR #2477 finding: a seed without null retained every default
// location, contradicting the registry help ("Without a null the
// list replaces outright").
func TestPathsConfigSeedWithoutNullReplacesDefaults(t *testing.T) {
	home := isolateChain(t)
	custom := filepath.Join(t.TempDir(), "custom")
	writeChainFile(t, filepath.Join(home, ".mcpx.json"),
		`{"mcpServers":{"homeSrv":{"command":"h"}}}`)
	writeChainFile(t, filepath.Join(custom, ".mcpx.json"),
		`{"mcpServers":{"customSrv":{"command":"c"}}}`)
	t.Setenv("MCPX_PATHS_CONFIG", custom)

	cfg, err := config.LoadFrom("", t.TempDir())
	if err != nil {
		t.Fatal(err)
	}
	if _, ok := cfg.MCPServers["customSrv"]; !ok {
		t.Errorf("seeded file did not load: %v", cfg.Sources)
	}
	if _, ok := cfg.MCPServers["homeSrv"]; ok {
		t.Errorf("a seed without null must replace the default walk, but the home file still loaded: %v", cfg.Sources)
	}
}

// PR #2477 finding: repeated --paths-config flags replaced the seed
// on each occurrence; the Repeatable setting must accumulate them.
func TestPathsConfigFlagSeedAccumulates(t *testing.T) {
	isolateChain(t)
	root := t.TempDir()
	dirA := filepath.Join(root, "a")
	dirB := filepath.Join(root, "b")
	writeChainFile(t, filepath.Join(dirA, ".mcpx.json"),
		`{"mcpServers":{"aSrv":{"command":"a"}}}`)
	writeChainFile(t, filepath.Join(dirB, ".mcpx.json"),
		`{"mcpServers":{"bSrv":{"command":"b"}}}`)
	config.AddPathsConfigSeed([]string{dirA})
	config.AddPathsConfigSeed([]string{dirB})

	cfg, err := config.LoadFrom("", root)
	if err != nil {
		t.Fatal(err)
	}
	if _, ok := cfg.MCPServers["aSrv"]; !ok {
		t.Errorf("first --paths-config value was lost: %v", cfg.Sources)
	}
	if _, ok := cfg.MCPServers["bSrv"]; !ok {
		t.Errorf("second --paths-config value did not load: %v", cfg.Sources)
	}
}

// PR #2477 finding: an auto-started daemon could not see the flag
// seed; the seed must be forwardable in MCPX_PATHS_CONFIG form.
func TestPathsConfigSeedEnvForwardsFlagSeed(t *testing.T) {
	isolateChain(t)
	if _, ok := config.PathsConfigSeedEnv(); ok {
		t.Fatal("no flag seed was set; nothing should be forwarded")
	}
	config.SetPathsConfigSeed([]string{"/a", settings.NullMarker, "/b"})
	env, ok := config.PathsConfigSeedEnv()
	if !ok {
		t.Fatal("flag seed set; it must be forwardable")
	}
	if env != `["/a",null,"/b"]` {
		t.Errorf("seed env form = %s, want the JSON array with a null splice", env)
	}
}

// PR #2477 finding: discovery rejected a string-valued paths.config
// that the settings layer accepts, so a previously valid config
// blocked every command once chain resolution ran before dispatch.
func TestPathsConfigStringValuedListAccepted(t *testing.T) {
	home := isolateChain(t)
	proj := filepath.Join(home, "proj")
	shared := filepath.Join(home, "shared")
	writeChainFile(t, filepath.Join(proj, ".mcpx.json"),
		`{"paths":{"config":"../shared"}, "mcpServers":{"near":{"command":"x"}}}`)
	writeChainFile(t, filepath.Join(shared, ".mcpx.json"),
		`{"mcpServers":{"far":{"command":"y"}}}`)

	cfg, err := config.LoadFrom("", proj)
	if err != nil {
		t.Fatal(err)
	}
	if _, ok := cfg.MCPServers["far"]; !ok {
		t.Errorf("string-valued paths.config did not extend the chain: %v", cfg.Sources)
	}
}

// PR #2477 finding: the cycle graph stored raw path spellings while
// membership was by file identity, so a cycle through a symlink
// alias was accepted.
func TestPathsConfigCycleThroughSymlinkAliasRefused(t *testing.T) {
	isolateChain(t)
	root := t.TempDir()
	dirA := filepath.Join(root, "a")
	dirB := filepath.Join(root, "b")
	linkA := filepath.Join(root, "link-to-a")
	writeChainFile(t, filepath.Join(dirA, ".mcpx.json"),
		`{"paths":{"config":[`+quoteJSON(dirB)+`]}, "mcpServers":{"a":{"command":"x"}}}`)
	writeChainFile(t, filepath.Join(dirB, ".mcpx.json"),
		`{"paths":{"config":[`+quoteJSON(linkA)+`]}, "mcpServers":{"b":{"command":"y"}}}`)
	if err := os.Symlink(dirA, linkA); err != nil {
		t.Fatal(err)
	}
	t.Setenv("MCPX_PATHS_CONFIG", dirA+","+dirB)

	_, _, err := config.ResolveChainFrom(root)
	if err == nil {
		t.Fatal("a paths.config cycle through a symlink alias must be refused")
	}
	if !strings.Contains(err.Error(), "cycle") {
		t.Errorf("error should name the cycle: %v", err)
	}
}

// PR #2477 finding: declarations applied in discovery order, so when
// A reordered the chain to A,C,B, C's no-null declaration then
// replaced the remainder B had already extended, dropping X.
func TestPathsConfigDeclarationsApplyInChainOrder(t *testing.T) {
	isolateChain(t)
	root := t.TempDir()
	dirA := filepath.Join(root, "a")
	dirB := filepath.Join(root, "b")
	dirC := filepath.Join(root, "c")
	dirX := filepath.Join(root, "x")
	writeChainFile(t, filepath.Join(dirA, ".mcpx.json"),
		`{"paths":{"config":[`+quoteJSON(dirC)+`,`+quoteJSON(dirB)+`]}, "mcpServers":{"aSrv":{"command":"a"}}}`)
	writeChainFile(t, filepath.Join(dirB, ".mcpx.json"),
		`{"paths":{"config":[`+quoteJSON(dirX)+`]}, "mcpServers":{"bSrv":{"command":"b"}}}`)
	writeChainFile(t, filepath.Join(dirC, ".mcpx.json"),
		`{"paths":{"config":[`+quoteJSON(dirB)+`]}, "mcpServers":{"cSrv":{"command":"c"}}}`)
	writeChainFile(t, filepath.Join(dirX, ".mcpx.json"),
		`{"mcpServers":{"xSrv":{"command":"x"}}}`)
	t.Setenv("MCPX_PATHS_CONFIG", dirA+","+dirB+","+dirC)

	cfg, err := config.LoadFrom("", root)
	if err != nil {
		t.Fatal(err)
	}
	for _, srv := range []string{"aSrv", "bSrv", "cSrv", "xSrv"} {
		if _, ok := cfg.MCPServers[srv]; !ok {
			t.Errorf("%s did not load — declarations did not apply in chain order: %v", srv, cfg.Sources)
		}
	}
	idx := map[string]int{}
	for i, p := range cfg.Chain.Load {
		for _, d := range []string{dirA, dirB, dirC, dirX} {
			if p == filepath.Join(d, ".mcpx.json") {
				idx[d] = i
			}
		}
	}
	if idx[dirA] >= idx[dirC] || idx[dirC] >= idx[dirB] || idx[dirB] >= idx[dirX] {
		t.Errorf("load order = %v, want A's file, then C, B, X", cfg.Chain.Load)
	}
}

// PR #5165 review: a nearer list inserting a symlink alias of an
// already-discovered file makes identity dedupe keep the alias
// spelling in load and drop the discovery spelling — the aliased
// file's declaration must still apply (matched by identity), or
// what it adds is lost to a pass-2 divergence report.
func TestPathsConfigAliasedFileDeclarationStillApplies(t *testing.T) {
	isolateChain(t)
	root := t.TempDir()
	dirA := filepath.Join(root, "a")
	dirB := filepath.Join(root, "b")
	dirX := filepath.Join(root, "x")
	linkB := filepath.Join(root, "link-to-b")
	writeChainFile(t, filepath.Join(dirA, ".mcpx.json"),
		`{"paths":{"config":[`+quoteJSON(linkB)+`,null]}, "mcpServers":{"aSrv":{"command":"a"}}}`)
	writeChainFile(t, filepath.Join(dirB, ".mcpx.json"),
		`{"paths":{"config":[`+quoteJSON(dirX)+`]}, "mcpServers":{"bSrv":{"command":"b"}}}`)
	writeChainFile(t, filepath.Join(dirX, ".mcpx.json"),
		`{"mcpServers":{"xSrv":{"command":"x"}}}`)
	if err := os.Symlink(dirB, linkB); err != nil {
		t.Fatal(err)
	}
	t.Setenv("MCPX_PATHS_CONFIG", dirA+","+dirB)

	cfg, err := config.LoadFrom("", root)
	if err != nil {
		t.Fatal(err)
	}
	if _, ok := cfg.MCPServers["xSrv"]; !ok {
		t.Errorf("the aliased file's declaration did not apply; X did not load: %v", cfg.Sources)
	}
	for _, d := range cfg.Chain.Divergent {
		if strings.Contains(d, "link-to-b") || strings.Contains(d, dirB) {
			t.Errorf("B's declaration was reported divergent instead of applied: %v", d)
		}
	}
}

// PR #5165 review: on the command line the splice marker is spelled
// "-", and the bootstrap flag must decode it — otherwise a seed of
// "/custom" + "-" has no null and (since the seed splice is now
// honoured) replaces the default walk instead of extending it.
func TestPathsConfigFlagSeedDashSplicesDefaults(t *testing.T) {
	home := isolateChain(t)
	custom := filepath.Join(t.TempDir(), "custom")
	writeChainFile(t, filepath.Join(home, ".mcpx.json"),
		`{"mcpServers":{"homeSrv":{"command":"h"}}}`)
	writeChainFile(t, filepath.Join(custom, ".mcpx.json"),
		`{"mcpServers":{"customSrv":{"command":"c"}}}`)
	config.AddPathsConfigSeed([]string{custom})
	config.AddPathsConfigSeed([]string{"-"})

	cfg, err := config.LoadFrom("", t.TempDir())
	if err != nil {
		t.Fatal(err)
	}
	if _, ok := cfg.MCPServers["customSrv"]; !ok {
		t.Errorf("flag-seeded file did not load: %v", cfg.Sources)
	}
	if _, ok := cfg.MCPServers["homeSrv"]; !ok {
		t.Errorf("the '-' splice must retain the default walk, but the home file did not load: %v", cfg.Sources)
	}
	env, ok := config.PathsConfigSeedEnv()
	if !ok || !strings.Contains(env, "null") {
		t.Errorf("seed env form should carry the decoded null splice, got %q (ok=%v)", env, ok)
	}
}

func quoteJSON(s string) string {
	return `"` + strings.ReplaceAll(s, `\`, `\\`) + `"`
}
