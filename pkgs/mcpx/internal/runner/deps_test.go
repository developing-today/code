package runner

import (
	"context"
	"encoding/json"
	"os"
	"path/filepath"
	"strings"
	"sync"
	"testing"
	"time"
)

func readJSON(t *testing.T, path string) map[string]any {
	t.Helper()
	b, err := os.ReadFile(path)
	if err != nil {
		t.Fatal(err)
	}
	var m map[string]any
	if err := json.Unmarshal(b, &m); err != nil {
		t.Fatalf("%s is not JSON: %v", path, err)
	}
	return m
}

func TestMaterialiseDepsSynthesisesManifestForBun(t *testing.T) {
	dir := t.TempDir()
	flags, err := materialiseDeps(dir, KindBun, &Deps{
		Packages:    []string{"lodash@4.17.21", "@scope/pkg", "zod@^3.0.0"},
		AutoInstall: "off",
	}, false, "")
	if err != nil {
		t.Fatal(err)
	}
	m := readJSON(t, filepath.Join(dir, "package.json"))
	deps, ok := m["dependencies"].(map[string]any)
	if !ok {
		t.Fatalf("package.json has no dependencies: %v", m)
	}
	if deps["lodash"] != "4.17.21" || deps["@scope/pkg"] != "*" || deps["zod"] != "^3.0.0" {
		t.Fatalf("dependencies = %v", deps)
	}
	if len(flags) != 1 || flags[0] != "--no-install" {
		t.Fatalf("flags = %v, want [--no-install] for autoInstall off", flags)
	}
}

func TestMaterialiseDepsManifestWinsAndLockfileVerbatim(t *testing.T) {
	dir := t.TempDir()
	lock := []byte("lock-bytes\x00\x01not-utf8")
	flags, err := materialiseDeps(dir, KindBun, &Deps{
		Packages:     []string{"lodash@9.9.9", "extra@1.0.0"},
		Manifest:     `{"name":"proj","dependencies":{"lodash":"4.17.21"}}`,
		ManifestName: "package.json",
		Lockfile:     lock,
		LockfileName: "bun.lockb",
		AutoInstall:  "off",
	}, false, "")
	if err != nil {
		t.Fatal(err)
	}
	m := readJSON(t, filepath.Join(dir, "package.json"))
	deps := m["dependencies"].(map[string]any)
	if deps["lodash"] != "4.17.21" {
		t.Fatalf("manifest version lost to explicit pin: %v", deps)
	}
	if deps["extra"] != "1.0.0" {
		t.Fatalf("explicit package not merged: %v", deps)
	}
	got, err := os.ReadFile(filepath.Join(dir, "bun.lockb"))
	if err != nil {
		t.Fatal(err)
	}
	if string(got) != string(lock) {
		t.Fatalf("lockfile rewritten: %q != %q", got, lock)
	}
	// Frozen beats the autoInstall mapping for bun: the frozen
	// install step (installFrozenBun) populates the workdir, and the
	// run goes --no-install so the runtime cannot re-resolve past
	// the lockfile.
	if len(flags) != 1 || flags[0] != "--no-install" {
		t.Fatalf("flags = %v, want [--no-install] for a frozen bun set", flags)
	}
}

func TestMaterialiseDepsDenoImportsMap(t *testing.T) {
	dir := t.TempDir()
	flags, err := materialiseDeps(dir, KindDeno, &Deps{
		Packages: []string{"zod@^3.0.0", "lodash"},
	}, false, "")
	if err != nil {
		t.Fatal(err)
	}
	m := readJSON(t, filepath.Join(dir, "deno.json"))
	imports, ok := m["imports"].(map[string]any)
	if !ok {
		t.Fatalf("deno.json has no imports: %v", m)
	}
	if imports["zod"] != "npm:zod@^3.0.0" || imports["lodash"] != "npm:lodash" {
		t.Fatalf("imports = %v", imports)
	}
	if len(flags) != 0 {
		t.Fatalf("flags = %v, want none without a deno.lock", flags)
	}
}

func TestMaterialiseDepsLocalArtifactResolvesAgainstManifestDir(t *testing.T) {
	// Codex review, PR #2589: discovery found the manifest ABOVE
	// the script, and its file: artifact resolved against the
	// script's directory instead of the manifest's, so the
	// transport looked for a file that was never there (ENOENT).
	project := t.TempDir()
	manifestDir := filepath.Join(project, "app")
	scriptDir := filepath.Join(manifestDir, "scripts")
	if err := os.MkdirAll(scriptDir, 0o755); err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(filepath.Join(manifestDir, "local.tgz"), []byte("artifact-bytes"), 0o644); err != nil {
		t.Fatal(err)
	}
	workdir := t.TempDir()
	_, err := materialiseDeps(workdir, KindNode, &Deps{
		Manifest:     `{"name":"proj","dependencies":{"local":"file:./local.tgz"}}`,
		ManifestName: "package.json",
		ManifestDir:  manifestDir,
		AutoInstall:  "off",
	}, true, scriptDir)
	if err != nil {
		t.Fatal(err)
	}
	got, err := os.ReadFile(filepath.Join(workdir, "local.tgz"))
	if err != nil {
		t.Fatalf("the artifact was not transported from the manifest directory: %v", err)
	}
	if string(got) != "artifact-bytes" {
		t.Fatalf("transported artifact = %q", got)
	}
}

