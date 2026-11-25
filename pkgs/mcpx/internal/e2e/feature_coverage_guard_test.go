package e2e_test

// The feature-coverage guard (#1978, Approach C from
// docs/testing/feature-inventory.md): the runtime ledger in
// feature_coverage_test.go records which features the suite actually
// exercised; this file enumerates what must be exercised — every
// registry surface — and evaluates the ledger against it where the
// full suite's rows exist (guardedMain, after m.Run()). A registry
// entry with no ledger row needs a reasoned exemption in
// feature_coverage_exemptions.txt; a feature with no ledger row on
// one GOOS only needs one in feature_coverage_exemptions_<goos>.txt
// instead, so the other platforms still block on it. Anything else
// is a violation and fails the run. Partial runs (-run filters) skip
// the evaluation: their ledger is incomplete by construction.

import (
	"encoding/json"
	"flag"
	"fmt"
	"os"
	"path/filepath"
	"regexp"
	"runtime"
	"sort"
	"strings"
	"testing"

	"github.com/developing-today/code/pkgs/mcpx/internal/api"
	"github.com/developing-today/code/pkgs/mcpx/internal/cli"
	"github.com/developing-today/code/pkgs/mcpx/internal/settings"
)

// requiredFeatures enumerates the full registry surface the ledger
// is held to: every /v1 op, every CLI command (canonical names from
// cli.Commands plus the alias keys the dispatcher actually resolves
// through App.Handlers), and every settings-registry path.
func requiredFeatures() []coverageFeature {
	seen := make(map[coverageFeature]bool)
	var required []coverageFeature
	add := func(kind, name string) {
		feature := coverageFeature{Kind: kind, Name: name}
		if name == "" || seen[feature] {
			return
		}
		seen[feature] = true
		required = append(required, feature)
	}
	for _, op := range api.Ops() {
		add("op", op.Name)
	}
	for _, command := range cli.Commands() {
		add("cli", command.Name)
	}
	for name := range (&cli.App{}).Handlers() {
		add("cli", name)
	}
	for _, setting := range settings.Registry() {
		add("setting", setting.Path)
	}
	sort.Slice(required, func(i, j int) bool {
		if required[i].Kind != required[j].Kind {
			return required[i].Kind < required[j].Kind
		}
		return required[i].Name < required[j].Name
	})
	return required
}

// pluginOpNames reads the plugin op surface — plugin/core/ops.gen.ts,
// generated from api.Ops() — and returns the op names it declares.
func pluginOpNames() ([]string, error) {
	root, err := moduleRoot()
	if err != nil {
		return nil, err
	}
	path := filepath.Join(root, "plugin", "core", "ops.gen.ts")
	body, err := os.ReadFile(path)
	if err != nil {
		return nil, fmt.Errorf("read plugin op surface: %w", err)
	}
	text := string(body)
	start := strings.Index(text, "export const OPS")
	if start < 0 {
		return nil, fmt.Errorf("plugin op surface %s has no OPS table", path)
	}
	entry := regexp.MustCompile(`(?m)^  ([a-z0-9_]+): \{`)
	var names []string
	for _, match := range entry.FindAllStringSubmatch(text[start:], -1) {
		names = append(names, match[1])
	}
	if len(names) == 0 {
		return nil, fmt.Errorf("plugin op surface %s declares no ops", path)
	}
	sort.Strings(names)
	return names, nil
}

// pluginSurfaceViolations asserts the generated plugin op surface is
// fresh: it must name exactly the ops api.Ops() declares. A stale
// ops.gen.ts means the plugin reaches a different surface than the
// daemon serves.
func pluginSurfaceViolations() []string {
	names, err := pluginOpNames()
	if err != nil {
		return []string{err.Error()}
	}
	inPlugin := make(map[string]bool, len(names))
	for _, name := range names {
		inPlugin[name] = true
	}
	var violations []string
	for _, op := range api.Ops() {
		if !inPlugin[op.Name] {
			violations = append(violations, fmt.Sprintf("plugin op surface is stale: op %s is declared by api.Ops() but missing from ops.gen.ts", op.Name))
		}
	}
	declared := make(map[string]bool)
	for _, op := range api.Ops() {
		declared[op.Name] = true
	}
	for _, name := range names {
		if !declared[name] {
			violations = append(violations, fmt.Sprintf("plugin op surface is stale: ops.gen.ts names unknown op %s", name))
		}
	}
	sort.Strings(violations)
	return violations
}

const featureExemptionsFile = "feature_coverage_exemptions.txt"

