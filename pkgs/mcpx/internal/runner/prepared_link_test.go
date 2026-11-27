package runner

import (
	"bytes"
	"context"
	"encoding/json"
	"os"
	"path/filepath"
	"strings"
	"testing"
	"time"

	"github.com/developing-today/code/pkgs/mcpx/internal/preflight"
)

// makePreparedEnv builds a fake prepared environment entry in the
// shape stage 3 of #1915 leaves in the cache: the prepared manifest
// and a populated node_modules.
func makePreparedEnv(t *testing.T, root, name string) string {
	t.Helper()
	dir := filepath.Join(root, name)
	pkg := filepath.Join(dir, "node_modules", "left-pad")
	if err := os.MkdirAll(pkg, 0o755); err != nil {
		t.Fatal(err)
	}
	manifest := "{\n  \"name\": \"mcpx-prepared-imports\",\n  \"version\": \"0.0.0\",\n  \"private\": true,\n  \"dependencies\": {\"left-pad\": \"1.3.0\"}\n}\n"
	if err := os.WriteFile(filepath.Join(dir, "package.json"), []byte(manifest), 0o644); err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(filepath.Join(pkg, "index.js"), []byte("module.exports = 1\n"), 0o644); err != nil {
		t.Fatal(err)
	}
	return dir
}

// makeWorkdirSet builds a fake runner workdir holding a materialised
// dependency set: a node_modules, with the generated client beside it
// (the marker runnerOwnedLinkTarget recognises).
func makeWorkdirSet(t *testing.T, root, name string) string {
	t.Helper()
	dir := filepath.Join(root, name)
	if err := os.MkdirAll(filepath.Join(dir, "node_modules", "zod"), 0o755); err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(filepath.Join(dir, clientFileName), []byte("// client\n"), 0o644); err != nil {
		t.Fatal(err)
	}
	return dir
}

func besideTarget(t *testing.T, scriptDir string) string {
	t.Helper()
	target, err := os.Readlink(filepath.Join(scriptDir, "node_modules"))
	if err != nil {
		t.Fatalf("readlink beside the script: %v", err)
	}
	if !filepath.IsAbs(target) {
		target = filepath.Join(scriptDir, target)
	}
	return filepath.Clean(target)
}

func TestLinkPreparedNodeModulesFreshLink(t *testing.T) {
	root := t.TempDir()
	env := makePreparedEnv(t, root, "env-a")
	scriptDir := filepath.Join(root, "proj")
	if err := os.MkdirAll(scriptDir, 0o755); err != nil {
		t.Fatal(err)
	}
	if err := linkPreparedNodeModules(scriptDir, env, filepath.Join(root, "work")); err != nil {
		t.Fatal(err)
	}
	if got, want := besideTarget(t, scriptDir), filepath.Join(env, "node_modules"); got != want {
		t.Errorf("the link points at the prepared environment: got %s, want %s", got, want)
	}
	// Linking again is a no-op that keeps the same target.
	if err := linkPreparedNodeModules(scriptDir, env, filepath.Join(root, "work")); err != nil {
		t.Fatal(err)
	}
	if got, want := besideTarget(t, scriptDir), filepath.Join(env, "node_modules"); got != want {
		t.Errorf("a repeat link keeps the environment: got %s, want %s", got, want)
	}
}

func TestLinkPreparedNodeModulesLeavesTheProjectsOwn(t *testing.T) {
	root := t.TempDir()
	env := makePreparedEnv(t, root, "env-a")
	work := filepath.Join(root, "work")

	// A real node_modules directory is the project's own install.
	scriptDir := filepath.Join(root, "proj")
	if err := os.MkdirAll(filepath.Join(scriptDir, "node_modules", "mine"), 0o755); err != nil {
		t.Fatal(err)
	}
	if err := linkPreparedNodeModules(scriptDir, env, work); err != nil {
		t.Fatal(err)
	}
	if fi, err := os.Lstat(filepath.Join(scriptDir, "node_modules")); err != nil || fi.Mode()&os.ModeSymlink != 0 {
		t.Errorf("a real directory is left untouched: mode %v, err %v", fi.Mode(), err)
	}

	// A symlink the project manages -- to a live directory no mcpx
	// mechanism wrote -- is likewise left alone.
	other := filepath.Join(root, "elsewhere")
	if err := os.MkdirAll(filepath.Join(other, "node_modules"), 0o755); err != nil {
		t.Fatal(err)
	}
	scriptDir2 := filepath.Join(root, "proj2")
	if err := os.MkdirAll(scriptDir2, 0o755); err != nil {
		t.Fatal(err)
	}
	if err := os.Symlink(filepath.Join(other, "node_modules"), filepath.Join(scriptDir2, "node_modules")); err != nil {
		t.Fatal(err)
	}
	if err := linkPreparedNodeModules(scriptDir2, env, work); err != nil {
		t.Fatal(err)
	}
	if got, want := besideTarget(t, scriptDir2), filepath.Join(other, "node_modules"); got != want {
		t.Errorf("a user-managed link is left as the project put it: got %s, want %s", got, want)
	}
}

