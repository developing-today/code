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
	calls      int
	dirs       []string
	pins       [][]preflight.ResolvedPackage
	err        error
	failOnCall int
}

func (f *fakeInstaller) Install(_ context.Context, dir string, pkgs []preflight.ResolvedPackage) error {
	f.calls++
	f.dirs = append(f.dirs, dir)
	f.pins = append(f.pins, append([]preflight.ResolvedPackage(nil), pkgs...))
	if f.err != nil && (f.failOnCall == 0 || f.calls == f.failOnCall) {
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
	t.Cleanup(first.ReleaseWarmLeases)
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
	t.Cleanup(second.ReleaseWarmLeases)
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
	t.Cleanup(a.ReleaseWarmLeases)
	b, err := preflight.PrepareSet(context.Background(), root, preflight.ResolvedSet{
		Packages: []preflight.ResolvedPackage{{Name: "alpha", Version: "1.2.4"}},
	}, inst)
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(b.ReleaseWarmLeases)
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
	t.Cleanup(prepared.ReleaseWarmLeases)
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

func TestPrepareSetMaterialisesEachProjectSetSeparately(t *testing.T) {
	// Oracle: the resolved entry set and each isolated foreign subset
	// must become separate prepared environments in the mcpx cache. The
	// injected installer reports exactly the pins it receives; neither
	// project's pins may leak into the other's environment, and no write
	// may land beside the source manifests.
	root := t.TempDir()
	entryDir := filepath.Join(root, "entry")
	foreignDir := filepath.Join(root, "foreign")
	for _, dir := range []string{entryDir, foreignDir} {
		if err := os.MkdirAll(dir, 0o755); err != nil {
			t.Fatal(err)
		}
	}
	inst := &fakeInstaller{}
	set := preflight.ResolvedSet{
		Packages: []preflight.ResolvedPackage{{Name: "alpha", Version: "1.2.3"}},
		Subsets: []preflight.DependencySubset{{
			Manifest: preflight.PackageManifest{Path: filepath.Join(foreignDir, "package.json"), Dir: foreignDir},
			Packages: []preflight.ResolvedPackage{{Name: "foreign-only", Version: "2.0.0"}},
		}},
	}
	prepared, err := preflight.PrepareSet(context.Background(), filepath.Join(root, "cache"), set, inst)
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(prepared.ReleaseWarmLeases)
	if prepared.Subsets != 1 {
		t.Errorf("the report retains one cross-project subset: %+v", prepared)
	}
	if len(prepared.PreparedSubsets) != 1 {
		t.Fatalf("one foreign cache entry is prepared separately: %+v", prepared)
	}
	subset := prepared.PreparedSubsets[0]
	if got, want := subset.ManifestDir, foreignDir; got != want {
		t.Errorf("the prepared subset keeps its source project scope: got %q, want %q", got, want)
	}
	if subset.ManifestPath != filepath.Join(foreignDir, "package.json") {
		t.Errorf("the prepared subset records the project manifest identity: %+v", subset)
	}
	if subset.Dir == prepared.Dir || subset.Key == prepared.Key {
		t.Errorf("entry and foreign sets have separate cache identities: entry=%+v subset=%+v", prepared, subset)
	}
	if _, err := os.Stat(filepath.Join(subset.Dir, "node_modules", "foreign-only", "package.json")); err != nil {
		t.Errorf("the foreign pin is present in its own cache entry: %v", err)
	}
	if _, err := os.Stat(filepath.Join(prepared.Dir, "node_modules", "foreign-only")); !os.IsNotExist(err) {
		t.Errorf("the foreign pin does not leak into the entry cache: stat error = %v", err)
	}
	if _, err := os.Stat(filepath.Join(subset.Dir, "node_modules", "alpha")); !os.IsNotExist(err) {
		t.Errorf("the entry pin does not leak into the foreign cache: stat error = %v", err)
	}
	if len(inst.pins) != 2 {
		t.Fatalf("entry and foreign project are installed separately; got %d installer calls with pins %+v", len(inst.pins), inst.pins)
	}
	if len(inst.pins[0]) != 1 || inst.pins[0][0].Name != "alpha" {
		t.Errorf("first cache entry contains only the entry project's pin: %+v", inst.pins[0])
	}
	if len(inst.pins[1]) != 1 || inst.pins[1][0].Name != "foreign-only" {
		t.Errorf("second cache entry contains only the foreign project's pin: %+v", inst.pins[1])
	}
	if inst.dirs[0] == inst.dirs[1] {
		t.Errorf("different project sets get distinct cache entries: %q", inst.dirs[0])
	}
	for _, dir := range []string{entryDir, foreignDir} {
		if _, err := os.Lstat(filepath.Join(dir, "node_modules")); !os.IsNotExist(err) {
			t.Errorf("prepare must not write into project directory %s; Lstat error = %v", dir, err)
		}
	}
}

func TestPrepareSetKeysIdenticalPinsByProjectIdentity(t *testing.T) {
	root := t.TempDir()
	cacheRoot := filepath.Join(root, "cache")
	projectA := filepath.Join(root, "project-a")
	projectB := filepath.Join(root, "project-b")
	for _, dir := range []string{projectA, projectB} {
		if err := os.MkdirAll(dir, 0o755); err != nil {
			t.Fatal(err)
		}
	}
	inst := &fakeInstaller{}
	set := preflight.ResolvedSet{Subsets: []preflight.DependencySubset{
		{
			Manifest: preflight.PackageManifest{Path: filepath.Join(projectA, "package.json"), Dir: projectA},
			Packages: []preflight.ResolvedPackage{{Name: "shared", Version: "1.0.0"}},
		},
		{
			Manifest: preflight.PackageManifest{Path: filepath.Join(projectB, "package.json"), Dir: projectB},
			Packages: []preflight.ResolvedPackage{{Name: "shared", Version: "1.0.0"}},
		},
	}}
	first, err := preflight.PrepareSet(context.Background(), cacheRoot, set, inst)
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(first.ReleaseWarmLeases)
	if len(first.PreparedSubsets) != 2 {
		t.Fatalf("each foreign project has a prepared subset: %+v", first)
	}
	if first.PreparedSubsets[0].Key == first.PreparedSubsets[1].Key || first.PreparedSubsets[0].Dir == first.PreparedSubsets[1].Dir {
		t.Fatalf("identical pins from distinct projects must not collide: %+v", first.PreparedSubsets)
	}
	if inst.calls != 2 {
		t.Fatalf("both project identities are installed once, got %d calls", inst.calls)
	}

	second, err := preflight.PrepareSet(context.Background(), cacheRoot, set, inst)
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(second.ReleaseWarmLeases)
	if len(second.PreparedSubsets) != 2 || !second.PreparedSubsets[0].Hit || !second.PreparedSubsets[1].Hit {
		t.Fatalf("same project identities and pins reuse both entries: %+v", second.PreparedSubsets)
	}
	if inst.calls != 2 {
		t.Errorf("subset cache hits do not reinstall packages, got %d calls", inst.calls)
	}
}

func TestPrepareSetSubsetInstallerFailureLeavesNoPartialSubsetEntry(t *testing.T) {
	root := t.TempDir()
	cacheRoot := filepath.Join(root, "cache")
	foreignDir := filepath.Join(root, "foreign")
	if err := os.MkdirAll(foreignDir, 0o755); err != nil {
		t.Fatal(err)
	}
	inst := &fakeInstaller{err: errors.New("foreign registry unavailable"), failOnCall: 2}
	_, err := preflight.PrepareSet(context.Background(), cacheRoot, preflight.ResolvedSet{
		Packages: []preflight.ResolvedPackage{{Name: "entry-only", Version: "1.0.0"}},
		Subsets: []preflight.DependencySubset{{
			Manifest: preflight.PackageManifest{Path: filepath.Join(foreignDir, "package.json"), Dir: foreignDir},
			Packages: []preflight.ResolvedPackage{{Name: "foreign-only", Version: "2.0.0"}},
		}},
	}, inst)
	if err == nil || !strings.Contains(err.Error(), "foreign registry unavailable") {
		t.Fatalf("a subset installer failure must fail prepare: %v", err)
	}
	entries, err := os.ReadDir(filepath.Join(cacheRoot, "imports"))
	if err != nil {
		t.Fatal(err)
	}
	if len(entries) != 1 || !entries[0].IsDir() || strings.HasPrefix(entries[0].Name(), ".tmp-") {
		t.Fatalf("the completed entry set remains whole and the failed subset leaves no partial cache entry: %v", entries)
	}
	if _, err := os.Lstat(filepath.Join(foreignDir, "node_modules")); !os.IsNotExist(err) {
		t.Errorf("a failed subset preparation never writes to the foreign project: %v", err)
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
	t.Cleanup(rep.Prepared.ReleaseWarmLeases)
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

func TestIsPreparedNodeModules(t *testing.T) {
	root := t.TempDir()
	inst := &fakeInstaller{}
	set := preflight.ResolvedSet{Packages: []preflight.ResolvedPackage{
		{Name: "alpha", Version: "1.2.3", Source: "explicit --package"},
	}}
	prepared, err := preflight.PrepareSet(context.Background(), root, set, inst)
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(prepared.ReleaseWarmLeases)
	if !preflight.IsPreparedNodeModules(filepath.Join(prepared.Dir, "node_modules")) {
		t.Error("a materialised entry's node_modules is recognised as prepared")
	}

	// A directory whose manifest carries any other name is not a
	// prepared environment, and neither is a path with no manifest.
	other := filepath.Join(root, "other")
	if err := os.MkdirAll(filepath.Join(other, "node_modules"), 0o755); err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(filepath.Join(other, "package.json"), []byte("{\"name\": \"some-project\"}\n"), 0o644); err != nil {
		t.Fatal(err)
	}
	if preflight.IsPreparedNodeModules(filepath.Join(other, "node_modules")) {
		t.Error("another project's node_modules is not a prepared environment")
	}
	if preflight.IsPreparedNodeModules(filepath.Join(root, "missing", "node_modules")) {
		t.Error("a missing path is not a prepared environment")
	}
}