func TestWorkDirKeyDiffersByLocalArtifactContent(t *testing.T) {
	// Codex review, PR #2589: two projects with identical dep
	// payloads but different content behind file:./local.tgz
	// selected the same cached workdir, and the second transport
	// overwrote the first project's installed package. The
	// artifact's content is part of the workdir identity.
	base := t.TempDir()
	mk := func(content string) string {
		if err := os.WriteFile(filepath.Join(base, "local.tgz"), []byte(content), 0o644); err != nil {
			t.Fatal(err)
		}
		opts := Options{Deps: &Deps{
			Manifest:     `{"name":"proj","dependencies":{"local":"file:./local.tgz"}}`,
			ManifestName: "package.json",
			ManifestDir:  base,
		}}
		return workDirKeyMaterial(KindNode, opts)
	}
	a1, a2, b := mk("content-A"), mk("content-A"), mk("content-B")
	if a1 != a2 {
		t.Errorf("the same artifact at the same base must share a workdir key")
	}
	if a1 == b {
		t.Errorf("different artifact content must not share a workdir key")
	}
	// The resolution base is part of the identity too (the
	// finding's terms): the same bytes behind a file: ref in a
	// different project are a different artifact source.
	other := t.TempDir()
	if err := os.WriteFile(filepath.Join(other, "local.tgz"), []byte("content-A"), 0o644); err != nil {
		t.Fatal(err)
	}
	opts := Options{Deps: &Deps{
		Manifest:     `{"name":"proj","dependencies":{"local":"file:./local.tgz"}}`,
		ManifestName: "package.json",
		ManifestDir:  other,
	}}
	if workDirKeyMaterial(KindNode, opts) == a1 {
		t.Errorf("a different resolution base must not share a workdir key")
	}
}

func TestDepsFlagsMatrix(t *testing.T) {
	cases := []struct {
		kind string
		deps *Deps
		want string
	}{
		{KindBun, &Deps{AutoInstall: "fallback"}, "--install=fallback"},
		{KindBun, &Deps{AutoInstall: "always"}, "--install=auto"},
		{KindBun, &Deps{AutoInstall: "off"}, "--no-install"},
		{KindBun, &Deps{}, "--no-install"},
		{KindDeno, &Deps{Lockfile: []byte("x"), LockfileName: "deno.lock"}, "--frozen"},
		{KindNode, &Deps{AutoInstall: "always"}, ""},
	}
	for _, c := range cases {
		got := depsFlags(c.kind, c.deps)
		if c.want == "" {
			if len(got) != 0 {
				t.Errorf("%s %+v: flags = %v, want none", c.kind, c.deps, got)
			}
			continue
		}
		if len(got) != 1 || got[0] != c.want {
			t.Errorf("%s %+v: flags = %v, want [%s]", c.kind, c.deps, got, c.want)
		}
	}
}

func TestMaterialiseDepsRejectsBadManifest(t *testing.T) {
	dir := t.TempDir()
	if _, err := materialiseDeps(dir, KindNode, &Deps{Manifest: "{nope"}, false, ""); err == nil {
		t.Fatal("invalid manifest accepted")
	}
	if _, err := os.Stat(filepath.Join(dir, "package.json")); !os.IsNotExist(err) {
		t.Fatal("a package.json was written for an invalid manifest")
	}
}

func TestFrozenNPM(t *testing.T) {
	d := &Deps{Lockfile: []byte("x"), LockfileName: "package-lock.json"}
	if !d.frozenNPM(KindNode) {
		t.Fatal("node + package-lock.json should be a frozen npm set")
	}
	ds := &Deps{Lockfile: []byte("x"), LockfileName: "npm-shrinkwrap.json"}
	if !ds.frozenNPM(KindNode) {
		t.Fatal("node + npm-shrinkwrap.json should be a frozen npm set (npm ci accepts either)")
	}
	if d.frozenNPM(KindBun) || d.frozenNPM(KindDeno) {
		t.Fatal("frozen npm is node-only")
	}
	if (&Deps{}).frozenNPM(KindNode) {
		t.Fatal("no lockfile is not frozen")
	}
}

func TestSplitSpecifier(t *testing.T) {
	cases := []struct{ spec, name, ver string }{
		{"lodash", "lodash", ""},
		{"lodash@4.17.21", "lodash", "4.17.21"},
		{"@scope/pkg", "@scope/pkg", ""},
		{"@scope/pkg@1.0", "@scope/pkg", "1.0"},
		{"zod@^3.0.0", "zod", "^3.0.0"},
		{"alias@npm:@scope/pkg@2.0.0", "alias", "npm:@scope/pkg@2.0.0"},
		{"alias@npm:lodash@4.17.21", "alias", "npm:lodash@4.17.21"},
	}
	for _, c := range cases {
		name, ver := splitSpecifier(c.spec)
		if name != c.name || ver != c.ver {
			t.Errorf("splitSpecifier(%q) = %q, %q; want %q, %q", c.spec, name, ver, c.name, c.ver)
		}
	}
}

