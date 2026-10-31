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

// TestAWildcardManifestRangeAdmitsMatchingPins is the Codex finding
// on PR #2589 (config.go): a manifest declaring foo: "1.x" or
// "1.2.*" was read as a plain version, the parse failed, and a
// compatible explicit pin (foo@1.2.3) was reported as a conflict.
// Component wildcards are ranges at their written precision.
func TestAWildcardManifestRangeAdmitsMatchingPins(t *testing.T) {
	project := t.TempDir()
	write(t, project, "package.json", `{"dependencies":{"foo":"1.x","bar":"1.2.*","baz":"1.2.3"}}`)
	for _, spec := range []string{"foo@1.2.3", "foo@1.9.9", "bar@1.2.0", "bar@1.2.9"} {
		c := cfg(t, deps.SourceCWD, project)
		c.Packages = []string{spec}
		set, err := deps.Resolve(c, "")
		if err != nil {
			t.Fatal(err)
		}
		if len(set.Conflicts) != 0 {
			t.Errorf("%s satisfies the wildcard range and must not conflict: %+v", spec, set.Conflicts)
		}
	}
	// And pins outside the wildcard still contradict it.
	for _, spec := range []string{"foo@2.0.0", "bar@1.3.0", "bar@2.2.1"} {
		c := cfg(t, deps.SourceCWD, project)
		c.Packages = []string{spec}
		set, err := deps.Resolve(c, "")
		if err != nil {
			t.Fatal(err)
		}
		if len(set.Conflicts) != 1 {
			t.Errorf("%s is outside the wildcard range and must conflict, got %+v", spec, set.Conflicts)
		}
	}
}

// TestADisjunctiveManifestRangeAdmitsEitherAlternative is the Codex
// finding on PR #2589 (config.go): the range check evaluated every
// whitespace-separated token as one conjunction, so a manifest declaring
// "^1.0.0 || ^2.0.0" contradicted a foo@2.3.0 pin its second alternative
// allows. A disjunction is satisfied when ANY alternative set is.
func TestADisjunctiveManifestRangeAdmitsEitherAlternative(t *testing.T) {
	project := t.TempDir()
	write(t, project, "package.json", `{"dependencies":{"foo":"^1.0.0 || ^2.0.0","bar":">=1.0.0 <2.0.0 || >=3.0.0"}}`)
	for _, spec := range []string{"foo@1.9.4", "foo@2.3.0", "bar@1.5.0", "bar@3.2.0"} {
		c := cfg(t, deps.SourceCWD, project)
		c.Packages = []string{spec}
		set, err := deps.Resolve(c, "")
		if err != nil {
			t.Fatal(err)
		}
		if len(set.Conflicts) != 0 {
			t.Errorf("%s satisfies one alternative of the manifest range and must not conflict: %+v", spec, set.Conflicts)
		}
	}
	for _, spec := range []string{"foo@3.0.0", "bar@2.5.0"} {
		c := cfg(t, deps.SourceCWD, project)
		c.Packages = []string{spec}
		set, err := deps.Resolve(c, "")
		if err != nil {
			t.Fatal(err)
		}
		if len(set.Conflicts) != 1 {
			t.Errorf("%s satisfies no alternative and must conflict, got %+v", spec, set.Conflicts)
		}
	}
}