func TestLinkPreparedNodeModulesRefreshesStaleMcpxLinks(t *testing.T) {
	root := t.TempDir()
	envA := makePreparedEnv(t, root, "env-a")
	envB := makePreparedEnv(t, root, "env-b")
	work := makeWorkdirSet(t, root, "0123456789abcdef")
	workRoot := root // the workdir's parent, for runnerOwnedLinkTarget

	// An earlier prepared environment's link refreshes to the new one.
	scriptDir := filepath.Join(root, "proj")
	if err := os.MkdirAll(scriptDir, 0o755); err != nil {
		t.Fatal(err)
	}
	if err := linkPreparedNodeModules(scriptDir, envA, work); err != nil {
		t.Fatal(err)
	}
	if err := linkPreparedNodeModules(scriptDir, envB, work); err != nil {
		t.Fatal(err)
	}
	if got, want := besideTarget(t, scriptDir), filepath.Join(envB, "node_modules"); got != want {
		t.Errorf("a changed set refreshes the link: got %s, want %s", got, want)
	}

	// A link to a runner workdir's materialised set refreshes too:
	// the prepared environment is the set this run resolved.
	scriptDir2 := filepath.Join(root, "proj2")
	if err := os.MkdirAll(scriptDir2, 0o755); err != nil {
		t.Fatal(err)
	}
	if err := os.Symlink(filepath.Join(work, "node_modules"), filepath.Join(scriptDir2, "node_modules")); err != nil {
		t.Fatal(err)
	}
	if err := linkPreparedNodeModules(scriptDir2, envA, workRoot); err != nil {
		t.Fatal(err)
	}
	if got, want := besideTarget(t, scriptDir2), filepath.Join(envA, "node_modules"); got != want {
		t.Errorf("a workdir set's link refreshes to the prepared environment: got %s, want %s", got, want)
	}
}

func TestRemovePreparedLink(t *testing.T) {
	root := t.TempDir()
	env := makePreparedEnv(t, root, "env-a")
	work := makeWorkdirSet(t, root, "0123456789abcdef")

	// A prepared link is removed.
	scriptDir := filepath.Join(root, "proj")
	if err := os.MkdirAll(scriptDir, 0o755); err != nil {
		t.Fatal(err)
	}
	if err := linkPreparedNodeModules(scriptDir, env, work); err != nil {
		t.Fatal(err)
	}
	removePreparedLink(scriptDir)
	if _, err := os.Lstat(filepath.Join(scriptDir, "node_modules")); !os.IsNotExist(err) {
		t.Errorf("the prepared link is removed, stat err = %v", err)
	}

	// A workdir set's link is not the wiring's to remove.
	scriptDir2 := filepath.Join(root, "proj2")
	if err := os.MkdirAll(scriptDir2, 0o755); err != nil {
		t.Fatal(err)
	}
	if err := os.Symlink(filepath.Join(work, "node_modules"), filepath.Join(scriptDir2, "node_modules")); err != nil {
		t.Fatal(err)
	}
	removePreparedLink(scriptDir2)
	if got, want := besideTarget(t, scriptDir2), filepath.Join(work, "node_modules"); got != want {
		t.Errorf("a workdir link is left alone: got %s, want %s", got, want)
	}
}

