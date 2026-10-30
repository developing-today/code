package runner

import (
	"encoding/json"
	"os"
	"path/filepath"
	"testing"
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
	})
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
	})
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
	// Frozen beats the autoInstall mapping for bun.
	if len(flags) != 1 || flags[0] != "--install=fallback" {
		t.Fatalf("flags = %v, want [--install=fallback] for a frozen bun set", flags)
	}
}

func TestMaterialiseDepsDenoImportsMap(t *testing.T) {
	dir := t.TempDir()
	flags, err := materialiseDeps(dir, KindDeno, &Deps{
		Packages: []string{"zod@^3.0.0", "lodash"},
	})
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
	if _, err := materialiseDeps(dir, KindNode, &Deps{Manifest: "{nope"}); err == nil {
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
	}); err != nil {
		t.Fatal(err)
	}
	m := readJSON(t, filepath.Join(dir, "package.json"))
	deps := m["dependencies"].(map[string]any)
	if deps["alias"] != "npm:@scope/pkg@2.0.0" || len(deps) != 1 {
		t.Fatalf("dependencies = %v, want {alias: npm:@scope/pkg@2.0.0}", deps)
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
	if _, err := materialiseDeps(dir, KindNode, d); err != nil {
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
}
