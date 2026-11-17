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
