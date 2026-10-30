package deps_test

import (
	"errors"
	"os"
	"path/filepath"
	"strings"
	"testing"

	"github.com/dezren39/mcpx/internal/deps"
	"github.com/dezren39/mcpx/internal/searchpath"
	"github.com/dezren39/mcpx/internal/settings"
)

// write puts a file in dir, creating parents, and returns its path.
func write(t *testing.T, dir, name, body string) string {
	t.Helper()
	p := filepath.Join(dir, name)
	if err := os.MkdirAll(filepath.Dir(p), 0o755); err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(p, []byte(body), 0o644); err != nil {
		t.Fatal(err)
	}
	return p
}

// cfg builds a Config whose search path holds dirs, so discovery is the thing
// under test rather than the settings layers.
func cfg(t *testing.T, source deps.Source, dirs ...string) deps.Config {
	t.Helper()
	return deps.Config{
		Source: source,
		Paths:  searchpath.Resolve(dirs, settings.NullMarker, searchpath.Options{}),
	}
}

// TestChainPrecedence is the order in the issue: an explicit request beats the
// project manifest, which beats the configuration directory. The host
// contributes nothing to the set.
func TestChainPrecedence(t *testing.T) {
	project := t.TempDir()
	write(t, project, "package.json", `{"dependencies":{"left-pad":"1.3.0","shared":"2.0.0"}}`)

	configDir := t.TempDir()
	t.Setenv("XDG_CONFIG_HOME", configDir)
	write(t, filepath.Join(configDir, "mcpx"), "package.json", `{"dependencies":{"shared":"9.9.9","config-only":"1.0.0"}}`)

	c := cfg(t, deps.SourceConfig, project)
	c.Packages = []string{"shared@2.0.0", "explicit-only@4.0.0"}

	set, err := deps.Resolve(c, "")
	if err != nil {
		t.Fatal(err)
	}
	got := map[string]string{}
	from := map[string]string{}
	for _, p := range set.Packages {
		got[p.Name] = p.Version
		from[p.Name] = p.From
	}
	want := map[string]string{
		"shared":        "2.0.0",
		"explicit-only": "4.0.0",
		"left-pad":      "1.3.0",
		"config-only":   "1.0.0",
	}
	for name, version := range want {
		if got[name] != version {
			t.Errorf("%s: version %q, want %q (set: %s)", name, got[name], version, set.Describe())
		}
	}
	if from["shared"] != "explicit" {
		t.Errorf("shared came from %q, want explicit", from["shared"])
	}
	if from["left-pad"] != "project" {
		t.Errorf("left-pad came from %q, want project", from["left-pad"])
	}
	if from["config-only"] != "config" {
		t.Errorf("config-only came from %q, want config", from["config-only"])
	}
}

// TestSourceNoneResolvesNothing pins the loud setting: nothing is provided and
// the runtime is allowed to fail.
func TestSourceNoneResolvesNothing(t *testing.T) {
	project := t.TempDir()
	write(t, project, "package.json", `{"dependencies":{"left-pad":"1.3.0"}}`)
	c := cfg(t, deps.SourceNone, project)
	c.Packages = []string{"explicit-only@4.0.0"}

	set, err := deps.Resolve(c, "")
	if err != nil {
		t.Fatal(err)
	}
	if len(set.Packages) != 0 {
		t.Fatalf("source none resolved %d packages: %s", len(set.Packages), set.Describe())
	}
}

// TestExplicitConflictsAreReportedNotMerged is the rule that keeps a manifest
// from becoming a lie: the pin is reported, and the manifest's own version is
// what the run carries.
func TestExplicitConflictsAreReportedNotMerged(t *testing.T) {
	project := t.TempDir()
	write(t, project, "package.json", `{"dependencies":{"left-pad":"1.3.0"}}`)
	c := cfg(t, deps.SourceCWD, project)
	c.Packages = []string{"left-pad@2.0.0"}

	set, err := deps.Resolve(c, "")
	if err != nil {
		t.Fatal(err)
	}
	if len(set.Conflicts) != 1 {
		t.Fatalf("want 1 conflict, got %d (%s)", len(set.Conflicts), set.Describe())
	}
	conflict := set.Conflicts[0]
	if conflict.Name != "left-pad" || conflict.Explicit != "2.0.0" || conflict.Manifest != "1.3.0" {
		t.Fatalf("unexpected conflict: %+v", conflict)
	}
	if got := set.Packages[0]; got.Name != "left-pad" || got.Version != "2.0.0" {
		t.Fatalf("explicit pin should still be the requested version, got %+v", got)
	}
}

