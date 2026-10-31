package cli

import (
	"bytes"
	"encoding/json"
	"os"
	"path/filepath"
	"strings"
	"testing"

	"github.com/dezren39/mcpx/internal/execsvc"
)

// TestDepsOptionsAreNilWithoutARequest is the default that keeps #1913 from
// changing every existing run: a script that mentioned no packages carries no
// dependency set, so the runtime discovers its own the way it always has.
func TestDepsOptionsAreNilWithoutARequest(t *testing.T) {
	a := &App{}
	got, err := a.depsOptions(t.TempDir(), "")
	if err != nil {
		t.Fatal(err)
	}
	if got != nil {
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
	got, err := a.depsOptions(dir, "")
	if err != nil {
		t.Fatal(err)
	}
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
	if got.LockfileName != "bun.lock" || string(got.Lockfile) != "{}" {
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
	got, err := a.depsOptions(dir, "")
	if err != nil {
		t.Fatal(err)
	}
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

// TestDepsOptionsAutoInstallOnlyIsARequest: an explicit autoInstall policy is
// a request even when the source stays host and no package was named -- the
// fast path must not discard it (Codex finding on PR 2589).
func TestDepsOptionsAutoInstallOnlyIsARequest(t *testing.T) {
	a := &App{}
	if err := a.Settings().SetRuntime("deps.autoInstall", "fallback"); err != nil {
		t.Fatal(err)
	}
	got, err := a.depsOptions(t.TempDir(), "")
	if err != nil {
		t.Fatal(err)
	}
	if got == nil {
		t.Fatal("an explicit autoInstall policy should carry a dependency set")
	}
	if got.AutoInstall != "fallback" {
		t.Fatalf("autoInstall: %q", got.AutoInstall)
	}
}

// TestDepsOptionsConfigLayerPackagesReachPayload: with deps.source=config the
// resolved set includes the configuration directory's packages, and the wire
// payload must carry them -- the serialized manifest is the project's, so a
// payload built from only the explicit layer would drop them (Codex finding
// on PR 2589).
func TestDepsOptionsConfigLayerPackagesReachPayload(t *testing.T) {
	xdg := t.TempDir()
	cfgDir := filepath.Join(xdg, "mcpx")
	if err := os.MkdirAll(cfgDir, 0o755); err != nil {
		t.Fatal(err)
	}
	body := `{"dependencies":{"conf-pkg":"2.0.0"}}`
	if err := os.WriteFile(filepath.Join(cfgDir, "package.json"), []byte(body), 0o644); err != nil {
		t.Fatal(err)
	}
	t.Setenv("XDG_CONFIG_HOME", xdg)

	a := &App{}
	if err := a.Settings().SetRuntime("deps.source", "config"); err != nil {
		t.Fatal(err)
	}
	got, err := a.depsOptions(t.TempDir(), "")
	if err != nil {
		t.Fatal(err)
	}
	if got == nil {
		t.Fatal("deps.source=config should carry a dependency set")
	}
	found := false
	for _, p := range got.Packages {
		if p == "conf-pkg@2.0.0" {
			found = true
		}
	}
	if !found {
		t.Fatalf("config-layer package missing from payload: %v", got.Packages)
	}
}

// TestDepsOptionsBinaryLockfileSurvivesWire: bun.lockb is binary; carried as
// bytes it survives JSON transport byte-for-byte, where a string would
// corrupt invalid UTF-8 (Codex finding on PR 2589).
func TestDepsOptionsBinaryLockfileSurvivesWire(t *testing.T) {
	dir := t.TempDir()
	if err := os.WriteFile(filepath.Join(dir, "package.json"), []byte(`{"dependencies":{"left-pad":"1.3.0"}}`), 0o644); err != nil {
		t.Fatal(err)
	}
	raw := []byte{0x00, 0x80, 0xff, 0x42}
	if err := os.WriteFile(filepath.Join(dir, "bun.lockb"), raw, 0o644); err != nil {
		t.Fatal(err)
	}
	a := &App{}
	if err := a.Settings().SetRuntime("deps.packages", "left-pad@1.3.0"); err != nil {
		t.Fatal(err)
	}
	got, err := a.depsOptions(dir, "")
	if err != nil {
		t.Fatal(err)
	}
	if got == nil || got.LockfileName != "bun.lockb" {
		t.Fatalf("lockfile name: %+v", got)
	}
	if !bytes.Equal(got.Lockfile, raw) {
		t.Fatalf("lockfile bytes corrupted in payload: %x", got.Lockfile)
	}
	b, err := json.Marshal(execsvc.Options{Deps: got})
	if err != nil {
		t.Fatal(err)
	}
	var out execsvc.Options
	if err := json.Unmarshal(b, &out); err != nil {
		t.Fatal(err)
	}
	if !bytes.Equal(out.Deps.Lockfile, raw) {
		t.Fatalf("lockfile bytes corrupted on the wire: %x", out.Deps.Lockfile)
	}
}

// TestDepsOptionsResolutionFailureIsAnError: a manifest that cannot be parsed
// is reported, not silently treated as an unconfigured run (Codex finding on
// PR 2589).
func TestDepsOptionsResolutionFailureIsAnError(t *testing.T) {
	dir := t.TempDir()
	bad := filepath.Join(dir, "package.json")
	if err := os.WriteFile(bad, []byte("{not json"), 0o644); err != nil {
		t.Fatal(err)
	}
	a := &App{}
	if err := a.Settings().SetRuntime("deps.manifest", bad); err != nil {
		t.Fatal(err)
	}
	if _, err := a.depsOptions(dir, ""); err == nil {
		t.Fatal("an unparseable explicit manifest should be an error")
	}
}

// TestDepsOptionsDiscoverFromScriptDir: discovery starts at the script's
// directory, not the caller's cwd -- running /repoB/scripts/report.ts from
// /repoA means repoB's project (Codex finding on PR 2589).
func TestDepsOptionsDiscoverFromScriptDir(t *testing.T) {
	repoA := t.TempDir()
	repoB := t.TempDir()
	body := `{"dependencies":{"b-pkg":"1.0.0"}}`
	if err := os.WriteFile(filepath.Join(repoB, "package.json"), []byte(body), 0o644); err != nil {
		t.Fatal(err)
	}
	script := filepath.Join(repoB, "report.ts")
	if err := os.WriteFile(script, []byte("console.log(1)"), 0o644); err != nil {
		t.Fatal(err)
	}
	a := &App{}
	if err := a.Settings().SetRuntime("deps.source", "cwd"); err != nil {
		t.Fatal(err)
	}
	got, err := a.depsOptions(repoA, script)
	if err != nil {
		t.Fatal(err)
	}
	if got == nil || got.Manifest != body {
		t.Fatalf("should carry repoB's manifest, got %+v", got)
	}
}

func TestDepsOptionsReportConflicts(t *testing.T) {
	// Codex finding on #2589: a contradictory explicit pin used to be
	// dropped with the resolution metadata and sent silently.
	dir := t.TempDir()
	if err := os.WriteFile(filepath.Join(dir, "package.json"),
		[]byte(`{"dependencies":{"left-pad":"1.3.0"}}`), 0o644); err != nil {
		t.Fatal(err)
	}
	a := &App{}
	if err := a.Settings().SetRuntime("deps.packages", "left-pad@2.0.0"); err != nil {
		t.Fatal(err)
	}
	_, err := a.depsOptions(dir, "")
	if err == nil || !strings.Contains(err.Error(), "left-pad") ||
		!strings.Contains(err.Error(), "2.0.0") || !strings.Contains(err.Error(), "1.3.0") {
		t.Fatalf("err = %v, want a conflict error naming both versions", err)
	}
}

// Codex round (PR #2589): an EXPLICIT deps.autoInstall=off is a
// policy, not an absence — the nil fast path must not swallow it,
// because the runner's --no-install flag is the only thing that
// disables Bun's default auto-install.
func TestDepsOptionsExplicitOffAutoInstallIsARequest(t *testing.T) {
	a := &App{}
	if err := a.Settings().SetRuntime("deps.autoInstall", "off"); err != nil {
		t.Fatal(err)
	}
	got, err := a.depsOptions(t.TempDir(), "")
	if err != nil {
		t.Fatal(err)
	}
	if got == nil {
		t.Fatal("an explicit deps.autoInstall=off should carry a dependency set")
	}
	if got.AutoInstall != "off" {
		t.Fatalf("autoInstall: %q", got.AutoInstall)
	}
}
