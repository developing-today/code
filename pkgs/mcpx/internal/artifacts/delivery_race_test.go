package artifacts_test

import (
	"fmt"
	"os"
	"path/filepath"
	"strings"
	"sync"
	"testing"

	"github.com/developing-today/code/pkgs/mcpx/internal/artifacts"
)

// TestClaimPathHoldsTheNameUntilTheWriteIsDone is the #4253 interleaving,
// deterministically: while the first claim is still held open (the write
// in flight), a second claim for the same name must be handed the
// suffixed name -- the resolve-then-rename composition this replaces
// handed both callers the same path, and the second rename silently
// destroyed the first delivery.
func TestClaimPathHoldsTheNameUntilTheWriteIsDone(t *testing.T) {
	dir := t.TempDir()

	first, claim, err := artifacts.ClaimPath(dir, "shot.png")
	if err != nil {
		t.Fatal(err)
	}
	if filepath.Base(first) != "shot.png" {
		t.Fatalf("first claim = %q, want shot.png", first)
	}

	second, claim2, err := artifacts.ClaimPath(dir, "shot.png")
	if err != nil {
		t.Fatal(err)
	}
	if filepath.Base(second) != "shot-1.png" {
		t.Fatalf("second claim while the first is held = %q, want shot-1.png", second)
	}

	if _, err := claim.Write([]byte("FIRST")); err != nil {
		t.Fatal(err)
	}
	if _, err := claim2.Write([]byte("SECOND")); err != nil {
		t.Fatal(err)
	}
	if err := claim.Close(); err != nil {
		t.Fatal(err)
	}
	if err := claim2.Close(); err != nil {
		t.Fatal(err)
	}
	for path, want := range map[string]string{first: "FIRST", second: "SECOND"} {
		got, err := os.ReadFile(path)
		if err != nil {
			t.Fatal(err)
		}
		if string(got) != want {
			t.Fatalf("%s holds %q, want %q", path, got, want)
		}
	}

	// A delivered file keeps the name taken after the claim is closed:
	// the next claim still suffixes rather than overwriting it.
	third, claim3, err := artifacts.ClaimPath(dir, "shot.png")
	if err != nil {
		t.Fatal(err)
	}
	defer func() { _ = claim3.Close() }()
	if filepath.Base(third) != "shot-2.png" {
		t.Fatalf("third claim = %q, want shot-2.png", third)
	}
}

// TestExportConcurrentSameNameDeliveriesAllSurvive runs the concurrent
// scenario uniquePath's own comment names -- two runs of the same script
// exporting the same name at the same time -- at class width: every
// delivery survives under a distinct path, with its own bytes.
func TestExportConcurrentSameNameDeliveriesAllSurvive(t *testing.T) {
	s := open(t, artifacts.Options{})
	out := t.TempDir()

	const n = 8
	metas := make([]artifacts.Meta, n)
	for i := 0; i < n; i++ {
		m, err := s.Put(strings.NewReader(fmt.Sprintf("body-%d", i)), artifacts.PutOptions{Name: "shot.png"})
		if err != nil {
			t.Fatal(err)
		}
		metas[i] = m
	}

	paths := make([]string, n)
	errs := make([]error, n)
	var wg sync.WaitGroup
	for i := 0; i < n; i++ {
		wg.Add(1)
		go func(i int) {
			defer wg.Done()
			paths[i], errs[i] = s.Export(metas[i], out)
		}(i)
	}
	wg.Wait()

	seen := map[string]bool{}
	for i := 0; i < n; i++ {
		if errs[i] != nil {
			t.Fatalf("export %d: %v", i, errs[i])
		}
		if seen[paths[i]] {
			t.Fatalf("export %d reused %s; a concurrent delivery was lost", i, paths[i])
		}
		seen[paths[i]] = true
		got, err := os.ReadFile(paths[i])
		if err != nil {
			t.Fatal(err)
		}
		if want := fmt.Sprintf("body-%d", i); string(got) != want {
			t.Fatalf("%s holds %q, want %q", paths[i], got, want)
		}
	}
}

// TestExportKeepsAForeignFileThatHoldsTheName: a file that already sits
// at the delivery name -- placed by anything, at any time before the
// delivery -- keeps its bytes; the delivery suffixes instead.
func TestExportKeepsAForeignFileThatHoldsTheName(t *testing.T) {
	s := open(t, artifacts.Options{})
	out := t.TempDir()
	foreign := filepath.Join(out, "shot.png")
	if err := os.WriteFile(foreign, []byte("FOREIGN"), 0o600); err != nil {
		t.Fatal(err)
	}
	m, err := s.Put(strings.NewReader("DELIVERED"), artifacts.PutOptions{Name: "shot.png"})
	if err != nil {
		t.Fatal(err)
	}
	p, err := s.Export(m, out)
	if err != nil {
		t.Fatal(err)
	}
	if filepath.Base(p) != "shot-1.png" {
		t.Fatalf("export landed at %q, want shot-1.png", p)
	}
	got, err := os.ReadFile(foreign)
	if err != nil {
		t.Fatal(err)
	}
	if string(got) != "FOREIGN" {
		t.Fatalf("foreign file holds %q; the delivery overwrote it", got)
	}
}

// TestDeliveryPathsHoldTheirClaims is the class guard for #4253: every
// delivery path (Store.Export in this package; the streaming and fetched
// paths in internal/cli) must deliver through the held ClaimPath claim.
// The racy resolve API and rename-onto-destination must not come back.
func TestDeliveryPathsHoldTheirClaims(t *testing.T) {
	files := []string{
		"internal/artifacts/artifacts.go",
		"internal/artifacts/name.go",
		"internal/cli/exec.go",
	}
	for _, name := range files {
		contents, err := os.ReadFile(filepath.Join("..", "..", filepath.FromSlash(name)))
		if err != nil {
			t.Fatalf("read %s: %v", name, err)
		}
		if strings.Contains(string(contents), "UniquePath") {
			t.Errorf("%s references UniquePath, the resolve-then-deliver API removed by #4253", name)
		}
	}
	exec, err := os.ReadFile(filepath.Join("..", "..", "internal/cli/exec.go"))
	if err != nil {
		t.Fatal(err)
	}
	if strings.Contains(string(exec), "os.Rename(") {
		t.Errorf("internal/cli/exec.go renames onto a delivery destination; delivery must write into the held claim (#4253)")
	}
}