// TestMaterialiseDepsAliasKeepsNpmSpecifier is the Codex finding on
// PR #2589 (runner/deps.go): an npm alias was split at the last "@",
// so the materialised dependency key was the corrupt
// "alias@npm:@scope/pkg" instead of the alias name.
func TestMaterialiseDepsAliasKeepsNpmSpecifier(t *testing.T) {
	dir := t.TempDir()
	if _, err := materialiseDeps(dir, KindBun, &Deps{
		Packages:    []string{"alias@npm:@scope/pkg@2.0.0"},
		AutoInstall: "off",
	}, false, ""); err != nil {
		t.Fatal(err)
	}
	m := readJSON(t, filepath.Join(dir, "package.json"))
	deps := m["dependencies"].(map[string]any)
	if deps["alias"] != "npm:@scope/pkg@2.0.0" || len(deps) != 1 {
		t.Fatalf("dependencies = %v, want {alias: npm:@scope/pkg@2.0.0}", deps)
	}
}

// TestMaterialiseDepsDenoAliasUsesNpmTarget is the Codex finding on
// PR #2589 (runner/deps.go): the Deno imports generation concatenated
// an npm alias behind its own name, producing
// "npm:alias@npm:@scope/pkg@2.0.0" instead of the alias's target.
func TestMaterialiseDepsDenoAliasUsesNpmTarget(t *testing.T) {
	dir := t.TempDir()
	if _, err := materialiseDeps(dir, KindDeno, &Deps{
		Packages: []string{"alias@npm:@scope/pkg@2.0.0"},
	}, false, ""); err != nil {
		t.Fatal(err)
	}
	m := readJSON(t, filepath.Join(dir, "deno.json"))
	imports, ok := m["imports"].(map[string]any)
	if !ok {
		t.Fatalf("deno.json has no imports: %v", m)
	}
	if imports["alias"] != "npm:@scope/pkg@2.0.0" {
		t.Fatalf("imports = %v, want alias -> npm:@scope/pkg@2.0.0", imports)
	}
}

// TestMaterialiseDepsKeepsDevDependenciesInTheirSection is the Codex
// finding on PR #2589 (runner/deps.go): a resolved dev dependency was
// promoted into "dependencies" when the manifest was materialised,
// changing the manifest a bun.lock was frozen against.
func TestMaterialiseDepsKeepsDevDependenciesInTheirSection(t *testing.T) {
	dir := t.TempDir()
	if _, err := materialiseDeps(dir, KindBun, &Deps{
		Manifest:    `{"name":"p","dependencies":{"a":"^1.0.0"},"devDependencies":{"tool":"^2.0.0"}}`,
		Packages:    []string{"tool@2.1.0"},
		AutoInstall: "off",
	}, false, ""); err != nil {
		t.Fatal(err)
	}
	m := readJSON(t, filepath.Join(dir, "package.json"))
	deps := m["dependencies"].(map[string]any)
	if _, promoted := deps["tool"]; promoted {
		t.Fatalf("dev dependency promoted into dependencies: %v", deps)
	}
	dev := m["devDependencies"].(map[string]any)
	if dev["tool"] != "2.1.0" {
		t.Fatalf("devDependencies = %v, want the governed pin 2.1.0", dev)
	}
}

// TestRebaseDenoMappingEscapesPathCharacters is the Codex finding on
// PR #2589 (runner/deps.go): the project path was concatenated into a
// file URL unescaped, so a directory containing '#' or a space changed
// the mapping's target.
func TestRebaseDenoMappingEscapesPathCharacters(t *testing.T) {
	if got := rebaseDenoMapping("./helper.ts", "/repo/project#one"); got != "file:///repo/project%23one/helper.ts" {
		t.Errorf("got %q", got)
	}
	if got := rebaseDenoMapping("./lib/", "/repo/a b"); got != "file:///repo/a%20b/lib/" {
		t.Errorf("prefix mapping: got %q", got)
	}
}

// TestMaterialiseDepsTransportsLocalFileDependencies is the Codex
// finding on PR #2589 (runner/deps.go): a relative file: requirement
// was copied into the workdir manifest unchanged while its artifact
// stayed in the project, so a frozen npm ci resolved it against the
// workdir and failed with ENOENT.
func TestMaterialiseDepsTransportsLocalFileDependencies(t *testing.T) {
	project := t.TempDir()
	if err := os.WriteFile(filepath.Join(project, "local-dep.tgz"), []byte("tarball-bytes"), 0o644); err != nil {
		t.Fatal(err)
	}
	if err := os.MkdirAll(filepath.Join(project, "vendor", "linked"), 0o755); err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(filepath.Join(project, "vendor", "linked", "index.js"), []byte("export {}"), 0o644); err != nil {
		t.Fatal(err)
	}
	dir := t.TempDir()
	if _, err := materialiseDeps(dir, KindNode, &Deps{
		Manifest: `{"name":"p","dependencies":{"foo":"file:./local-dep.tgz","bar":"link:./vendor/linked"}}`,
	}, true, project); err != nil {
		t.Fatal(err)
	}
	got, err := os.ReadFile(filepath.Join(dir, "local-dep.tgz"))
	if err != nil || string(got) != "tarball-bytes" {
		t.Fatalf("tarball not transported: %q, %v", got, err)
	}
	got, err = os.ReadFile(filepath.Join(dir, "vendor", "linked", "index.js"))
	if err != nil || string(got) != "export {}" {
		t.Fatalf("linked directory not transported: %q, %v", got, err)
	}
}