// loadFeatureExemptions reads the checked-in exemption file: one
// exemption per line, `kind name reason...`; blank lines and lines
// starting with # are comments. The reason is mandatory —
// coverageViolations treats an empty one as a violation, as it does
// an exemption for a feature that is covered or unknown.
func loadFeatureExemptions() (map[coverageFeature]string, error) {
	return loadFeatureExemptionsForGOOS(runtime.GOOS)
}

// platformFeatureExemptionsFile names the optional per-platform
// exemption overlay: features with no integration-test ledger row on
// that GOOS only (linux-tagged tests, /proc-gated skips), with a
// reasoned one-line entry each. The file may be absent — a platform
// with no ineligible features needs no overlay — but when present it
// parses like the base file and must not repeat a base entry.
func platformFeatureExemptionsFile(goos string) string {
	return fmt.Sprintf("feature_coverage_exemptions_%s.txt", goos)
}

// loadFeatureExemptionsForGOOS merges the base exemption file with
// the per-platform overlay for goos. The overlay lets a platform
// excuse its genuinely ineligible features truthfully while every
// other platform still blocks on them.
func loadFeatureExemptionsForGOOS(goos string) (map[coverageFeature]string, error) {
	root, err := moduleRoot()
	if err != nil {
		return nil, err
	}
	base, err := parseFeatureExemptionsFile(filepath.Join(root, "internal", "e2e", featureExemptionsFile))
	if err != nil {
		return nil, err
	}
	overlay, err := parseFeatureExemptionsFile(filepath.Join(root, "internal", "e2e", platformFeatureExemptionsFile(goos)))
	if err != nil {
		if os.IsNotExist(err) {
			return base, nil
		}
		return nil, err
	}
	for feature, reason := range overlay {
		if _, dup := base[feature]; dup {
			return nil, fmt.Errorf("%s: duplicate exemption for %s %s (already exempt in %s)", platformFeatureExemptionsFile(goos), feature.Kind, feature.Name, featureExemptionsFile)
		}
		base[feature] = reason
	}
	return base, nil
}

func parseFeatureExemptionsFile(path string) (map[coverageFeature]string, error) {
	body, err := os.ReadFile(path)
	if err != nil {
		return nil, err
	}
	return parseFeatureExemptions(string(body), filepath.Base(path))
}

func parseFeatureExemptions(body, source string) (map[coverageFeature]string, error) {
	exemptions := make(map[coverageFeature]string)
	for i, line := range strings.Split(body, "\n") {
		line = strings.TrimSpace(line)
		if line == "" || strings.HasPrefix(line, "#") {
			continue
		}
		fields := strings.Fields(line)
		if len(fields) < 2 {
			return nil, fmt.Errorf("%s:%d: exemption line needs at least a kind and a name: %q", source, i+1, line)
		}
		reason := ""
		if len(fields) > 2 {
			reason = strings.Join(fields[2:], " ")
		}
		feature := coverageFeature{Kind: fields[0], Name: fields[1]}
		if _, dup := exemptions[feature]; dup {
			return nil, fmt.Errorf("%s:%d: duplicate exemption for %s %s", source, i+1, feature.Kind, feature.Name)
		}
		exemptions[feature] = reason
	}
	return exemptions, nil
}

// moduleRoot finds the source module from the package test's working
// directory. runtime.Caller is not suitable here: Nix Go builds use
// -trimpath, which turns source paths into import paths rather than files.
func moduleRoot() (string, error) {
	dir, err := os.Getwd()
	if err != nil {
		return "", fmt.Errorf("get e2e test working directory: %w", err)
	}
	for {
		if _, err := os.Stat(filepath.Join(dir, "go.mod")); err == nil {
			return dir, nil
		}
		parent := filepath.Dir(dir)
		if parent == dir {
			return "", fmt.Errorf("go.mod not found from e2e test working directory %s", dir)
		}
		dir = parent
	}
}

// evaluateFeatureCoverage is the guard's whole evaluation: the
// ledger rows against the enumerated registries plus exemptions,
// and the plugin surface's freshness.
func evaluateFeatureCoverage(rows []featureCoverageRow) []string {
	exemptions, err := loadFeatureExemptions()
	if err != nil {
		return []string{err.Error()}
	}
	violations := coverageViolations(requiredFeatures(), rows, exemptions)
	return append(violations, pluginSurfaceViolations()...)
}