// TestAbbreviatedRelationalBoundsDesugarAtWrittenPrecision is the
// Codex finding on PR #2589 (config.go, ace6c3609 round): the relational
// branches zero-filled abbreviated bounds, so foo: ">1" admitted
// foo@1.0.1. npm desugars >1 to >=2.0.0, >1.2 to >=1.3.0, <=1 to
// <2.0.0 and <=1.2 to <1.3.0.
func TestAbbreviatedRelationalBoundsDesugarAtWrittenPrecision(t *testing.T) {
	project := t.TempDir()
	write(t, project, "package.json", `{"dependencies":{"gt":">1","gtminor":">1.2","le":"<=1","leminor":"<=1.2"}}`)
	for _, spec := range []string{"gt@2.0.0", "gt@3.1.0", "gtminor@1.3.0", "le@1.9.9", "leminor@1.2.9"} {
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
	for _, spec := range []string{"gt@1.0.1", "gt@1.9.9", "gtminor@1.2.9", "le@2.0.0", "leminor@1.3.0"} {
		c := cfg(t, deps.SourceCWD, project)
		c.Packages = []string{spec}
		set, err := deps.Resolve(c, "")
		if err != nil {
			t.Fatal(err)
		}
		if len(set.Conflicts) != 1 {
			t.Errorf("%s is outside the manifest range and must conflict, got %+v", spec, set.Conflicts)
		}
	}
}

// TestZeroMajorCaretWildcardsKeepTheirTighterBound is the Codex finding
// on PR #2589 (config.go, 511ab7adb round): satisfiesWildcard always
// incremented the major component for ^, so ^0.2.x admitted 0.9.0 and
// ^0.0.x admitted 0.1.0. Zero-major carets keep the left-most non-zero
// component fixed: ^0.2.x is >=0.2.0 <0.3.0, ^0.0.x is >=0.0.0 <0.1.0.
func TestZeroMajorCaretWildcardsKeepTheirTighterBound(t *testing.T) {
	project := t.TempDir()
	write(t, project, "package.json", `{"dependencies":{"a":"^0.2.x","b":"^0.0.x","c":"^1.2.x","d":"^0.x"}}`)
	for _, spec := range []string{"a@0.2.0", "a@0.2.9", "b@0.0.0", "b@0.0.7", "c@1.2.0", "c@1.9.9", "d@0.5.0"} {
		c := cfg(t, deps.SourceCWD, project)
		c.Packages = []string{spec}
		set, err := deps.Resolve(c, "")
		if err != nil {
			t.Fatal(err)
		}
		if len(set.Conflicts) != 0 {
			t.Errorf("%s satisfies the wildcard range and must not conflict: %+v", spec, set.Conflicts)
		}
	}
	for _, spec := range []string{"a@0.9.0", "a@0.3.0", "b@0.1.0", "c@2.0.0", "d@1.0.0"} {
		c := cfg(t, deps.SourceCWD, project)
		c.Packages = []string{spec}
		set, err := deps.Resolve(c, "")
		if err != nil {
			t.Fatal(err)
		}
		if len(set.Conflicts) != 1 {
			t.Errorf("%s is outside the wildcard range and must conflict, got %+v", spec, set.Conflicts)
		}
	}
}

// TestRelationalWildcardsDesugarAtWrittenPrecision is the Codex finding
// on PR #2589 (config.go, f1fa5ed0c round): satisfiesWildcard implemented
// only bare, =, ^ and ~ operators, so a manifest range like ">=1.x" or
// "<2.x" reported every explicit pin as a conflict. Relational wildcard
// bounds desugar at their written precision: >=1.x is >=1.0.0, <2.x is
// <2.0.0, >1.2.x is >=1.3.0 and <=1.x is <2.0.0.
func TestRelationalWildcardsDesugarAtWrittenPrecision(t *testing.T) {
	project := t.TempDir()
	write(t, project, "package.json", `{"dependencies":{"foo":">=1.x","bar":"<2.x","baz":">1.2.x","qux":"<=1.x"}}`)
	for _, spec := range []string{"foo@1.5.0", "foo@1.0.0", "bar@1.9.9", "baz@1.3.0", "baz@2.0.0", "qux@1.9.9"} {
		c := cfg(t, deps.SourceCWD, project)
		c.Packages = []string{spec}
		set, err := deps.Resolve(c, "")
		if err != nil {
			t.Fatal(err)
		}
		if len(set.Conflicts) != 0 {
			t.Errorf("%s satisfies the relational wildcard range and must not conflict: %+v", spec, set.Conflicts)
		}
	}
	for _, spec := range []string{"foo@0.9.9", "bar@2.0.0", "baz@1.2.9", "qux@2.0.0"} {
		c := cfg(t, deps.SourceCWD, project)
		c.Packages = []string{spec}
		set, err := deps.Resolve(c, "")
		if err != nil {
			t.Fatal(err)
		}
		if len(set.Conflicts) != 1 {
			t.Errorf("%s is outside the relational wildcard range and must conflict, got %+v", spec, set.Conflicts)
		}
	}
}

// TestManifestNPMRCTextReadsBesideTheManifest covers the transport
// added for the Codex finding on PR #2589 (runner/deps.go,
// f1fa5ed0c round): the project's .npmrc must travel with the
// manifest and lockfile, and its absence is a nil, not an error.
func TestManifestNPMRCTextReadsBesideTheManifest(t *testing.T) {
	dir := t.TempDir()
	m := &deps.Manifest{Dir: dir, Path: filepath.Join(dir, "package.json")}
	if b, err := m.NPMRCText(); err != nil || b != nil {
		t.Fatalf("absent .npmrc = %q, %v; want nil, nil", b, err)
	}
	write(t, dir, ".npmrc", "legacy-peer-deps=true\n")
	b, err := m.NPMRCText()
	if err != nil || string(b) != "legacy-peer-deps=true\n" {
		t.Fatalf("NPMRCText = %q, %v", b, err)
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

// TestConfigManifestOriginPreservedWithoutALockfile is the Codex finding
// on PR #2589 (chain.go): with deps.source=config and no project
// manifest, a config manifest carrying a relative file: declaration but
// NO lockfile was discarded entirely, so its directory never reached
// the transport as ManifestDir and the runner resolved the artifact
// against the script's directory instead. The origin must be preserved
// whether or not a lockfile exists; only Frozen depends on the lockfile.
func TestConfigManifestOriginPreservedWithoutALockfile(t *testing.T) {
	configDir := t.TempDir()
	t.Setenv("XDG_CONFIG_HOME", configDir)
	write(t, filepath.Join(configDir, "mcpx"), "package.json", `{"dependencies":{"localpkg":"file:./localpkg"}}`)

	set, err := deps.Resolve(cfg(t, deps.SourceConfig, t.TempDir()), t.TempDir())
	if err != nil {
		t.Fatal(err)
	}
	if set.Manifest == nil {
		t.Fatal("config manifest was discarded for lack of a lockfile; its origin is lost")
	}
	if want := filepath.Join(configDir, "mcpx"); set.Manifest.Dir != want {
		t.Fatalf("manifest dir = %q, want the configuration directory %q", set.Manifest.Dir, want)
	}
	if set.Frozen {
		t.Error("no lockfile beside the config manifest, but the set is frozen")
	}
}

// TestConfigPackageDirsPreservedWhenProjectManifestExists is the Codex
// finding on PR #2589 (chain.go): with deps.source=config AND a project
// manifest, a config-only package's origin was lost -- the set's single
// ManifestDir names the project's directory, so the runner resolved the
// config layer's relative file: artifacts against the project. The
// per-package origin must survive resolution.
func TestConfigPackageDirsPreservedWhenProjectManifestExists(t *testing.T) {
	project := t.TempDir()
	write(t, project, "package.json", `{"dependencies":{"proj-pkg":"1.0.0"}}`)

	xdg := t.TempDir()
	t.Setenv("XDG_CONFIG_HOME", xdg)
	cfgDir := filepath.Join(xdg, "mcpx")
	write(t, cfgDir, "package.json", `{"dependencies":{"helper":"file:./helper","proj-pkg":"9.9.9"}}`)

	set, err := deps.Resolve(cfg(t, deps.SourceConfig), project)
	if err != nil {
		t.Fatal(err)
	}
	if set.Manifest == nil || set.Manifest.Dir != project {
		t.Fatalf("selected manifest = %+v, want the project's", set.Manifest)
	}
	if got := set.PackageDirs["helper"]; got != cfgDir {
		t.Fatalf("PackageDirs[helper] = %q, want the configuration directory %q", got, cfgDir)
	}
	if _, ok := set.PackageDirs["proj-pkg"]; ok {
		t.Fatal("proj-pkg is supplied by the project manifest; it must not claim a config origin")
	}
}

// TestConflictsSpacedComparatorsAndHyphenRanges is the Codex finding
// on PR #2589 (deps/config.go): a manifest range written ">= 1.0.0"
// or as a hyphen range "1.0.0 - 2.0.0" is valid npm syntax, and a pin
// inside it is not a conflict. Tokenising on raw whitespace treated
// ">=" as a comparator with an empty operand and rejected both forms.
func TestConflictsSpacedComparatorsAndHyphenRanges(t *testing.T) {
	for _, tc := range []struct {
		name     string
		manifest string
		explicit string
		conflict bool
	}{
		{"spaced >= satisfied", ">= 1.0.0", "foo@1.5.0", false},
		{"spaced >= violated", ">= 1.0.0", "foo@0.9.0", true},
		{"spaced <= satisfied", "<= 2.0.0", "foo@1.5.0", false},
		{"hyphen range inside", "1.0.0 - 2.0.0", "foo@1.5.0", false},
		{"hyphen range above", "1.0.0 - 2.0.0", "foo@2.5.0", true},
		{"hyphen range below", "1.0.0 - 2.0.0", "foo@0.5.0", true},
		{"conjunction spaced", ">= 1.0.0 < 2.0.0", "foo@1.5.0", false},
	} {
		t.Run(tc.name, func(t *testing.T) {
			got := deps.Conflicts([]string{tc.explicit}, map[string]string{"foo": tc.manifest})
			if tc.conflict && len(got) != 1 {
				t.Fatalf("Conflicts(%q against %q) = %v, want one conflict", tc.explicit, tc.manifest, got)
			}
			if !tc.conflict && len(got) != 0 {
				t.Fatalf("Conflicts(%q against %q) = %v, want none", tc.explicit, tc.manifest, got)
			}
		})
	}
}