// TestSelectedLockfileFollowsTheRuntime is the Codex finding on
// PR #2589 (deps/chain.go): with both lockfiles present, discovery's
// first (bun.lock) was transported and used for every runtime, so a
// node run never saw its package-lock.json and never froze.
func TestSelectedLockfileFollowsTheRuntime(t *testing.T) {
	d := &Deps{Lockfiles: []Lockfile{
		{Name: "bun.lock", Content: []byte("bun")},
		{Name: "package-lock.json", Content: []byte("npm")},
	}}
	if name, content, ok := d.selectedLockfile(KindNode); !ok || name != "package-lock.json" || string(content) != "npm" {
		t.Fatalf("node selected %q %q %v", name, content, ok)
	}
	if name, _, ok := d.selectedLockfile(KindBun); !ok || name != "bun.lock" {
		t.Fatalf("bun selected %q %v", name, ok)
	}
	if !d.frozenNPM(KindNode) {
		t.Fatal("node with a transported package-lock.json must be a frozen npm set")
	}
	// Materialising a node set writes the npm lockfile, not bun's.
	dir := t.TempDir()
	if _, err := materialiseDeps(dir, KindNode, d, false, ""); err != nil {
		t.Fatal(err)
	}
	if _, err := os.Stat(filepath.Join(dir, "package-lock.json")); err != nil {
		t.Fatalf("package-lock.json not materialised: %v", err)
	}
	if _, err := os.Stat(filepath.Join(dir, "bun.lock")); !os.IsNotExist(err) {
		t.Fatal("bun.lock materialised for a node run")
	}
}

func TestDepsFingerprintSeparatesSets(t *testing.T) {
	a := (&Deps{Packages: []string{"foo@1.0.0"}}).fingerprint()
	b := (&Deps{Packages: []string{"foo@2.0.0"}}).fingerprint()
	c := (&Deps{Packages: []string{"foo@1.0.0"}}).fingerprint()
	if a == "" || a == b {
		t.Fatalf("fingerprints must differ across sets: %q vs %q", a, b)
	}
	if a != c {
		t.Fatal("identical sets must fingerprint alike (the workdir cache depends on it)")
	}
	var nilDeps *Deps
	if nilDeps.fingerprint() != "" {
		t.Fatal("a nil set fingerprints as no set")
	}
}

// TestLinkNodeModules is the Codex finding on PR #2589 (runner.go):
// installed deps sat in the exec workdir while a file script resolved
// imports from its own directory, so the install changed nothing.
func TestMaterialiseDepsStripsScriptsAndKeepsESM(t *testing.T) {
	dir := t.TempDir()
	d := &Deps{Manifest: `{"name":"p","type":"commonjs","scripts":{"prepare":"node ./scripts/prepare.js","preinstall":"x"},"dependencies":{"foo":"1.0.0"}}`}
	if _, err := materialiseDeps(dir, KindNode, d, false, ""); err != nil {
		t.Fatal(err)
	}
	b, err := os.ReadFile(filepath.Join(dir, "package.json"))
	if err != nil {
		t.Fatal(err)
	}
	var got map[string]any
	if err := json.Unmarshal(b, &got); err != nil {
		t.Fatal(err)
	}
	if _, ok := got["scripts"]; ok {
		t.Fatal("materialised manifest still carries the project's lifecycle scripts")
	}
	if got["type"] != "module" {
		t.Fatalf("materialised manifest type = %v, want module (generated files are ESM)", got["type"])
	}
}

