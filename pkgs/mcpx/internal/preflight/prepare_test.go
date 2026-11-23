package preflight_test

import (
	"context"
	"errors"
	"os"
	"path/filepath"
	"strings"
	"testing"
	"time"

	"github.com/developing-today/code/pkgs/mcpx/internal/preflight"
)

// fakeInstaller records what stage 3 asked it to install and stands in
// for npm: it creates the node_modules marker a real install would, so
// tests can tell a materialised entry from an empty directory.
type fakeInstaller struct {
	calls int
	dirs  []string
	pins  [][]preflight.ResolvedPackage
	err   error
}

func (f *fakeInstaller) Install(_ context.Context, dir string, pkgs []preflight.ResolvedPackage) error {
	f.calls++
	f.dirs = append(f.dirs, dir)
	f.pins = append(f.pins, append([]preflight.ResolvedPackage(nil), pkgs...))
	if f.err != nil {
		return f.err
	}
	for _, p := range pkgs {
		marker := filepath.Join(dir, "node_modules", p.Name, "package.json")
		if err := os.MkdirAll(filepath.Dir(marker), 0o755); err != nil {
			return err
		}
		if err := os.WriteFile(marker, []byte("{}\n"), 0o644); err != nil {
			return err
		}
	}
	return nil
}

func TestPrepareSetMaterialisesThenServesTheCacheHit(t *testing.T) {
	root := t.TempDir()
	inst := &fakeInstaller{}
	set := preflight.ResolvedSet{Packages: []preflight.ResolvedPackage{
		{Name: "zeta", Version: "2.0.0", Source: "explicit --package"},
		{Name: "alpha", Version: "1.2.3", Source: "explicit --package"},
	}}
	first, err := preflight.PrepareSet(context.Background(), root, set, inst)
	if err != nil {
		t.Fatal(err)
	}
	if first.Hit {
		t.Error("the first prepare of a set cannot be a cache hit")
	}
	if first.Dir == "" || first.Key == "" {
		t.Fatalf("a materialised set names its entry: %+v", first)
	}
	if filepath.Dir(first.Dir) != filepath.Join(root, "imports") || filepath.Base(first.Dir) != first.Key {
		t.Errorf("the entry lives under <root>/imports/<key>: %s", first.Dir)
	}
	manifest, err := os.ReadFile(filepath.Join(first.Dir, "package.json"))
	if err != nil {
		t.Fatal(err)
	}
	for _, want := range []string{`"alpha": "1.2.3"`, `"zeta": "2.0.0"`} {
		if !strings.Contains(string(manifest), want) {
			t.Errorf("the prepared manifest pins %s: %s", want, manifest)
		}
	}
	if _, err := os.Stat(filepath.Join(first.Dir, "node_modules", "alpha", "package.json")); err != nil {
		t.Errorf("the installer populated the entry: %v", err)
	}
	if inst.calls != 1 {
		t.Fatalf("one cold prepare installs once, got %d", inst.calls)
	}
	if got := inst.pins[0]; len(got) != 2 || got[0].Name != "alpha" || got[1].Name != "zeta" {
		t.Errorf("the installer receives the canonical (sorted) pins: %+v", got)
	}

	// The same set in a different order is the same environment: a hit,
	// and no second install.
	second, err := preflight.PrepareSet(context.Background(), root, preflight.ResolvedSet{
		Packages: []preflight.ResolvedPackage{set.Packages[1], set.Packages[0]},
	}, inst)
	if err != nil {
		t.Fatal(err)
	}
	if !second.Hit || second.Dir != first.Dir || second.Key != first.Key {
		t.Errorf("the same set must key the same entry: first %+v, second %+v", first, second)
	}
	if inst.calls != 1 {
		t.Errorf("a cache hit installs nothing again, got %d installs", inst.calls)
	}
}