// featureCoverageGuardViolations is what guardedMain enforces after
// a full, passing suite run. It returns nil when the evaluation does
// not apply: a -run-filtered run's ledger is partial by construction,
// a -list run executes no tests at all, a -skip-filtered run omits
// the skipped tests' rows, and a failing run's rows are incomplete
// for the same reason its tests failed.
func featureCoverageGuardViolations(runCode int, rows []featureCoverageRow) []string {
	if runCode != 0 {
		return nil
	}
	for _, name := range []string{"test.run", "test.list", "test.skip"} {
		if f := flag.Lookup(name); f != nil && f.Value.String() != "" {
			return nil
		}
	}
	return evaluateFeatureCoverage(rows)
}

// TestFeatureCoverageGuardAgainstRecordedLedger evaluates the guard
// against a ledger recorded by an earlier full-suite run instead of
// this run's rows. It is the fast iteration loop for the exemption
// file (a full suite run is ~25 minutes) and the way a recorded
// ledger — e.g. the one CI uploads — can be re-judged after the fact.
// Set MCPX_FEATURE_COVERAGE_EVALUATE_LEDGER to the ledger path; when
// it is unset the test is a no-op pass, NOT a skip: the unfiltered CI
// job's skip-count ratchet (scripts/gotest-report.py decide(), keyed
// on the MCPX_GO_TESTS record) rejects a green run whose skip count
// exceeds the stored ceiling, so an opt-in utility must never add a
// permanent skip to normal runs (Codex finding on PR #8475).
func TestFeatureCoverageGuardAgainstRecordedLedger(t *testing.T) {
	path := os.Getenv("MCPX_FEATURE_COVERAGE_EVALUATE_LEDGER")
	if path == "" {
		return
	}
	body, err := os.ReadFile(path)
	if err != nil {
		t.Fatal(err)
	}
	var rows []featureCoverageRow
	for i, line := range strings.Split(strings.TrimSpace(string(body)), "\n") {
		if strings.TrimSpace(line) == "" {
			continue
		}
		var row featureCoverageRow
		if err := json.Unmarshal([]byte(line), &row); err != nil {
			t.Fatalf("ledger line %d is not a coverage row: %v", i+1, err)
		}
		rows = append(rows, row)
	}
	if violations := evaluateFeatureCoverage(rows); len(violations) != 0 {
		t.Fatalf("recorded ledger %s has %d guard violation(s):\n%s", path, len(violations), strings.Join(violations, "\n"))
	}
}

func TestFeatureCoverageRequiredSetEnumeratesTheRegistries(t *testing.T) {
	required := requiredFeatures()
	byKind := make(map[string]int)
	present := make(map[coverageFeature]bool)
	for _, feature := range required {
		byKind[feature.Kind]++
		present[feature] = true
	}
	for _, want := range []coverageFeature{
		{Kind: "op", Name: "health"},
		{Kind: "op", Name: "status"},
		{Kind: "cli", Name: "status"},
		{Kind: "cli", Name: "ls"},
		{Kind: "setting", Name: "daemon.port"},
	} {
		if !present[want] {
			t.Errorf("required set is missing %s %s", want.Kind, want.Name)
		}
	}
	if byKind["op"] != len(api.Ops()) {
		t.Errorf("required ops = %d, api.Ops() = %d", byKind["op"], len(api.Ops()))
	}
	if byKind["setting"] != len(settings.Registry()) {
		t.Errorf("required settings = %d, settings.Registry() = %d", byKind["setting"], len(settings.Registry()))
	}
	if byKind["cli"] < len(cli.Commands()) {
		t.Errorf("required cli = %d, cli.Commands() = %d (handlers may only add)", byKind["cli"], len(cli.Commands()))
	}
}

func TestFeatureCoveragePluginSurfaceIsFresh(t *testing.T) {
	if violations := pluginSurfaceViolations(); len(violations) != 0 {
		t.Fatalf("plugin op surface should be fresh on main, got %v", violations)
	}
}

func TestFeatureCoverageExemptionsAreWellFormed(t *testing.T) {
	exemptions, err := loadFeatureExemptions()
	if err != nil {
		t.Fatal(err)
	}
	if len(exemptions) == 0 {
		t.Fatal("the exemption file names no exemptions; either the suite covers everything (remove this expectation) or the file is not being read")
	}
	// Against an empty ledger every covered-check is vacuous, but two
	// violation classes must already be absent: empty reasons and
	// exemptions naming features the registries do not declare.
	for _, violation := range coverageViolations(requiredFeatures(), nil, exemptions) {
		if strings.Contains(violation, "empty exemption reason") || strings.Contains(violation, "exemption names unknown") {
			t.Errorf("malformed exemption: %s", violation)
		}
	}
}

