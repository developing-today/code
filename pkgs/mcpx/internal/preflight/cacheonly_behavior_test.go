package preflight_test

import (
	"context"
	"os"
	"os/exec"
	"path/filepath"
	"strings"
	"testing"
	"time"

	"github.com/developing-today/code/pkgs/mcpx/internal/preflight"
)

// TestCacheOnlyGraphResolutionPermutations is the behavioural
// half of the #8457 fix, against the real deno: under
// --cached-only (deps.autoInstall=off), the import graph must
// resolve what the cache already holds — HTTPS and npm alike —
// and refuse what it does not, without downloading. The first
// fix translated the flag to --no-remote, which fails the cached
// permutations (CodeRabbit review, PR #10289). Seeding the cache
// needs the network; where it is unavailable the test skips
// rather than pretending to have run.
func TestCacheOnlyGraphResolutionPermutations(t *testing.T) {
	deno, err := exec.LookPath("deno")
	if err != nil {
		t.Skip("no deno binary on PATH")
	}
	dir := t.TempDir()
	denoDir := filepath.Join(dir, "deno-dir")
	t.Setenv("DENO_DIR", denoDir)

	write := func(name, body string) string {
		path := filepath.Join(dir, name)
		if err := os.WriteFile(path, []byte(body), 0o644); err != nil {
			t.Fatal(err)
		}
		return path
	}
	seed := write("seed.ts",
		"import \"https://deno.land/std@0.224.0/path/mod.ts\";\n"+
			"import \"npm:chalk@5\";\n")
	ctx, cancel := context.WithTimeout(context.Background(), 3*time.Minute)
	defer cancel()
	cmd := exec.CommandContext(ctx, deno, "cache", seed)
	cmd.Dir = dir
	if out, err := cmd.CombinedOutput(); err != nil {
		t.Skipf("seeding the deno cache failed (offline?): %v: %s", err, out)
	}

	cached := write("cached.ts",
		"import \"https://deno.land/std@0.224.0/path/mod.ts\";\n"+
			"import \"npm:chalk@5\";\nconsole.log(1);\n")
	rep := preflight.CheckImportsResolved(context.Background(), deno, cached,
		preflight.ImportsReport, preflight.DynamicIgnore, 2*time.Minute,
		[]string{"--cached-only"}, dir)
	if rep.Err() != nil {
		t.Fatalf("cached imports must resolve under --cached-only: %v", rep.Err())
	}

	uncached := write("uncached.ts",
		"import \"https://deno.land/std@0.224.0/http/mod.ts\";\n"+
			"import \"npm:lodash@4\";\nconsole.log(1);\n")
	rep = preflight.CheckImportsResolved(context.Background(), deno, uncached,
		preflight.ImportsReport, preflight.DynamicIgnore, 2*time.Minute,
		[]string{"--cached-only"}, dir)
	if rep.Err() == nil {
		t.Fatal("uncached imports must not resolve under --cached-only")
	}
	if msg := rep.Err().Error(); !strings.Contains(msg, "http/mod.ts") && !strings.Contains(msg, "lodash") {
		t.Fatalf("the report should name an uncached specifier: %v", rep.Err())
	}
}