func TestPrepareSetKeyFollowsTheSet(t *testing.T) {
	root := t.TempDir()
	inst := &fakeInstaller{}
	a, err := preflight.PrepareSet(context.Background(), root, preflight.ResolvedSet{
		Packages: []preflight.ResolvedPackage{{Name: "alpha", Version: "1.2.3"}},
	}, inst)
	if err != nil {
		t.Fatal(err)
	}
	b, err := preflight.PrepareSet(context.Background(), root, preflight.ResolvedSet{
		Packages: []preflight.ResolvedPackage{{Name: "alpha", Version: "1.2.4"}},
	}, inst)
	if err != nil {
		t.Fatal(err)
	}
	if a.Key == b.Key || a.Dir == b.Dir {
		t.Errorf("a different version is a different environment: %s vs %s", a.Dir, b.Dir)
	}
	if inst.calls != 2 {
		t.Errorf("each distinct set installs once, got %d", inst.calls)
	}
}

func TestPrepareSetInstallerFailureLeavesNoEntry(t *testing.T) {
	root := t.TempDir()
	inst := &fakeInstaller{err: errors.New("registry unreachable")}
	_, err := preflight.PrepareSet(context.Background(), root, preflight.ResolvedSet{
		Packages: []preflight.ResolvedPackage{{Name: "alpha", Version: "1.2.3"}},
	}, inst)
	if err == nil || !strings.Contains(err.Error(), "registry unreachable") {
		t.Fatalf("the installer's failure must surface: %v", err)
	}
	entries, rerr := os.ReadDir(filepath.Join(root, "imports"))
	if rerr != nil && !os.IsNotExist(rerr) {
		t.Fatal(rerr)
	}
	if len(entries) != 0 {
		t.Errorf("a failed install leaves no entry to be served later: %v", entries)
	}
	// And the next prepare retries rather than hitting a phantom entry.
	inst.err = nil
	prepared, err := preflight.PrepareSet(context.Background(), root, preflight.ResolvedSet{
		Packages: []preflight.ResolvedPackage{{Name: "alpha", Version: "1.2.3"}},
	}, inst)
	if err != nil {
		t.Fatal(err)
	}
	if prepared.Hit {
		t.Error("nothing was ever stored, so the retry cannot be a hit")
	}
}

func TestPrepareSetEmptySetIsANoOp(t *testing.T) {
	inst := &fakeInstaller{}
	prepared, err := preflight.PrepareSet(context.Background(), t.TempDir(), preflight.ResolvedSet{}, inst)
	if err != nil {
		t.Fatal(err)
	}
	if prepared.Dir != "" || prepared.Key != "" || len(prepared.Packages) != 0 {
		t.Errorf("an empty set materialises nothing: %+v", prepared)
	}
	if inst.calls != 0 {
		t.Error("an empty set installs nothing")
	}
}

func TestPrepareSetRefusesUnpinnedAndConflictingSets(t *testing.T) {
	inst := &fakeInstaller{}
	if _, err := preflight.PrepareSet(context.Background(), t.TempDir(), preflight.ResolvedSet{
		Packages: []preflight.ResolvedPackage{{Name: "alpha"}},
	}, inst); err == nil || !strings.Contains(err.Error(), "unpinned") {
		t.Errorf("a package with no version cannot be prepared: %v", err)
	}
	if _, err := preflight.PrepareSet(context.Background(), t.TempDir(), preflight.ResolvedSet{
		Packages: []preflight.ResolvedPackage{
			{Name: "alpha", Version: "1.0.0"},
			{Name: "alpha", Version: "2.0.0"},
		},
	}, inst); err == nil || !strings.Contains(err.Error(), "both") {
		t.Errorf("one name pinned two ways cannot be prepared: %v", err)
	}
	if _, err := preflight.PrepareSet(context.Background(), "", preflight.ResolvedSet{
		Packages: []preflight.ResolvedPackage{{Name: "alpha", Version: "1.0.0"}},
	}, inst); err == nil || !strings.Contains(err.Error(), "no cache directory") {
		t.Errorf("preparing without a cache root must say so: %v", err)
	}
	if inst.calls != 0 {
		t.Error("a refused set installs nothing")
	}
}

