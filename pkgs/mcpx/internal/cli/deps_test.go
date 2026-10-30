package cli

import (
	"encoding/json"
	"os"
	"path/filepath"
	"testing"

	"github.com/dezren39/mcpx/internal/execsvc"
)

// TestDepsOptionsAreNilWithoutARequest is the default that keeps #1913 from
// changing every existing run: a script that mentioned no packages carries no
// dependency set, so the runtime discovers its own the way it always has.
func TestDepsOptionsAreNilWithoutARequest(t *testing.T) {
	a := &App{}
	if got := a.depsOptions(t.TempDir()); got != nil {
		t.Fatalf("no request should carry no deps, got %+v", got)
	}
}

// TestDepsOptionsCarryManifestText: the wire carries content, never a path, so
// a daemon that shares no filesystem can still materialise the project.
func TestDepsOptionsCarryManifestText(t *testing.T) {
	dir := t.TempDir()
	manifest := filepath.Join(dir, "package.json")
	body := `{"dependencies":{"left-pad":"1.3.0"}}`
	if err := os.WriteFile(manifest, []byte(body), 0o644); err != nil {
		t.Fatal(err)
	}
	lock := filepath.Join(dir, "bun.lock")
	if err := os.WriteFile(lock, []byte("{}"), 0o644); err != nil {
		t.Fatal(err)
	}

	a := &App{}
	if err := a.Settings().SetRuntime("deps.packages", "left-pad@1.3.0"); err != nil {
		t.Fatal(err)
	}
	got := a.depsOptions(dir)
	if got == nil {
		t.Fatal("a package request should carry a dependency set")
	}
	if len(got.Packages) != 1 || got.Packages[0] != "left-pad@1.3.0" {
		t.Fatalf("packages: %v", got.Packages)
	}
	if got.Manifest != body {
		t.Fatalf("manifest should be the file's text, got %q", got.Manifest)
	}
	if got.ManifestName != "package.json" {
		t.Fatalf("manifest name: %q", got.ManifestName)
	}
	if got.LockfileName != "bun.lock" || got.Lockfile != "{}" {
		t.Fatalf("lockfile: %q %q", got.LockfileName, got.Lockfile)
	}
	if got.AutoInstall != "off" {
		t.Fatalf("auto-install default should be off, got %q", got.AutoInstall)
	}
}

// TestDepsOptionsStripJSONC: a caller's package.jsonc reaches the runtime as
// plain JSON under the name package.json, because no runtime reads comments.
func TestDepsOptionsStripJSONC(t *testing.T) {
	dir := t.TempDir()
	body := "{\n  // the demo needs this\n  \"dependencies\": {\"left-pad\": \"1.3.0\"},\n}\n"
	if err := os.WriteFile(filepath.Join(dir, "package.jsonc"), []byte(body), 0o644); err != nil {
		t.Fatal(err)
	}

	a := &App{}
	if err := a.Settings().SetRuntime("deps.source", "cwd"); err != nil {
		t.Fatal(err)
	}
	got := a.depsOptions(dir)
	if got == nil {
		t.Fatal("deps.source=cwd should carry the project's set")
	}
	if got.ManifestName != "package.json" {
		t.Fatalf("jsonc must be sent as package.json, got %q", got.ManifestName)
	}
	var doc map[string]any
	if err := json.Unmarshal([]byte(got.Manifest), &doc); err != nil {
		t.Fatalf("sent manifest is not plain JSON (%v): %s", err, got.Manifest)
	}
}

// TestDepsOptionsAreOnTheWire is the parity check that matters: the struct the
// CLI fills is the struct the daemon decodes.
func TestDepsOptionsAreOnTheWire(t *testing.T) {
	in := execsvc.Options{Deps: &execsvc.DepsOptions{
		Packages: []string{"left-pad@1.3.0"},
		Source:   "cwd",
	}}
	b, err := json.Marshal(in)
	if err != nil {
		t.Fatal(err)
	}
	var out execsvc.Options
	if err := json.Unmarshal(b, &out); err != nil {
		t.Fatal(err)
	}
	if out.Deps == nil || len(out.Deps.Packages) != 1 || out.Deps.Packages[0] != "left-pad@1.3.0" {
		t.Fatalf("deps did not survive the wire: %s", b)
	}
	if out.Deps.Source != "cwd" {
		t.Fatalf("source did not survive the wire: %s", b)
	}
}
