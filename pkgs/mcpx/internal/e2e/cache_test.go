package e2e_test

import (
	"os"
	"path/filepath"
	"strings"
	"testing"

	"github.com/developing-today/code/pkgs/mcpx/internal/cache"
	"github.com/developing-today/code/pkgs/mcpx/internal/defaults"
)

// TestCacheCommandListsPrunesAndRemoves is the integration-test
// exercise for the cli `cache` command (#8461 burn-down, chain 1 of
// 6): it seeds a real entry through internal/cache under the env's
// cache root, then drives the command's list, prune, and remove
// surfaces through the harness — each successful invocation records
// the coverage-ledger row that lets the `cli cache` exemption go.
// The command is deliberately local (no daemon), so the exercise
// needs no server beyond the env's own setup.
func TestCacheCommandListsPrunesAndRemoves(t *testing.T) {
	e := newEnv(t, oneServer)
	c := cache.Open(filepath.Join(e.dir, "cache"))
	lim := cache.Limits{
		MaxBytes:      defaults.CacheMaxBytes,
		MaxAge:        defaults.CacheMaxAge,
		LeaseStaleAge: defaults.CacheLeaseStaleAge,
	}
	_, _, err := c.Put("catalog", "demo-entry", lim, func(dir string) error {
		return os.WriteFile(filepath.Join(dir, "payload.txt"), []byte("cached payload"), 0o600)
	})
	if err != nil {
		t.Fatalf("seed cache entry: %v", err)
	}

	out := e.run("cache")
	if !strings.Contains(out, "catalog") || !strings.Contains(out, "demo-entry") {
		t.Fatalf("cache listing does not show the seeded entry:\n%s", out)
	}

	out = e.run("cache", "prune")
	if !strings.Contains(out, "catalog: pruned 0 entr(ies)") {
		t.Fatalf("cache prune did not report the in-bounds kind untouched:\n%s", out)
	}
	if _, ok := c.Get("catalog", "demo-entry"); !ok {
		t.Fatal("cache prune removed an entry inside the configured bounds")
	}

	out = e.run("cache", "remove", "catalog", "demo-entry")
	if !strings.Contains(out, "removed catalog/demo-entry") {
		t.Fatalf("cache remove did not report the removal:\n%s", out)
	}
	if _, ok := c.Get("catalog", "demo-entry"); ok {
		t.Fatal("cache remove left the entry in place")
	}

	out = e.run("cache")
	if !strings.Contains(out, "cache: empty") {
		t.Fatalf("cache listing after remove is not the empty report:\n%s", out)
	}
}