// TestPackageJSONCIsAccepted covers the format the issue asks for: a manifest
// with comments is read, and the runtime never sees the comments.
func TestPackageJSONCIsAccepted(t *testing.T) {
	project := t.TempDir()
	write(t, project, "package.jsonc", `{
  // the package the demo script needs
  "dependencies": {
    "left-pad": "1.3.0", /* pinned deliberately */
  },
}`)

	m, err := deps.LoadDir(project)
	if err != nil {
		t.Fatal(err)
	}
	if !m.JSONC {
		t.Fatal("manifest should be marked JSONC")
	}
	if m.Packages["left-pad"] != "1.3.0" {
		t.Fatalf("packages: %v", m.Packages)
	}
}

// TestStripJSONCLeavesStringsAlone is why the stripper is a scanner: a url
// containing // is a value, not a comment.
func TestStripJSONCLeavesStringsAlone(t *testing.T) {
	in := []byte(`{"dependencies":{"pkg":"git+https://example.com/pkg.git","x":"a/*b*/c"} // note
}`)
	out := string(deps.StripJSONC(in))
	if !strings.Contains(out, "git+https://example.com/pkg.git") {
		t.Fatalf("url corrupted: %s", out)
	}
	if !strings.Contains(out, "a/*b*/c") {
		t.Fatalf("string content corrupted: %s", out)
	}
	if strings.Contains(out, "// note") {
		t.Fatalf("comment survived: %s", out)
	}
}

// TestFrozenWhenLockfilePresent: a present lockfile means the runtime is asked
// to honour it, and mcpx never rewrites it.
func TestFrozenWhenLockfilePresent(t *testing.T) {
	project := t.TempDir()
	write(t, project, "package.json", `{"dependencies":{"left-pad":"1.3.0"}}`)
	write(t, project, "bun.lock", `{}`)

	set, err := deps.Resolve(cfg(t, deps.SourceCWD, project), "")
	if err != nil {
		t.Fatal(err)
	}
	if !set.Frozen {
		t.Fatalf("lockfile present but set is not frozen: %s", set.Describe())
	}
	if !strings.Contains(set.Describe(), "honoured frozen") {
		t.Fatalf("Describe should say the lockfile is honoured: %s", set.Describe())
	}
}

// TestDiscoveryWalksUp: a script in a subdirectory belongs to the project above
// it, because that is where the person who wrote it believes they are.
func TestDiscoveryWalksUp(t *testing.T) {
	project := t.TempDir()
	write(t, project, "package.json", `{"dependencies":{"left-pad":"1.3.0"}}`)
	script := filepath.Join(project, "scripts", "deep")
	if err := os.MkdirAll(script, 0o755); err != nil {
		t.Fatal(err)
	}

	m, err := deps.Find(cfg(t, deps.SourceCWD, project), script)
	if err != nil {
		t.Fatal(err)
	}
	if filepath.Dir(m.Path) != project {
		t.Fatalf("found %s, want the manifest in %s", m.Path, project)
	}
}

// TestParseKeepsScope is the one string rule worth a test: @scope/pkg@1.2.3
// has one version separator and its scope is not a version.
func TestParseKeepsScope(t *testing.T) {
	cases := map[string]deps.Specifier{
		"left-pad":             {Name: "left-pad"},
		"left-pad@1.3.0":       {Name: "left-pad", Version: "1.3.0"},
		"@scope/pkg":           {Name: "@scope/pkg"},
		"@scope/pkg@1.2.3":     {Name: "@scope/pkg", Version: "1.2.3"},
		"pkg@npm:@scope/x@2.0": {Name: "pkg", Version: "npm:@scope/x@2.0"},
	}
	for in, want := range cases {
		if got := deps.Parse(in); got != want {
			t.Errorf("Parse(%q) = %+v, want %+v", in, got, want)
		}
	}
}

