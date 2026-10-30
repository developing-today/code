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
	}
	for _, c := range cases {
		name, ver := splitSpecifier(c.spec)
		if name != c.name || ver != c.ver {
			t.Errorf("splitSpecifier(%q) = %q, %q; want %q, %q", c.spec, name, ver, c.name, c.ver)
		}
	}
}