func TestLinkNodeModules(t *testing.T) {
	work := t.TempDir()
	if err := os.MkdirAll(filepath.Join(work, "node_modules", "foo"), 0o755); err != nil {
		t.Fatal(err)
	}
	scriptDir := t.TempDir()
	if err := linkNodeModules(scriptDir, work); err != nil {
		t.Fatal(err)
	}
	got, err := os.Readlink(filepath.Join(scriptDir, "node_modules"))
	if err != nil || got != filepath.Join(work, "node_modules") {
		t.Fatalf("link = %q, %v", got, err)
	}
	// A project's own node_modules wins untouched.
	own := t.TempDir()
	if err := os.MkdirAll(filepath.Join(own, "node_modules"), 0o755); err != nil {
		t.Fatal(err)
	}
	if err := linkNodeModules(own, work); err != nil {
		t.Fatal(err)
	}
	if _, err := os.Readlink(filepath.Join(own, "node_modules")); err == nil {
		t.Fatal("an existing node_modules directory was replaced by a link")
	}
	// Nothing installed in the workdir: no link, no error.
	if err := linkNodeModules(t.TempDir(), t.TempDir()); err != nil {
		t.Fatal(err)
	}
	// A stale runner-owned link (an earlier workdir, recognisable by
	// the generated client beside its node_modules) is refreshed to
	// the current set.
	oldWork := t.TempDir()
	if err := os.MkdirAll(filepath.Join(oldWork, "node_modules"), 0o755); err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(filepath.Join(oldWork, clientFileName), []byte("// client"), 0o600); err != nil {
		t.Fatal(err)
	}
	proj := t.TempDir()
	if err := os.Symlink(filepath.Join(oldWork, "node_modules"), filepath.Join(proj, "node_modules")); err != nil {
		t.Fatal(err)
	}
	if err := linkNodeModules(proj, work); err != nil {
		t.Fatal(err)
	}
	if got, err := os.Readlink(filepath.Join(proj, "node_modules")); err != nil || got != filepath.Join(work, "node_modules") {
		t.Fatalf("stale runner link = %q, %v; want refreshed to the current set", got, err)
	}
	// A live symlink the runner did not leave (no generated client
	// beside its target) is the project's own and wins untouched.
	foreign := t.TempDir()
	if err := os.MkdirAll(filepath.Join(foreign, "node_modules"), 0o755); err != nil {
		t.Fatal(err)
	}
	proj2 := t.TempDir()
	if err := os.Symlink(filepath.Join(foreign, "node_modules"), filepath.Join(proj2, "node_modules")); err != nil {
		t.Fatal(err)
	}
	if err := linkNodeModules(proj2, work); err != nil {
		t.Fatal(err)
	}
	if got, _ := os.Readlink(filepath.Join(proj2, "node_modules")); got != filepath.Join(foreign, "node_modules") {
		t.Fatalf("foreign link = %q; want untouched", got)
	}
}

func TestInstallFrozenNPMInstallsOncePerSet(t *testing.T) {
	dir := t.TempDir()
	binDir := t.TempDir()
	calls := filepath.Join(binDir, "calls")
	stub := "#!/bin/sh\n" +
		"echo x >> \"" + calls + "\"\n" +
		"prev=\n" +
		"for a in \"$@\"; do\n" +
		"  if [ \"$prev\" = \"--prefix\" ]; then mkdir -p \"$a/node_modules\"; fi\n" +
		"  prev=\"$a\"\n" +
		"done\n"
	if err := os.WriteFile(filepath.Join(binDir, "npm"), []byte(stub), 0o755); err != nil {
		t.Fatal(err)
	}
	t.Setenv("PATH", binDir+string(os.PathListSeparator)+os.Getenv("PATH"))

	d := &Deps{Lockfile: []byte("locked"), LockfileName: "package-lock.json"}
	ctx := context.Background()
	var wg sync.WaitGroup
	errs := make(chan error, 4)
	for i := 0; i < 4; i++ {
		wg.Add(1)
		go func() {
			defer wg.Done()
			errs <- installFrozenNPM(ctx, dir, d)
		}()
	}
	wg.Wait()
	close(errs)
	for err := range errs {
		if err != nil {
			t.Fatal(err)
		}
	}
	b, err := os.ReadFile(calls)
	if err != nil {
		t.Fatal(err)
	}
	if n := len(strings.Fields(string(b))); n != 1 {
		t.Fatalf("npm ci ran %d times for 4 concurrent runs of one set, want 1", n)
	}
	// A different set installs again.
	d2 := &Deps{Lockfile: []byte("locked-v2"), LockfileName: "package-lock.json"}
	if err := installFrozenNPM(ctx, dir, d2); err != nil {
		t.Fatal(err)
	}
	b, _ = os.ReadFile(calls)
	if n := len(strings.Fields(string(b))); n != 2 {
		t.Fatalf("npm ci ran %d times after the set changed, want 2", n)
	}
}

func TestMaterialiseDepsCompatiblePinGovernsOverManifestRange(t *testing.T) {
	// Codex review, PR #2589: foo@1.3.0 satisfies the manifest's
	// ^1.0.0, so resolution accepts it -- and the materialised set
	// must then carry the pin, not the range, or the run executes a
	// version the caller did not pin.
	dir := t.TempDir()
	_, err := materialiseDeps(dir, KindBun, &Deps{
		Packages:     []string{"foo@1.3.0"},
		Manifest:     `{"name":"proj","dependencies":{"foo":"^1.0.0"}}`,
		ManifestName: "package.json",
		AutoInstall:  "off",
	}, false, "")
	if err != nil {
		t.Fatal(err)
	}
	m := readJSON(t, filepath.Join(dir, "package.json"))
	deps := m["dependencies"].(map[string]any)
	if deps["foo"] != "1.3.0" {
		t.Fatalf("compatible explicit pin lost to the manifest range: %v", deps)
	}

	dir2 := t.TempDir()
	if _, err := materialiseDeps(dir2, KindDeno, &Deps{
		Packages:     []string{"foo@1.3.0"},
		Manifest:     `{"name":"proj","dependencies":{"foo":"^1.0.0"}}`,
		ManifestName: "package.json",
	}, false, ""); err != nil {
		t.Fatal(err)
	}
	dj := readJSON(t, filepath.Join(dir2, "deno.json"))
	imports := dj["imports"].(map[string]any)
	if imports["foo"] != "npm:foo@1.3.0" {
		t.Fatalf("deno imports map lost the explicit pin: %v", imports)
	}
}