func TestPointRunAtPreparedEnv(t *testing.T) {
	root := t.TempDir()
	env := makePreparedEnv(t, root, "env-a")
	work := filepath.Join(root, "work")
	scriptDir := filepath.Join(root, "proj")
	if err := os.MkdirAll(scriptDir, 0o755); err != nil {
		t.Fatal(err)
	}
	script := filepath.Join(scriptDir, "main.ts")
	if err := os.WriteFile(script, []byte("import 'left-pad'\n"), 0o644); err != nil {
		t.Fatal(err)
	}
	opts := Options{File: script}
	prepared := &preflight.PreparedSet{Dir: env, Key: "k", Packages: []preflight.ResolvedPackage{{Name: "left-pad", Version: "1.3.0"}}}

	// A Node file run is pointed at the environment, and releasing
	// frees the directory lock for the next run.
	release, _, err := pointRunAtPreparedEnv(context.Background(), opts, KindNode, prepared, work, false)
	if err != nil {
		t.Fatal(err)
	}
	if release == nil {
		t.Fatal("the wiring holds the lock and the lease until release")
	}
	if got, want := besideTarget(t, scriptDir), filepath.Join(env, "node_modules"); got != want {
		t.Errorf("the run resolves against the prepared environment: got %s, want %s", got, want)
	}
	release()
	release2, _, err := pointRunAtPreparedEnv(context.Background(), opts, KindNode, prepared, work, false)
	if err != nil {
		t.Fatalf("after release the next run wires again: %v", err)
	}
	release2()

	// When the caller already holds the directory lock, the wiring
	// must not take it again: with the lock held and a short
	// deadline, a second take would fail instead of wiring.
	unlock, err := lockScriptDirs(context.Background(), scriptDir)
	if err != nil {
		t.Fatal(err)
	}
	ctx, cancel := context.WithTimeout(context.Background(), 5*time.Second)
	defer cancel()
	release3, _, err := pointRunAtPreparedEnv(ctx, opts, KindNode, prepared, work, true)
	if err != nil {
		t.Fatalf("already-locked wiring: %v", err)
	}
	release3()
	unlock()

	// A run that prepared nothing clears the stale link, holding
	// only the directory lock while it runs.
	release4, _, err := pointRunAtPreparedEnv(context.Background(), opts, KindNode, nil, work, false)
	if err != nil {
		t.Fatal(err)
	}
	if release4 == nil {
		t.Fatal("a run with no environment still holds the directory lock")
	}
	release4()
	if _, err := os.Lstat(filepath.Join(scriptDir, "node_modules")); !os.IsNotExist(err) {
		t.Errorf("the stale prepared link is cleared, stat err = %v", err)
	}

	// A Deno run is wired in the import-map form instead: no
	// beside-script link, but a map rendered into the workdir, the
	// --import-map flag naming it, and the cache lease held until
	// release.
	if err := os.MkdirAll(work, 0o755); err != nil {
		t.Fatal(err)
	}
	release5, flags5, err := pointRunAtPreparedEnv(context.Background(), opts, KindDeno, prepared, work, false)
	if err != nil {
		t.Fatal(err)
	}
	if release5 == nil {
		t.Error("a deno run holds the prepared entry's lease until release")
	}
	mapPath := filepath.Join(work, preparedImportMapName)
	if len(flags5) != 2 || flags5[0] != "--import-map" || flags5[1] != mapPath {
		t.Errorf("a deno run is passed the rendered import map: got %v", flags5)
	}
	mapText, rerr := os.ReadFile(mapPath)
	if rerr != nil {
		t.Fatal(rerr)
	}
	var rendered struct {
		Imports map[string]string `json:"imports"`
	}
	if err := json.Unmarshal(mapText, &rendered); err != nil {
		t.Fatal(err)
	}
	envModules := filepath.Join(env, "node_modules", "left-pad")
	if got, want := rendered.Imports["left-pad"], "file://"+envModules+"/index.js"; got != want {
		t.Errorf("the map names the package entry point: got %s, want %s", got, want)
	}
	if got, want := rendered.Imports["left-pad/"], "file://"+envModules+"/"; got != want {
		t.Errorf("the map names the package prefix: got %s, want %s", got, want)
	}
	release5()
	if _, err := os.Lstat(filepath.Join(scriptDir, "node_modules")); !os.IsNotExist(err) {
		t.Errorf("a deno run gets no node_modules link, stat err = %v", err)
	}
}

