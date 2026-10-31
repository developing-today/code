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

// TestConfigLayerConflictMaskedByProjectIsIgnored is the Codex finding on
// PR #2589 (chain.go): with deps.source=config, the project manifest
// supplies "shared" (2.0.0) and the explicit pin matches it; the config
// layer's overridden 9.9.9 declaration must not be reported as a
// conflict, or depsOptions rejects a run that matches the winning
// project declaration. (TestChainPrecedence's fixture, asserting the
// conflicts it previously never looked at.)
func TestConfigLayerConflictMaskedByProjectIsIgnored(t *testing.T) {
	project := t.TempDir()
	write(t, project, "package.json", `{"dependencies":{"shared":"2.0.0"}}`)
	configDir := t.TempDir()
	t.Setenv("XDG_CONFIG_HOME", configDir)
	write(t, filepath.Join(configDir, "mcpx"), "package.json", `{"dependencies":{"shared":"9.9.9"}}`)

	c := cfg(t, deps.SourceConfig, project)
	c.Packages = []string{"shared@2.0.0"}
	set, err := deps.Resolve(c, "")
	if err != nil {
		t.Fatal(err)
	}
	if len(set.Conflicts) != 0 {
		t.Errorf("pin matching the project manifest must not conflict with the overridden config entry: %+v", set.Conflicts)
	}
}

// TestPrereleasePinsConflict is the Codex finding on PR #2589
// (config.go): prerelease identifiers are part of the version. A
// prerelease of the pinned release is not the pin, and a range without
// a prerelease comparator does not admit prereleases of other tuples.
func TestPrereleasePinsConflict(t *testing.T) {
	project := t.TempDir()
	write(t, project, "package.json", `{"dependencies":{"foo":"1.3.0","bar":"^1.3.0","baz":"^1.3.0-beta.1"}}`)
	for _, tc := range []struct {
		spec     string
		conflict bool
	}{
		{"foo@1.3.0-beta.1", true},  // a prerelease is not the 1.3.0 pin
		{"foo@1.3.0", false},        // the pin itself
		{"bar@1.4.0-beta.1", true},  // ^1.3.0 has no prerelease comparator
		{"bar@1.4.0", false},        // an ordinary in-range release
		{"baz@1.3.0-beta.2", false}, // same tuple, prerelease comparator present
		{"baz@1.3.0-alpha", true},   // below the range's prerelease floor
	} {
		c := cfg(t, deps.SourceCWD, project)
		c.Packages = []string{tc.spec}
		set, err := deps.Resolve(c, "")
		if err != nil {
			t.Fatal(err)
		}
		if got := len(set.Conflicts) != 0; got != tc.conflict {
			t.Errorf("%s: conflict = %v, want %v (conflicts: %+v)", tc.spec, got, tc.conflict, set.Conflicts)
		}
	}
}

// TestConfigLayerLockfileIsPreservedWithoutAProject is the Codex
// finding on PR #2589 (chain.go): with deps.source=config and no
// project manifest, the config layer's own lockfile was discarded —
// the set was neither frozen nor carrying a manifest, so no lockfile
// ever reached the payload.
func TestConfigLayerLockfileIsPreservedWithoutAProject(t *testing.T) {
	configDir := t.TempDir()
	t.Setenv("XDG_CONFIG_HOME", configDir)
	write(t, filepath.Join(configDir, "mcpx"), "package.json", `{"dependencies":{"foo":"^1.0.0"}}`)
	write(t, filepath.Join(configDir, "mcpx"), "package-lock.json", `{"lockfileVersion":3}`)

	set, err := deps.Resolve(cfg(t, deps.SourceConfig, t.TempDir()), t.TempDir())
	if err != nil {
		t.Fatal(err)
	}
	if !set.Frozen {
		t.Fatalf("config lockfile present but set is not frozen: %s", set.Describe())
	}
	if set.Manifest == nil {
		t.Fatal("config manifest was not preserved on the set, so its lockfile cannot be transported")
	}
	name, text, err := set.Manifest.LockfileText()
	if err != nil {
		t.Fatal(err)
	}
	if name != "package-lock.json" || !strings.Contains(string(text), "lockfileVersion") {
		t.Fatalf("lockfile = %q %q, want the config layer's package-lock.json", name, text)
	}
}

// TestLockfileTextPropagatesReadErrors is the second Codex finding on
// PR #2589 (chain.go): a discovered lockfile that cannot be read was
// reported exactly like no lockfile at all. A directory standing
// where the lockfile should be is the portable unreadable file.
func TestLockfileTextPropagatesReadErrors(t *testing.T) {
	m := &deps.Manifest{Lockfiles: []string{t.TempDir()}}
	if _, _, err := m.LockfileText(); err == nil {
		t.Fatal("an unreadable lockfile must return its read error, not an empty lockfile")
	}
	none := &deps.Manifest{}
	name, text, err := none.LockfileText()
	if err != nil || name != "" || text != nil {
		t.Fatalf("no lockfile = (%q, %q, %v), want empty and nil error", name, text, err)
	}
}