func TestLockScriptDirSerializes(t *testing.T) {
	dir := t.TempDir()
	unlock, err := lockScriptDir(context.Background(), dir)
	if err != nil {
		t.Fatal(err)
	}
	acquired := make(chan func(), 1)
	go func() {
		u, lerr := lockScriptDir(context.Background(), dir)
		if lerr == nil {
			acquired <- u
		}
	}()
	select {
	case <-acquired:
		t.Fatal("second lock on the same script dir acquired while the first was held")
	case <-time.After(100 * time.Millisecond):
	}
	unlock()
	select {
	case second := <-acquired:
		second()
	case <-time.After(5 * time.Second):
		t.Fatal("second lock on the same script dir never acquired after release")
	}
	// A different directory is not serialized against this one.
	other, oerr := lockScriptDir(context.Background(), t.TempDir())
	if oerr != nil {
		t.Fatal(oerr)
	}
	other()
}

func TestMaterialiseDepsCanonicalisesSelectedManifestName(t *testing.T) {
	dir := t.TempDir()
	_, err := materialiseDeps(dir, KindNode, &Deps{
		Manifest:     `{"name":"proj","dependencies":{"foo":"^1.0.0"}}`,
		ManifestName: "alt-manifest.json",
	}, false, "")
	if err != nil {
		t.Fatal(err)
	}
	if _, err := os.Stat(filepath.Join(dir, "package.json")); err != nil {
		t.Fatalf("materialised manifest is not package.json: %v", err)
	}
	if _, err := os.Stat(filepath.Join(dir, "alt-manifest.json")); !os.IsNotExist(err) {
		t.Fatalf("selected basename was preserved in the workdir: %v", err)
	}
}

func TestMaterialiseDepsDenoFileRunGetsConfigFlag(t *testing.T) {
	dir := t.TempDir()
	flags, err := materialiseDeps(dir, KindDeno, &Deps{
		Packages: []string{"zod@^3.0.0"},
	}, true, "")
	if err != nil {
		t.Fatal(err)
	}
	want := filepath.Join(dir, "deno.json")
	found := false
	for i, f := range flags {
		if f == "--config" && i+1 < len(flags) && flags[i+1] == want {
			found = true
		}
	}
	if !found {
		t.Fatalf("flags = %v, want --config %s for a deno file run", flags, want)
	}

	flags, err = materialiseDeps(dir, KindDeno, &Deps{
		Packages: []string{"zod@^3.0.0"},
	}, false, "")
	if err != nil {
		t.Fatal(err)
	}
	for _, f := range flags {
		if f == "--config" {
			t.Fatalf("flags = %v, inline runs discover the workdir config and must not get --config", flags)
		}
	}
}

func TestLockScriptDirHonoursCancellation(t *testing.T) {
	dir := t.TempDir()
	unlock, err := lockScriptDir(context.Background(), dir)
	if err != nil {
		t.Fatal(err)
	}
	defer unlock()
	ctx, cancel := context.WithCancel(context.Background())
	done := make(chan error, 1)
	go func() {
		_, lerr := lockScriptDir(ctx, dir)
		done <- lerr
	}()
	select {
	case lerr := <-done:
		t.Fatalf("lock acquired or failed before cancellation: %v", lerr)
	case <-time.After(100 * time.Millisecond):
	}
	cancel()
	select {
	case lerr := <-done:
		if lerr == nil {
			t.Fatal("cancelled lock acquisition returned no error")
		}
	case <-time.After(5 * time.Second):
		t.Fatal("cancelled lock acquisition never returned")
	}
}