func TestPointRunAtPreparedEnvFailsClosedOnAMissingEnvironment(t *testing.T) {
	root := t.TempDir()
	scriptDir := filepath.Join(root, "proj")
	if err := os.MkdirAll(scriptDir, 0o755); err != nil {
		t.Fatal(err)
	}
	script := filepath.Join(scriptDir, "main.ts")
	if err := os.WriteFile(script, []byte("import 'left-pad'\n"), 0o644); err != nil {
		t.Fatal(err)
	}
	// The entry exists (its manifest is there) but node_modules is
	// gone: prepare reported an environment the run cannot use.
	env := filepath.Join(root, "env-broken")
	if err := os.MkdirAll(env, 0o755); err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(filepath.Join(env, "package.json"), []byte("{\"name\": \"mcpx-prepared-imports\"}\n"), 0o644); err != nil {
		t.Fatal(err)
	}
	prepared := &preflight.PreparedSet{Dir: env, Key: "k"}
	if _, _, err := pointRunAtPreparedEnv(context.Background(), Options{File: script}, KindNode, prepared, root, false); err == nil {
		t.Fatal("a prepared environment with no node_modules must fail the run, not fall back")
	}
	if _, _, err := pointRunAtPreparedEnv(context.Background(), Options{File: script}, KindDeno, prepared, root, false); err == nil {
		t.Fatal("a prepared environment with no node_modules must fail a deno run too, not fall back")
	}
	if _, err := os.Lstat(filepath.Join(scriptDir, "node_modules")); !os.IsNotExist(err) {
		t.Errorf("no link is left by the failed wiring, stat err = %v", err)
	}
}

func TestPointRunAtPreparedEnvLinksTheManifestScope(t *testing.T) {
	root := t.TempDir()
	env := makePreparedEnv(t, root, "env-a")
	work := filepath.Join(root, "work")
	// The project: a manifest at its root and the entry script in a
	// child directory, so the manifest's scope is a parent the
	// beside-script link alone cannot cover -- a helper module in
	// the manifest's directory resolves its bare imports from there.
	manifestDir := filepath.Join(root, "proj")
	scriptDir := filepath.Join(manifestDir, "tools")
	if err := os.MkdirAll(scriptDir, 0o755); err != nil {
		t.Fatal(err)
	}
	script := filepath.Join(scriptDir, "main.ts")
	if err := os.WriteFile(script, []byte("import 'left-pad'\n"), 0o644); err != nil {
		t.Fatal(err)
	}
	opts := Options{File: script, Deps: &Deps{ManifestDir: manifestDir}}
	prepared := &preflight.PreparedSet{Dir: env, Key: "k", Packages: []preflight.ResolvedPackage{{Name: "left-pad", Version: "1.3.0"}}}

	release, _, err := pointRunAtPreparedEnv(context.Background(), opts, KindNode, prepared, work, false)
	if err != nil {
		t.Fatal(err)
	}
	if release == nil {
		t.Fatal("the wiring holds the locks and the lease until release")
	}
	if got, want := besideTarget(t, scriptDir), filepath.Join(env, "node_modules"); got != want {
		t.Errorf("the script's directory resolves against the prepared environment: got %s, want %s", got, want)
	}
	if got, want := besideTarget(t, manifestDir), filepath.Join(env, "node_modules"); got != want {
		t.Errorf("the manifest's directory resolves against the prepared environment: got %s, want %s", got, want)
	}
	release()

	// Releasing frees both directories' locks for the next run.
	release2, _, err := pointRunAtPreparedEnv(context.Background(), opts, KindNode, prepared, work, false)
	if err != nil {
		t.Fatalf("after release the next run wires both placements again: %v", err)
	}
	release2()

	// A run that prepared nothing clears the stale links at both
	// placements.
	release3, _, err := pointRunAtPreparedEnv(context.Background(), opts, KindNode, nil, work, false)
	if err != nil {
		t.Fatal(err)
	}
	release3()
	for _, dir := range []string{scriptDir, manifestDir} {
		if _, err := os.Lstat(filepath.Join(dir, "node_modules")); !os.IsNotExist(err) {
			t.Errorf("the stale prepared link at %s is cleared, stat err = %v", dir, err)
		}
	}

	// A manifest in the script's own directory adds no second
	// placement: the beside-script link is the only one, and the
	// wiring still succeeds.
	optsSame := Options{File: script, Deps: &Deps{ManifestDir: scriptDir}}
	release4, _, err := pointRunAtPreparedEnv(context.Background(), optsSame, KindNode, prepared, work, false)
	if err != nil {
		t.Fatal(err)
	}
	release4()
	if got, want := besideTarget(t, scriptDir), filepath.Join(env, "node_modules"); got != want {
		t.Errorf("a same-directory manifest keeps the beside-script placement: got %s, want %s", got, want)
	}
}