func TestPrepareSetCountsSubsetsWithoutMaterialisingThem(t *testing.T) {
	inst := &fakeInstaller{}
	set := preflight.ResolvedSet{
		Packages: []preflight.ResolvedPackage{{Name: "alpha", Version: "1.2.3"}},
		Subsets: []preflight.DependencySubset{{
			Manifest: preflight.PackageManifest{Path: "/foreign/package.json", Dir: "/foreign"},
			Packages: []preflight.ResolvedPackage{{Name: "foreign-only", Version: "2"}},
		}},
	}
	prepared, err := preflight.PrepareSet(context.Background(), t.TempDir(), set, inst)
	if err != nil {
		t.Fatal(err)
	}
	if prepared.Subsets != 1 {
		t.Errorf("the subset is counted for the report: %+v", prepared)
	}
	if len(inst.pins) != 1 || len(inst.pins[0]) != 1 || inst.pins[0][0].Name != "alpha" {
		t.Errorf("only the entry project's set is installed: %+v", inst.pins)
	}
}

// The graph `deno info --json` emits for a script whose one bare import
// does not resolve: the root module's dependency joins to a module entry
// carrying the error.
const denoInfoMissingBareFixture = `{
  "version": 1,
  "roots": ["file:///work/main.ts"],
  "modules": [
    {
      "kind": "esm",
      "specifier": "file:///work/main.ts",
      "dependencies": [
        {"specifier": "some-pkg", "code": {"specifier": "npm:some-pkg", "span": {"start": {"line": 0, "character": 16}, "end": {"line": 0, "character": 26}}}}
      ]
    },
    {"specifier": "npm:some-pkg", "error": "Module not found: some-pkg"}
  ],
  "redirects": {},
  "npmPackages": {}
}`

func TestCheckImportsPrepareModeMaterialisesTheResolvedSet(t *testing.T) {
	dir := t.TempDir()
	entry := filepath.Join(dir, "main.ts")
	if err := os.WriteFile(entry, []byte("import \"some-pkg\";\n"), 0o644); err != nil {
		t.Fatal(err)
	}
	stub := filepath.Join(dir, "deno-stub")
	script := "#!/bin/sh\ncat <<'JSON'\n" + denoInfoMissingBareFixture + "\nJSON\n"
	if err := os.WriteFile(stub, []byte(script), 0o755); err != nil {
		t.Fatal(err)
	}
	cacheRoot := t.TempDir()
	inst := &fakeInstaller{}
	rep := preflight.CheckImportsResolved(context.Background(), stub, entry,
		preflight.ImportsPrepare, preflight.DynamicWarn, 10*time.Second, nil, dir,
		preflight.ResolveOptions{
			Explicit:       []string{"some-pkg@1.2.3"},
			Missing:        preflight.MissingInstall,
			PrepareCache:   cacheRoot,
			PrepareInstall: inst,
		})
	if err := rep.Err(); err != nil {
		t.Fatalf("prepare mode with a resolvable set should not fail the run: %v", err)
	}
	if rep.Prepared == nil {
		t.Fatalf("prepare mode records the prepared environment: %+v", rep.Problems)
	}
	if rep.Prepared.Dir == "" || rep.Prepared.Hit {
		t.Errorf("a cold cache materialises a fresh entry: %+v", rep.Prepared)
	}
	if _, err := os.Stat(filepath.Join(rep.Prepared.Dir, "node_modules", "some-pkg", "package.json")); err != nil {
		t.Errorf("the resolved package is in the prepared environment: %v", err)
	}
	for _, p := range rep.Problems {
		if strings.Contains(p.What, "is not part of this build") {
			t.Errorf("the stage-3 stub text must be gone in prepare mode: %q", p.What)
		}
	}
	if inst.calls != 1 {
		t.Errorf("prepare installs the set once, got %d", inst.calls)
	}
}