func TestMaterialiseDepsDenoPreservesProjectConfig(t *testing.T) {
	project := t.TempDir()
	if err := os.WriteFile(filepath.Join(project, "deno.json"), []byte(`{
		"imports": {
			"@app/": "./src/",
			"zod": "npm:zod@2.0.0",
			"lib": "jsr:@scope/lib@1"
		},
		"scopes": {
			"./tools/": {"helper": "./helpers/h.ts"}
		}
	}`), 0o644); err != nil {
		t.Fatal(err)
	}
	scriptDir := filepath.Join(project, "scripts")
	if err := os.MkdirAll(scriptDir, 0o755); err != nil {
		t.Fatal(err)
	}
	dir := t.TempDir()
	if _, err := materialiseDeps(dir, KindDeno, &Deps{
		Packages: []string{"zod@3.0.0"},
	}, true, scriptDir); err != nil {
		t.Fatal(err)
	}
	dj := readJSON(t, filepath.Join(dir, "deno.json"))
	imports := dj["imports"].(map[string]any)
	wantAlias := "file://" + filepath.ToSlash(filepath.Join(project, "src")) + "/"
	if imports["@app/"] != wantAlias {
		t.Fatalf("project alias not preserved/rebased: %v (want %q)", imports, wantAlias)
	}
	if imports["lib"] != "jsr:@scope/lib@1" {
		t.Fatalf("project JSR mapping not preserved: %v", imports)
	}
	if imports["zod"] != "npm:zod@3.0.0" {
		t.Fatalf("resolved pin did not govern over the project mapping: %v", imports)
	}
	scopes := dj["scopes"].(map[string]any)
	// The scope key is rebased against the project directory too, or
	// it would name a directory inside the workdir and never match.
	wantScope := "file://" + filepath.ToSlash(filepath.Join(project, "tools")) + "/"
	toolScope, ok := scopes[wantScope].(map[string]any)
	if !ok {
		t.Fatalf("scope key not rebased to %q: %v", wantScope, scopes)
	}
	wantHelper := "file://" + filepath.ToSlash(filepath.Join(project, "helpers", "h.ts"))
	if toolScope["helper"] != wantHelper {
		t.Fatalf("scope mapping not preserved/rebased: %v (want %q)", scopes, wantHelper)
	}
}

// Codex round (PR #2589): the workdir identity must cover the project
// Deno configuration the generated config carries, or two projects
// with identical dependency payloads would share one workdir and
// overwrite each other's generated deno.json.
func TestProjectDenoConfigIdentityDistinguishesProjects(t *testing.T) {
	write := func(dir, alias string) {
		t.Helper()
		body := `{"imports": {"@app/": "` + alias + `"}}`
		if err := os.WriteFile(filepath.Join(dir, "deno.json"), []byte(body), 0o644); err != nil {
			t.Fatal(err)
		}
	}
	a, b, none := t.TempDir(), t.TempDir(), t.TempDir()
	write(a, "./src-a/")
	write(b, "./src-b/")
	ida, idb := projectDenoConfigIdentity(a), projectDenoConfigIdentity(b)
	if ida == "" || idb == "" {
		t.Fatalf("projects with configs must have identities, got %q and %q", ida, idb)
	}
	if ida == idb {
		t.Fatalf("different project configs produced one identity %q", ida)
	}
	if again := projectDenoConfigIdentity(a); again != ida {
		t.Fatalf("identity not stable: %q then %q", ida, again)
	}
	if got := projectDenoConfigIdentity(none); got != "" {
		t.Fatalf("a project with no config must have empty identity, got %q", got)
	}
}

// Codex round (PR #2589): the generated Deno config replaces the
// project's, so its compilerOptions — jsx/jsxImportSource above all —
// must be carried into it, or a .tsx run silently falls back to the
// classic JSX transform.
func TestMaterialiseDepsDenoPreservesProjectCompilerOptions(t *testing.T) {
	project := t.TempDir()
	if err := os.WriteFile(filepath.Join(project, "deno.json"), []byte(`{
		"imports": {"@app/": "./src/"},
		"compilerOptions": {"jsx": "react-jsx", "jsxImportSource": "preact", "strict": true}
	}`), 0o644); err != nil {
		t.Fatal(err)
	}
	dir := t.TempDir()
	if _, err := materialiseDeps(dir, KindDeno, &Deps{
		Packages: []string{"zod@3.0.0"},
	}, false, project); err != nil {
		t.Fatal(err)
	}
	dj := readJSON(t, filepath.Join(dir, "deno.json"))
	co, ok := dj["compilerOptions"].(map[string]any)
	if !ok {
		t.Fatalf("generated deno.json dropped the project compilerOptions: %v", dj)
	}
	if co["jsx"] != "react-jsx" || co["jsxImportSource"] != "preact" || co["strict"] != true {
		t.Fatalf("compilerOptions not preserved verbatim: %v", co)
	}
}

// Codex round (PR #2589): waiting for another run's install must
// observe cancellation, like lockScriptDir already does.
func TestLockInstallDirHonoursCancellation(t *testing.T) {
	dir := t.TempDir()
	unlock, err := lockInstallDir(context.Background(), dir)
	if err != nil {
		t.Fatal(err)
	}
	defer unlock()
	ctx, cancel := context.WithCancel(context.Background())
	done := make(chan error, 1)
	go func() {
		_, lerr := lockInstallDir(ctx, dir)
		done <- lerr
	}()
	select {
	case lerr := <-done:
		t.Fatalf("install lock acquired or failed before cancellation: %v", lerr)
	case <-time.After(100 * time.Millisecond):
	}
	cancel()
	select {
	case lerr := <-done:
		if lerr == nil {
			t.Fatal("cancelled install-lock acquisition returned no error")
		}
	case <-time.After(5 * time.Second):
		t.Fatal("cancelled install-lock acquisition never returned")
	}
}