func TestPointRunAtPreparedEnvManifestScopeUnderTheDepsLock(t *testing.T) {
	root := t.TempDir()
	env := makePreparedEnv(t, root, "env-a")
	work := filepath.Join(root, "work")
	manifestDir := filepath.Join(root, "proj")
	scriptDir := filepath.Join(manifestDir, "tools")
	if err := os.MkdirAll(scriptDir, 0o755); err != nil {
		t.Fatal(err)
	}
	script := filepath.Join(scriptDir, "main.ts")
	if err := os.WriteFile(script, []byte("import 'left-pad'\n"), 0o644); err != nil {
		t.Fatal(err)
	}
	opts := Options{File: script, Deps: &Deps{ManifestDir: manifestDir}}
	prepared := &preflight.PreparedSet{Dir: env, Key: "k", Packages: []preflight.ResolvedPackage{{Name: "left-pad", Version: "1.3.0"}}}

	// The deps block locks the script's and the manifest's
	// directories together; the prepared wiring under alreadyLocked
	// must take neither again -- with both held and a short
	// deadline, a second take would fail instead of wiring.
	unlock, err := lockScriptDirs(context.Background(), scriptDir, manifestDir)
	if err != nil {
		t.Fatal(err)
	}
	ctx, cancel := context.WithTimeout(context.Background(), 5*time.Second)
	defer cancel()
	release, _, err := pointRunAtPreparedEnv(ctx, opts, KindNode, prepared, work, true)
	if err != nil {
		t.Fatalf("already-locked wiring covers the manifest scope: %v", err)
	}
	release()
	if got, want := besideTarget(t, manifestDir), filepath.Join(env, "node_modules"); got != want {
		t.Errorf("the manifest's directory is linked under the deps lock: got %s, want %s", got, want)
	}

	// Under the same held locks, a run that prepared nothing clears
	// both placements.
	if _, _, err := pointRunAtPreparedEnv(ctx, opts, KindNode, nil, work, true); err != nil {
		t.Fatalf("already-locked clearing covers the manifest scope: %v", err)
	}
	unlock()
	for _, dir := range []string{scriptDir, manifestDir} {
		if _, err := os.Lstat(filepath.Join(dir, "node_modules")); !os.IsNotExist(err) {
			t.Errorf("the stale prepared link at %s is cleared, stat err = %v", dir, err)
		}
	}
}