func TestUnversionedRequestInheritsTheLowerLayerPin(t *testing.T) {
	// Codex finding on #2589: an unversioned explicit request used to
	// mark the name seen, so the configuration layer's pin was
	// discarded and the payload carried a floating name.
	configDir := t.TempDir()
	t.Setenv("XDG_CONFIG_HOME", configDir)
	write(t, filepath.Join(configDir, "mcpx"), "package.json", `{"dependencies":{"foo":"1.2.3"}}`)

	c := cfg(t, deps.SourceConfig, t.TempDir())
	c.Packages = []string{"foo"}
	set, err := deps.Resolve(c, t.TempDir())
	if err != nil {
		t.Fatal(err)
	}
	if len(set.Packages) != 1 || set.Packages[0].Name != "foo" || set.Packages[0].Version != "1.2.3" {
		t.Fatalf("Packages = %+v, want foo@1.2.3 inherited from the config layer", set.Packages)
	}
}

func TestConfigLayerManifestErrorIsNotSwallowed(t *testing.T) {
	configDir := t.TempDir()
	t.Setenv("XDG_CONFIG_HOME", configDir)
	write(t, filepath.Join(configDir, "mcpx"), "package.json", `{not json`)

	c := cfg(t, deps.SourceConfig, t.TempDir())
	if _, err := deps.Resolve(c, t.TempDir()); err == nil {
		t.Fatal("a malformed configuration manifest must fail resolution, not resolve empty")
	}
}

// TestAPinInsideTheManifestRangeIsNotAConflict is the Codex finding on
// PR #2589 (config.go): the conflict check compared strings, so a manifest
// declaring "^1.3.0" contradicted an explicit left-pad@1.3.0 it allows.
func TestAPinInsideTheManifestRangeIsNotAConflict(t *testing.T) {
	project := t.TempDir()
	write(t, project, "package.json", `{"dependencies":{"left-pad":"^1.3.0","tilde":"~2.0.0","floor":">=1.0.0"}}`)
	for _, spec := range []string{"left-pad@1.9.4", "tilde@2.0.7", "floor@3.1.0"} {
		c := cfg(t, deps.SourceCWD, project)
		c.Packages = []string{spec}
		set, err := deps.Resolve(c, "")
		if err != nil {
			t.Fatal(err)
		}
		if len(set.Conflicts) != 0 {
			t.Errorf("%s satisfies the manifest range and must not conflict: %+v", spec, set.Conflicts)
		}
	}
	// And a pin outside the range still contradicts it.
	c := cfg(t, deps.SourceCWD, project)
	c.Packages = []string{"left-pad@2.0.0"}
	set, err := deps.Resolve(c, "")
	if err != nil {
		t.Fatal(err)
	}
	if len(set.Conflicts) != 1 {
		t.Errorf("left-pad@2.0.0 against ^1.3.0 must conflict, got %+v", set.Conflicts)
	}
}

// TestAnExplicitlyNamedManifestThatIsMissingFails is the Codex finding on
// PR #2589 (chain.go): discovery tolerates finding nothing, but a manifest
// the caller named by path must not vanish silently.
func TestAnExplicitlyNamedManifestThatIsMissingFails(t *testing.T) {
	c := cfg(t, deps.SourceCWD, t.TempDir())
	c.Manifest = filepath.Join(t.TempDir(), "no-such-package.json")
	c.Packages = []string{"foo@1.0.0"}
	if _, err := deps.Resolve(c, ""); !errors.Is(err, deps.ErrNoManifest) {
		t.Fatalf("a named-but-missing manifest must fail with ErrNoManifest, got %v", err)
	}
}

// TestConfigLayerConflictsAreReported extends the conflict rule to the
// configuration directory's manifest: its declarations are a manifest too.
func TestConfigLayerConflictsAreReported(t *testing.T) {
	configDir := t.TempDir()
	t.Setenv("XDG_CONFIG_HOME", configDir)
	write(t, filepath.Join(configDir, "mcpx"), "package.json", `{"dependencies":{"left-pad":"1.3.0"}}`)

	c := cfg(t, deps.SourceConfig, t.TempDir())
	c.Packages = []string{"left-pad@2.0.0"}
	set, err := deps.Resolve(c, t.TempDir())
	if err != nil {
		t.Fatal(err)
	}
	if len(set.Conflicts) != 1 || set.Conflicts[0].Name != "left-pad" {
		t.Fatalf("want the config-layer conflict reported, got %+v", set.Conflicts)
	}
}