func TestFeatureCoverageGuardFailsForFabricatedRegistryEntry(t *testing.T) {
	exemptions, err := loadFeatureExemptions()
	if err != nil {
		t.Fatal(err)
	}
	required := append(requiredFeatures(), coverageFeature{Kind: "op", Name: "fabricated_unexercised_op"})
	violations := coverageViolations(required, nil, exemptions)
	found := false
	for _, violation := range violations {
		if strings.Contains(violation, "fabricated_unexercised_op") {
			found = true
		}
	}
	if !found {
		t.Fatalf("a fabricated unexercised registry entry should fail the guard, got %v", violations)
	}
}

// TestFeatureCoverageGuardSkipsPartialExecutionModes pins the Codex
// finding on PR #8475: listing (-list) and skip-filtered (-skip) runs
// produce empty or partial ledgers without setting test.run, so the
// guard must not evaluate them.
func TestFeatureCoverageGuardSkipsPartialExecutionModes(t *testing.T) {
	for _, name := range []string{"test.list", "test.skip"} {
		f := flag.Lookup(name)
		if f == nil {
			t.Fatalf("flag %s not registered", name)
		}
		orig := f.Value.String()
		if err := flag.Set(name, "TestNothingMatchesThis"); err != nil {
			t.Fatalf("set %s: %v", name, err)
		}
		if violations := featureCoverageGuardViolations(0, nil); violations != nil {
			_ = flag.Set(name, orig)
			t.Fatalf("with %s set, guard returned %v, want nil (evaluation excluded)", name, violations)
		}
		if err := flag.Set(name, orig); err != nil {
			t.Fatalf("restore %s: %v", name, err)
		}
	}
	// A failing run is never evaluated either.
	if violations := featureCoverageGuardViolations(1, nil); violations != nil {
		t.Fatalf("failing run: guard returned %v, want nil", violations)
	}
}

// TestFeatureCoveragePlatformExemptionsAreScopedToDarwin pins the
// macOS stress-leg gap: `cli explore` is exercised only by a
// linux-tagged pty test (cli_inspection_explore_linux_test.go, via
// /dev/ptmx ioctls with no darwin equivalent) and `cli supervise`
// only by a test that skips without /proc
// (TestSuperviseKeepsTheChildAcrossADaemonStop) — so a full-suite
// darwin ledger legitimately lacks both rows while the linux ledger
// has them. Platform-scoped exemptions must excuse the two features
// on darwin only; linux coverage stays blocking.
func TestFeatureCoveragePlatformExemptionsAreScopedToDarwin(t *testing.T) {
	platformIneligible := map[coverageFeature]bool{
		{Kind: "cli", Name: "explore"}:   true,
		{Kind: "cli", Name: "supervise"}: true,
	}
	// Simulate a darwin full-suite ledger: every required feature is
	// covered except the two platform-ineligible ones — and except the
	// base-exempt features, which no ledger covers (marking them
	// covered would trip the "both exercised and exempt" violation).
	baseExempt, err := loadFeatureExemptionsForGOOS("linux")
	if err != nil {
		t.Fatalf("load base exemptions: %v", err)
	}
	var rows []featureCoverageRow
	for _, feature := range requiredFeatures() {
		if _, ok := baseExempt[feature]; platformIneligible[feature] || ok {
			continue
		}
		rows = append(rows, featureCoverageRow{Test: "TestSyntheticCoverage", Kind: feature.Kind, Name: feature.Name})
	}
	violationsFor := func(goos string) []string {
		exemptions, err := loadFeatureExemptionsForGOOS(goos)
		if err != nil {
			t.Fatalf("load exemptions for %s: %v", goos, err)
		}
		return coverageViolations(requiredFeatures(), rows, exemptions)
	}
	var linuxNames []string
	for _, violation := range violationsFor("linux") {
		for _, name := range []string{"explore", "supervise"} {
			if strings.Contains(violation, "cli "+name) {
				linuxNames = append(linuxNames, name)
			}
		}
	}
	sort.Strings(linuxNames)
	if strings.Join(linuxNames, ",") != "explore,supervise" {
		t.Fatalf("linux must still block both platform-gated features, got %v", violationsFor("linux"))
	}
	if violations := violationsFor("darwin"); len(violations) != 0 {
		t.Fatalf("darwin should excuse both platform-ineligible features, got %v", violations)
	}
}