func TestRunUsesTheForeignProjectsPreparedSubset(t *testing.T) {
	// Oracle: Node resolves bare imports by walking up from each importing
	// file and using the nearest node_modules. This deterministic runtime
	// probe exercises that public launch path without requiring Node on the
	// Fleet runner; it also accepts Node's transform-types capability probe.
	// The entry project and foreign helper pin conflicting versions, and
	// the output reports the version found at each importing scope.
	root := t.TempDir()
	entryDir := filepath.Join(root, "entry")
	foreignDir := filepath.Join(entryDir, "foreign")
	for _, dir := range []string{entryDir, foreignDir} {
		if err := os.MkdirAll(dir, 0o755); err != nil {
			t.Fatal(err)
		}
	}
	entry := filepath.Join(entryDir, "main.mjs")
	helper := filepath.Join(foreignDir, "helper.mjs")
	if err := os.WriteFile(filepath.Join(entryDir, "package.json"), []byte("{\"dependencies\":{\"shared\":\"1.0.0\"}}\n"), 0o644); err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(filepath.Join(foreignDir, "package.json"), []byte("{\"dependencies\":{\"shared\":\"2.0.0\"}}\n"), 0o644); err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(entry, []byte("import { value as entryValue } from \"shared\";\nimport { value as foreignValue } from \"./foreign/helper.mjs\";\nconsole.log(`${entryValue}:${foreignValue}`);\n"), 0o644); err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(helper, []byte("import { value } from \"shared\";\nexport { value };\n"), 0o644); err != nil {
		t.Fatal(err)
	}

	span := map[string]any{
		"start": map[string]any{"line": 0, "character": 0},
		"end":   map[string]any{"line": 0, "character": 6},
	}
	fileEdge := map[string]any{
		"specifier": "./foreign/helper.mjs",
		"code":      map[string]any{"specifier": fileURL(helper), "span": span},
	}
	missingEdge := func() map[string]any {
		return map[string]any{
			"specifier": "shared",
			"code":      map[string]any{"error": "Module not found: shared", "span": span},
		}
	}
	graph, err := json.Marshal(map[string]any{
		"roots": []string{fileURL(entry)},
		"modules": []any{
			map[string]any{
				"kind": "esm", "specifier": fileURL(entry), "local": entry,
				"dependencies": []any{missingEdge(), fileEdge},
			},
			map[string]any{
				"kind": "esm", "specifier": fileURL(helper), "local": helper,
				"dependencies": []any{missingEdge()},
			},
		},
	})
	if err != nil {
		t.Fatal(err)
	}
	graphFile := filepath.Join(root, "deno-info.json")
	if err := os.WriteFile(graphFile, graph, 0o600); err != nil {
		t.Fatal(err)
	}

	binDir := filepath.Join(root, "bin")
	if err := os.MkdirAll(binDir, 0o755); err != nil {
		t.Fatal(err)
	}
	writeExecutable(t, filepath.Join(binDir, "deno"), "#!/bin/sh\ncat "+shellQuote(graphFile)+"\n")
	writeExecutable(t, filepath.Join(binDir, "npm"), `#!/bin/sh
set -eu
version=$(sed -n 's/.*"shared": "\([^"]*\)".*/\1/p' package.json)
case "$version" in
  1.0.0) value=entry-1.0.0 ;;
  2.0.0) value=foreign-2.0.0 ;;
  *) echo "unexpected shared pin: $version" >&2; exit 11 ;;
esac
mkdir -p node_modules/shared
printf '{"name":"shared","version":"%s","type":"module","exports":"./index.mjs"}\n' "$version" > node_modules/shared/package.json
printf 'export const value = "%s";\n' "$value" > node_modules/shared/index.mjs
`)
	writeExecutable(t, filepath.Join(binDir, "node"), `#!/bin/sh
set -eu
for arg do script=$arg; done
case "$script" in *mcpx-node-transform-probe*) exit 0 ;; esac
entry_dir=$(dirname "$script")
entry_version=$(sed -n 's/.*"version":"\([^"]*\)".*/\1/p' "$entry_dir/node_modules/shared/package.json")
foreign_version=$(sed -n 's/.*"version":"\([^"]*\)".*/\1/p' "$entry_dir/foreign/node_modules/shared/package.json")
printf 'entry-%s:foreign-%s\n' "$entry_version" "$foreign_version"
`)
	t.Setenv("PATH", binDir+string(os.PathListSeparator)+os.Getenv("PATH"))
	t.Setenv("MCPX_CACHE_DIR", filepath.Join(root, "cache"))

	var stdout, stderr bytes.Buffer
	_, err = Run(context.Background(), Options{
		File:                entry,
		Runtime:             "node",
		Permissions:         "all",
		ClientSource:        "// test client\n",
		Imports:             "prepare",
		ImportsOnMissing:    "install",
		ImportsCrossProject: "isolate",
		ImportsNetwork:      "off",
		Launcher:            LauncherNone,
		WorkRoot:            filepath.Join(root, "work"),
		Dir:                 entryDir,
		Timeout:             20 * time.Second,
		Stdout:              &stdout,
		Stderr:              &stderr,
	})
	if err != nil {
		t.Fatalf("prepare-mode runner failed: %v; stderr: %s", err, stderr.String())
	}
	if got, want := strings.TrimSpace(stdout.String()), "entry-1.0.0:foreign-2.0.0"; got != want {
		t.Fatalf("each importing project must resolve its own pins: got %q, want %q", got, want)
	}
	if _, err := os.Lstat(filepath.Join(foreignDir, "node_modules")); !os.IsNotExist(err) {
		t.Errorf("the runner removes its temporary foreign-scope link after execution: stat error = %v", err)
	}
}

func writeExecutable(t *testing.T, path, body string) {
	t.Helper()
	if err := os.WriteFile(path, []byte(body), 0o755); err != nil {
		t.Fatal(err)
	}
}

func shellQuote(value string) string {
	return "'" + strings.ReplaceAll(value, "'", "'\\''") + "'"
}
