package runner

import (
	"context"
	"os"
	"path/filepath"
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
	release, err := pointRunAtPreparedEnv(context.Background(), opts, KindNode, prepared, work, false)
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
	release2, err := pointRunAtPreparedEnv(context.Background(), opts, KindNode, prepared, work, false)
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
	release3, err := pointRunAtPreparedEnv(ctx, opts, KindNode, prepared, work, true)
	if err != nil {
		t.Fatalf("already-locked wiring: %v", err)
	}
	release3()
	unlock()

	// A run that prepared nothing clears the stale link, holding
	// only the directory lock while it runs.
	release4, err := pointRunAtPreparedEnv(context.Background(), opts, KindNode, nil, work, false)
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

	// Deno is not wired by this slice: no link, no error.
	release5, err := pointRunAtPreparedEnv(context.Background(), opts, KindDeno, prepared, work, false)
	if err != nil {
		t.Fatal(err)
	}
	if release5 != nil {
		t.Error("a deno run holds nothing from this wiring")
	}
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
	if _, err := pointRunAtPreparedEnv(context.Background(), Options{File: script}, KindNode, prepared, root, false); err == nil {
		t.Fatal("a prepared environment with no node_modules must fail the run, not fall back")
	}
	if _, err := os.Lstat(filepath.Join(scriptDir, "node_modules")); !os.IsNotExist(err) {
		t.Errorf("no link is left by the failed wiring, stat err = %v", err)
	}
}
