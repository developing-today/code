package runner

import (
	"context"
	"encoding/json"
	"os"
	"path/filepath"
	"testing"

	"github.com/developing-today/code/pkgs/mcpx/internal/preflight"
)

// writeInstalledPackage lays down one installed package inside a
// prepared environment's node_modules: a package.json with the given
// body (none when empty) and the named files, each with dummy content.
func writeInstalledPackage(t *testing.T, modules, name, manifest string, files ...string) string {
	t.Helper()
	dir := filepath.Join(modules, filepath.FromSlash(name))
	if err := os.MkdirAll(dir, 0o755); err != nil {
		t.Fatal(err)
	}
	if manifest != "" {
		if err := os.WriteFile(filepath.Join(dir, "package.json"), []byte(manifest), 0o644); err != nil {
			t.Fatal(err)
		}
	}
	for _, f := range files {
		p := filepath.Join(dir, filepath.FromSlash(f))
		if err := os.MkdirAll(filepath.Dir(p), 0o755); err != nil {
			t.Fatal(err)
		}
		if err := os.WriteFile(p, []byte("// pkg\n"), 0o644); err != nil {
			t.Fatal(err)
		}
	}
	return dir
}

// makeDenoPreparedEnv builds a fake prepared environment entry whose
// node_modules exercises the import map's entry-point forms: a main, a
// string exports, a conditional exports, a scoped package on the
// index.js default, and one package with no resolvable entry point.
func makeDenoPreparedEnv(t *testing.T, root string) string {
	t.Helper()
	env := filepath.Join(root, "env-deno")
	modules := filepath.Join(env, "node_modules")
	if err := os.MkdirAll(modules, 0o755); err != nil {
		t.Fatal(err)
	}
	manifest := "{\n  \"name\": \"mcpx-prepared-imports\",\n  \"version\": \"0.0.0\",\n  \"private\": true\n}\n"
	if err := os.WriteFile(filepath.Join(env, "package.json"), []byte(manifest), 0o644); err != nil {
		t.Fatal(err)
	}
	writeInstalledPackage(t, modules, "mainpkg", `{"name":"mainpkg","main":"lib/main.js"}`, "lib/main.js")
	writeInstalledPackage(t, modules, "exportstr", `{"name":"exportstr","exports":"./esm.js"}`, "esm.js")
	writeInstalledPackage(t, modules, "exportcond", `{"name":"exportcond","exports":{".":{"import":"./imp.js","default":"./def.js"}}}`, "imp.js", "def.js")
	writeInstalledPackage(t, modules, "@scope/scoped", `{"name":"@scope/scoped"}`, "index.js")
	writeInstalledPackage(t, modules, "entryless", `{"name":"entryless","main":"gone.js"}`)
	return env
}