// TestLockfilesTextReturnsEveryLockfile is the transport half of the
// Codex finding on PR #2589 (chain.go): LockfileText returned only
// the first discovered lockfile (bun.lock, by discovery order), so a
// node run's package-lock.json never left the caller's machine.
func TestLockfilesTextReturnsEveryLockfile(t *testing.T) {
	dir := t.TempDir()
	bun := filepath.Join(dir, "bun.lock")
	npm := filepath.Join(dir, "package-lock.json")
	if err := os.WriteFile(bun, []byte("bun"), 0o600); err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(npm, []byte("npm"), 0o600); err != nil {
		t.Fatal(err)
	}
	m := &deps.Manifest{Lockfiles: []string{bun, npm}}
	locks, err := m.LockfilesText()
	if err != nil {
		t.Fatal(err)
	}
	if len(locks) != 2 || locks[0].Name != "bun.lock" || locks[1].Name != "package-lock.json" {
		t.Fatalf("lockfiles = %+v", locks)
	}
	if string(locks[0].Text) != "bun" || string(locks[1].Text) != "npm" {
		t.Fatalf("lockfile contents = %+v", locks)
	}
}

// Codex round (PR #2589): abbreviated ranges keep their written
// precision — "~1" is not "~1.0.0" and "^0.0" is not "^0.0.0" — so a
// pin inside the abbreviated range must not be reported as a
// conflict (both cases verified against npm's semver).
func TestAbbreviatedRangesKeepTheirArity(t *testing.T) {
	project := t.TempDir()
	write(t, project, "package.json", `{"dependencies":{"tilde":"~1","caret":"^0.0","bare":"1.2"}}`)
	for _, spec := range []string{"tilde@1.9.0", "caret@0.0.5", "bare@1.2.9"} {
		c := cfg(t, deps.SourceCWD, project)
		c.Packages = []string{spec}
		set, err := deps.Resolve(c, "")
		if err != nil {
			t.Fatal(err)
		}
		if len(set.Conflicts) != 0 {
			t.Errorf("%s satisfies the abbreviated manifest range and must not conflict: %+v", spec, set.Conflicts)
		}
	}
	// And pins outside the abbreviated ranges still contradict them.
	for _, spec := range []string{"tilde@2.0.0", "caret@0.1.0", "bare@1.3.0"} {
		c := cfg(t, deps.SourceCWD, project)
		c.Packages = []string{spec}
		set, err := deps.Resolve(c, "")
		if err != nil {
			t.Fatal(err)
		}
		if len(set.Conflicts) != 1 {
			t.Errorf("%s is outside the abbreviated range and must conflict, got %+v", spec, set.Conflicts)
		}
	}
}

// TestOptionalDependenciesInManifestAndConflicts verifies that
// optionalDependencies are decoded and take precedence over dependencies
// of the same name (npm documented precedence), and that contradictory
// explicit pins are reported as conflicts (Codex review, PR #2589).
func TestOptionalDependenciesInManifestAndConflicts(t *testing.T) {
	project := t.TempDir()
	write(t, project, "package.json", `{
		"dependencies": {"shared": "1.0.0", "plain": "1.0.0"},
		"optionalDependencies": {"shared": "2.0.0", "opt": "^2.0.0"}
	}`)
	m, err := deps.Load(filepath.Join(project, "package.json"))
	if err != nil {
		t.Fatal(err)
	}
	if m.Packages["shared"] != "2.0.0" {
		t.Fatalf("optionalDependencies must override dependencies: got %q, want 2.0.0", m.Packages["shared"])
	}
	if m.Packages["opt"] != "^2.0.0" {
		t.Fatalf("optionalDependencies must be included: got %q, want ^2.0.0", m.Packages["opt"])
	}
	// Explicit pin contradicting optionalDependencies must be reported as a conflict
	conflicts := deps.Conflicts([]string{"shared@1.0.0", "opt@1.0.0"}, m.Packages)
	if len(conflicts) != 2 {
		t.Fatalf("conflicts = %+v, want 2", conflicts)
	}
	// Compatible pin should not conflict
	noConflicts := deps.Conflicts([]string{"opt@2.1.0"}, m.Packages)
	if len(noConflicts) != 0 {
		t.Fatalf("compatible pin conflicted: %+v", noConflicts)
	}
}
