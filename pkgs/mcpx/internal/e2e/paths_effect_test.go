package e2e_test

import (
	"os"
	"path/filepath"
	"strings"
	"testing"
)

// Effect probe for paths.scripts (#8467 burn-down). The exemption
// claimed the effect appears "only when fixture content exists
// solely on that path, which the harness does not plant per-path" --
// so this probe plants per-path: the same recipe file sits in a
// directory nothing searches by default, and only the env whose
// paths.scripts names that directory can run it by name.

func plantOffPathRecipe(t *testing.T, e *env) string {
	t.Helper()
	dir := filepath.Join(e.dir, "off-path-recipes")
	if err := os.MkdirAll(dir, 0o755); err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(filepath.Join(dir, "say.ts"), []byte(sayRecipe), 0o644); err != nil {
		t.Fatal(err)
	}
	return dir
}

func TestPathsScriptsFindsRecipesThatLiveOnlyThere(t *testing.T) {
	stranded := newEnv(t, oneServer)
	plantOffPathRecipe(t, stranded)
	if out, err := stranded.try("recipes", "run", "say", "message=stranded"); err == nil {
		t.Fatalf("a recipe outside every search path should not resolve by name:\n%s", out)
	}

	found := newEnv(t, oneServer)
	dir := plantOffPathRecipe(t, found)
	found.setenv("MCPX_PATHS_SCRIPTS=" + dir)
	out, err := found.try("recipes", "run", "say", "message=found-it")
	if err != nil || !strings.Contains(out, "found-it") {
		t.Fatalf("paths.scripts should make the off-path recipe runnable by name: %v\n%s", err, out)
	}
}

func TestPathsCacheChangesTheDoctorDirectory(t *testing.T) {
	e := newEnv(t, oneServer)
	defaultCache := filepath.Join(e.dir, "cache")
	out := e.run("doctor", "-v")
	if !strings.Contains(out, defaultCache) {
		t.Fatalf("doctor should report the configured default cache directory %q:\n%s", defaultCache, out)
	}

	customCache := filepath.Join(e.dir, "alternate-cache")
	e.setenv("MCPX_CACHE_DIR=" + customCache)
	out = e.run("doctor", "-v")
	if !strings.Contains(out, customCache) || strings.Contains(out, defaultCache) {
		t.Fatalf("paths.cache should redirect doctor to %q instead of %q:\n%s", customCache, defaultCache, out)
	}
}

// Effect probe for paths.plugins (#8467 burn-down): a plugin manifest
// outside every configured discovery path is invisible to `plugin list`,
// then becomes discoverable when the user points paths.plugins at the
// directory that holds it.
func TestPathsPluginsDiscoversAPluginThatLivesOnlyThere(t *testing.T) {
	e := newEnv(t, oneServer)
	dir := filepath.Join(e.dir, "off-path-plugins")
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

	out, err := e.try("plugin", "list")
	if err != nil {
		t.Fatalf("plugin list: %v\n%s", err, out)
	}
	if strings.Contains(out, "weather") {
		t.Fatalf("a plugin outside every search path should not be discovered:\n%s", out)
	}

	e.envVars = append(e.envVars, "MCPX_PATHS_PLUGINS="+dir)
	out = e.run("plugin", "list")
	if !strings.Contains(out, "weather") {
		t.Fatalf("paths.plugins should make the off-path plugin discoverable:\n%s", out)
	}
}