func TestPointDenoRunAtPreparedEnvRendersTheImportMap(t *testing.T) {
	root := t.TempDir()
	env := makeDenoPreparedEnv(t, root)
	work := filepath.Join(root, "work")
	if err := os.MkdirAll(work, 0o755); err != nil {
		t.Fatal(err)
	}
	scriptDir := filepath.Join(root, "proj")
	if err := os.MkdirAll(scriptDir, 0o755); err != nil {
		t.Fatal(err)
	}
	script := filepath.Join(scriptDir, "main.ts")
	if err := os.WriteFile(script, []byte("import 'mainpkg'\n"), 0o644); err != nil {
		t.Fatal(err)
	}
	prepared := &preflight.PreparedSet{Dir: env, Key: "k", Packages: []preflight.ResolvedPackage{
		{Name: "mainpkg", Version: "1.0.0"},
		{Name: "exportstr", Version: "2.0.0"},
		{Name: "exportcond", Version: "3.0.0"},
		{Name: "@scope/scoped", Version: "4.0.0"},
	}}
	release, flags, err := pointRunAtPreparedEnv(context.Background(), Options{File: script}, KindDeno, prepared, work, false)
	if err != nil {
		t.Fatal(err)
	}
	if release == nil {
		t.Fatal("the wiring holds the cache lease until release")
	}
	defer release()
	mapPath := filepath.Join(work, preparedImportMapName)
	if len(flags) != 2 || flags[0] != "--import-map" || flags[1] != mapPath {
		t.Fatalf("the run is passed the rendered import map: got %v", flags)
	}
	text, err := os.ReadFile(mapPath)
	if err != nil {
		t.Fatal(err)
	}
	var rendered struct {
		Imports map[string]string `json:"imports"`
	}
	if err := json.Unmarshal(text, &rendered); err != nil {
		t.Fatal(err)
	}
	modules := filepath.Join(env, "node_modules")
	want := map[string]string{
		"mainpkg":        "file://" + filepath.Join(modules, "mainpkg", "lib", "main.js"),
		"mainpkg/":       "file://" + filepath.Join(modules, "mainpkg") + "/",
		"exportstr":      "file://" + filepath.Join(modules, "exportstr", "esm.js"),
		"exportstr/":     "file://" + filepath.Join(modules, "exportstr") + "/",
		"exportcond":     "file://" + filepath.Join(modules, "exportcond", "imp.js"),
		"@scope/scoped":  "file://" + filepath.Join(modules, "@scope", "scoped", "index.js"),
		"@scope/scoped/": "file://" + filepath.Join(modules, "@scope", "scoped") + "/",
		"entryless/":     "file://" + filepath.Join(modules, "entryless") + "/",
	}
	for name, w := range want {
		if got := rendered.Imports[name]; got != w {
			t.Errorf("imports[%q]: got %s, want %s", name, got, w)
		}
	}
	if _, ok := rendered.Imports["entryless"]; ok {
		t.Error("an unpinned package with no entry point gets no exact mapping")
	}
	if _, err := os.Lstat(filepath.Join(scriptDir, "node_modules")); !os.IsNotExist(err) {
		t.Errorf("a deno run gets no node_modules link, stat err = %v", err)
	}
}

func TestPointDenoRunAtPreparedEnvFailsClosed(t *testing.T) {
	root := t.TempDir()
	env := makeDenoPreparedEnv(t, root)
	work := filepath.Join(root, "work")
	if err := os.MkdirAll(work, 0o755); err != nil {
		t.Fatal(err)
	}
	script := filepath.Join(root, "main.ts")

	// A pin the entry does not install: prepare reported a set the
	// environment does not carry.
	prepared := &preflight.PreparedSet{Dir: env, Key: "k", Packages: []preflight.ResolvedPackage{{Name: "ghost", Version: "1.0.0"}}}
	if _, _, err := pointRunAtPreparedEnv(context.Background(), Options{File: script}, KindDeno, prepared, work, false); err == nil {
		t.Fatal("a pin the prepared environment does not install must fail the run, not fall back")
	}

	// A pin whose package has no entry point Deno can import.
	prepared = &preflight.PreparedSet{Dir: env, Key: "k", Packages: []preflight.ResolvedPackage{{Name: "entryless", Version: "1.0.0"}}}
	if _, _, err := pointRunAtPreparedEnv(context.Background(), Options{File: script}, KindDeno, prepared, work, false); err == nil {
		t.Fatal("a pin with no entry point must fail the run, not fall back")
	}
}

func TestPointDenoRunAtPreparedEnvClearsAStaleMap(t *testing.T) {
	root := t.TempDir()
	work := filepath.Join(root, "work")
	if err := os.MkdirAll(work, 0o755); err != nil {
		t.Fatal(err)
	}
	mapPath := filepath.Join(work, preparedImportMapName)
	if err := os.WriteFile(mapPath, []byte("{}\n"), 0o644); err != nil {
		t.Fatal(err)
	}
	script := filepath.Join(root, "main.ts")
	release, flags, err := pointRunAtPreparedEnv(context.Background(), Options{File: script}, KindDeno, nil, work, false)
	if err != nil {
		t.Fatal(err)
	}
	if release != nil || len(flags) != 0 {
		t.Errorf("a run with no environment holds and passes nothing: flags=%v", flags)
	}
	if _, err := os.Lstat(mapPath); !os.IsNotExist(err) {
		t.Errorf("the stale prepared import map is cleared, stat err = %v", err)
	}
}