func TestFrozenBunFollowsTheLockfile(t *testing.T) {
	if !(&Deps{Lockfiles: []Lockfile{{Name: "bun.lock", Content: []byte("x")}}}).frozenBun(KindBun) {
		t.Fatal("a bun run carrying bun.lock must be frozen")
	}
	if (&Deps{Lockfiles: []Lockfile{{Name: "package-lock.json", Content: []byte("x")}}}).frozenBun(KindBun) {
		t.Fatal("an npm lockfile alone must not freeze a bun run")
	}
	if (&Deps{Lockfiles: []Lockfile{{Name: "bun.lock", Content: []byte("x")}}}).frozenBun(KindNode) {
		t.Fatal("frozenBun is bun-only")
	}
}

// TestShrinkwrapPreferredOverPackageLock verifies that npm-shrinkwrap takes
// precedence over package-lock.json (npm documented precedence), per the
// Codex review on PR #2589.
func TestShrinkwrapPreferredOverPackageLock(t *testing.T) {
	d := &Deps{Lockfiles: []Lockfile{
		{Name: "package-lock.json", Content: []byte("npm")},
		{Name: "npm-shrinkwrap.json", Content: []byte("shrinkwrap")},
	}}
	name, content, ok := d.selectedLockfile(KindNode)
	if !ok || name != "npm-shrinkwrap.json" || string(content) != "shrinkwrap" {
		t.Fatalf("node selected %q %q %v, want npm-shrinkwrap.json", name, content, ok)
	}
	if !d.frozenNPM(KindNode) {
		t.Fatal("node with a transported npm-shrinkwrap.json must be a frozen npm set")
	}
	// Materialising a node set writes the shrinkwrap lockfile, not package-lock.
	dir := t.TempDir()
	if _, err := materialiseDeps(dir, KindNode, d, false, ""); err != nil {
		t.Fatal(err)
	}
	if _, err := os.Stat(filepath.Join(dir, "npm-shrinkwrap.json")); err != nil {
		t.Fatalf("npm-shrinkwrap.json not materialised: %v", err)
	}
	if _, err := os.Stat(filepath.Join(dir, "package-lock.json")); !os.IsNotExist(err) {
		t.Fatal("package-lock.json materialised for a node run when shrinkwrap is preferred")
	}
}

// TestMaterialiseDepsOptionalOverrides ensures that optionalDependencies take
// precedence over dependencies of the same name in the materialised manifest,
// and that compatible explicit pins update both maps rather than stranding
// one (Codex review, PR #2589).
func TestMaterialiseDepsOptionalOverrides(t *testing.T) {
	dir := t.TempDir()
	if _, err := materialiseDeps(dir, KindBun, &Deps{
		Packages:    []string{"shared@2.0.0"},
		Manifest:    `{"name":"proj","dependencies":{"shared":"1.0.0"},"optionalDependencies":{"shared":"2.0.0"}}`,
		AutoInstall: "off",
	}, false, ""); err != nil {
		t.Fatal(err)
	}
	m := readJSON(t, filepath.Join(dir, "package.json"))
	deps := m["dependencies"].(map[string]any)
	optDeps := m["optionalDependencies"].(map[string]any)
	if deps["shared"] != "2.0.0" || optDeps["shared"] != "2.0.0" {
		t.Fatalf("optional override lost: dependencies=%v optionalDependencies=%v", deps, optDeps)
	}
}

// TestMaterialiseSiblingLocalDependency is the Codex finding on
// PR #2589 (runner/deps.go): a project declaring a sibling dependency
// (file:../localpkg) was rejected with "escapes its base directory"
// even though the artifact exists and the project's locked install
// works in place. The artifact must be transported into the workdir
// under a rebased spelling and the materialised manifest must name
// that spelling.
func TestMaterialiseSiblingLocalDependency(t *testing.T) {
	root := t.TempDir()
	project := filepath.Join(root, "proj")
	sibling := filepath.Join(root, "localpkg")
	workdir := filepath.Join(root, "work")
	for _, d := range []string{project, sibling, workdir} {
		if err := os.MkdirAll(d, 0o755); err != nil {
			t.Fatal(err)
		}
	}
	if err := os.WriteFile(filepath.Join(sibling, "package.json"),
		[]byte(`{"name":"localpkg","version":"1.0.0"}`), 0o644); err != nil {
		t.Fatal(err)
	}
	d := &Deps{
		Manifest:    `{"name":"proj","version":"1.0.0","dependencies":{"localpkg":"file:../localpkg"}}`,
		ManifestDir: project,
	}
	if _, err := materialiseDeps(workdir, KindNode, d, false, project); err != nil {
		t.Fatalf("sibling file: dependency must materialise, got %v", err)
	}
	m := readJSON(t, filepath.Join(workdir, "package.json"))
	depsMap, _ := m["dependencies"].(map[string]any)
	spec, _ := depsMap["localpkg"].(string)
	if !strings.HasPrefix(spec, "file:./.mcpx-local/") || !strings.HasSuffix(spec, "/localpkg") {
		t.Fatalf("materialised spec = %q, want a rebased file:./.mcpx-local/.../localpkg", spec)
	}
	copied := filepath.Join(workdir, filepath.FromSlash(strings.TrimPrefix(spec, "file:./")), "package.json")
	if _, err := os.Stat(copied); err != nil {
		t.Fatalf("sibling artifact not transported to %s: %v", copied, err)
	}
}
