package e2e_test

import (
	"encoding/json"
	"errors"
	"fmt"
	"io"
	"net/http"
	"os"
	"path/filepath"
	"sort"
	"strings"
	"sync"
	"testing"

	"github.com/developing-today/code/pkgs/mcpx/internal/api"
	"github.com/developing-today/code/pkgs/mcpx/internal/cli"
	"github.com/developing-today/code/pkgs/mcpx/internal/settings"
)

const (
	featureLedgerEnv = "MCPX_FEATURE_COVERAGE_LEDGER"
	// featureGuardEnv turns TestFeatureCoverageGuard from inert into the
	// CI gate. It is set only for the second `go test` invocation, the one
	// that reads the ledger the full suite wrote (#2691).
	featureGuardEnv = "MCPX_FEATURE_COVERAGE_GUARD"
	// featureExemptionsPath is checked in: one reasoned exemption per
	// feature that has no behavioural integration test yet.
	featureExemptionsPath = "testdata/feature-coverage-exemptions.json"
)

type featureCoverageRow struct {
	Test string `json:"test"`
	Kind string `json:"kind"`
	Name string `json:"name"`
}

type coverageFeature struct {
	Kind string
	Name string
}

var featureCoverage = struct {
	sync.Mutex
	rows map[featureCoverageRow]struct{}
}{rows: make(map[featureCoverageRow]struct{})}

func recordFeature(t *testing.T, kind, name string) {
	t.Helper()
	recordFeatureForTest(t.Name(), kind, name)
}

func recordFeatureForTest(test, kind, name string) {
	if test == "" || kind == "" || name == "" {
		return
	}
	row := featureCoverageRow{Test: test, Kind: kind, Name: name}
	featureCoverage.Lock()
	featureCoverage.rows[row] = struct{}{}
	featureCoverage.Unlock()
}

func featureCoverageSnapshot() []featureCoverageRow {
	featureCoverage.Lock()
	defer featureCoverage.Unlock()
	rows := make([]featureCoverageRow, 0, len(featureCoverage.rows))
	for row := range featureCoverage.rows {
		rows = append(rows, row)
	}
	sort.Slice(rows, func(i, j int) bool {
		if rows[i].Kind != rows[j].Kind {
			return rows[i].Kind < rows[j].Kind
		}
		if rows[i].Name != rows[j].Name {
			return rows[i].Name < rows[j].Name
		}
		return rows[i].Test < rows[j].Test
	})
	return rows
}

func writeFeatureCoverageLedger(path string, rows []featureCoverageRow) error {
	if path == "" {
		path = filepath.Join(os.TempDir(), fmt.Sprintf("mcpx-feature-coverage-%d.jsonl", os.Getpid()))
	}
	if err := os.MkdirAll(filepath.Dir(path), 0o755); err != nil {
		return fmt.Errorf("create feature coverage ledger directory: %w", err)
	}
	f, err := os.Create(path)
	if err != nil {
		return fmt.Errorf("create feature coverage ledger: %w", err)
	}
	enc := json.NewEncoder(f)
	for _, row := range rows {
		if err := enc.Encode(row); err != nil {
			_ = f.Close()
			return fmt.Errorf("write feature coverage ledger: %w", err)
		}
	}
	if err := f.Close(); err != nil {
		return fmt.Errorf("close feature coverage ledger: %w", err)
	}
	return nil
}

func TestMain(m *testing.M) {
	code := m.Run()
	// The guard invocation reads the ledger the full suite wrote. Writing
	// here would replace those rows with the guard run's (zero) rows.
	if os.Getenv(featureGuardEnv) == "1" {
		os.Exit(code)
	}
	path := os.Getenv(featureLedgerEnv)
	if path == "" {
		path = filepath.Join(os.TempDir(), fmt.Sprintf("mcpx-feature-coverage-%d.jsonl", os.Getpid()))
	}
	rows := featureCoverageSnapshot()
	if err := writeFeatureCoverageLedger(path, rows); err != nil {
		fmt.Fprintln(os.Stderr, err)
		code = 1
	} else {
		fmt.Printf("feature coverage ledger: %s (%d rows)\n", path, len(rows))
	}
	os.Exit(code)
}

func recordSettingProbes(t *testing.T) {
	for path, test := range settings.EffectProbes {
		if test == t.Name() {
			recordFeature(t, "setting", path)
		}
	}
}

func (e *env) recordCLI(args []string) {
	name := resolveCLICommand(args, e.dir)
	if name == "" {
		return
	}
	recordFeature(e.t, "cli", name)
}

func resolveCLICommand(args []string, dir string) string {
	handlers := (&cli.App{}).Handlers()
	for i := 0; i < len(args); i++ {
		arg := args[i]
		switch arg {
		case "--json", "--skip-default", "--all-profiles":
			continue
		case "--tui":
			return "tui"
		case "--help", "-h":
			return "help"
		case "--version", "-v":
			return ""
		case "--profile", "--config", "--paths-config":
			i++
			continue
		}
		if strings.HasPrefix(arg, "--profile=") || strings.HasPrefix(arg, "--config=") || strings.HasPrefix(arg, "--paths-config=") {
			continue
		}
		if strings.HasPrefix(arg, "--") {
			flagName, _, hasValue := strings.Cut(strings.TrimPrefix(arg, "--"), "=")
			setting, ok := settingForFlag(flagName)
			if !ok {
				return ""
			}
			if !hasValue && setting.Kind != settings.KindBool {
				i++
			}
			continue
		}
		if strings.HasPrefix(arg, "-") {
			return ""
		}
		if _, ok := handlers[arg]; ok {
			return arg
		}
		if runnableCommand(arg, dir) == "run" {
			return "run"
		}
		if runnableCommand(arg, dir) == "exec" {
			return "exec"
		}
		return ""
	}
	return ""
}

func settingForFlag(name string) (settings.Setting, bool) {
	for _, setting := range settings.Registry() {
		for _, candidate := range append([]string{setting.FlagName()}, setting.FlagAliases...) {
			if candidate == name {
				return setting, true
			}
		}
	}
	return settings.Setting{}, false
}

func runnableCommand(arg, dir string) string {
	if arg == "" || strings.HasPrefix(arg, "-") {
		return ""
	}
	for _, ext := range []string{".ts", ".js", ".mts", ".mjs"} {
		if strings.HasSuffix(arg, ext) {
			return "run"
		}
	}
	if strings.ContainsAny(arg, "(){};") || strings.Contains(arg, "await ") ||
		strings.Contains(arg, "console.") || strings.Contains(arg, "tools.") || strings.Contains(arg, "=>") {
		return "exec"
	}
	path := arg
	if !filepath.IsAbs(path) {
		path = filepath.Join(dir, path)
	}
	if st, err := os.Stat(path); err == nil && !st.IsDir() {
		return "run"
	}
	return ""
}

func matchAPIOperation(method, path string) string {
	for _, op := range api.Ops() {
		if op.Method != method || !matchRoutePath(op.Path, path) {
			continue
		}
		return op.Name
	}
	return ""
}

func matchRoutePath(pattern, path string) bool {
	want := strings.Split(strings.Trim(pattern, "/"), "/")
	got := strings.Split(strings.Trim(path, "/"), "/")
	if len(want) != len(got) {
		return false
	}
	for i := range want {
		if strings.HasPrefix(want[i], "{") && strings.HasSuffix(want[i], "}") {
			continue
		}
		if want[i] != got[i] {
			return false
		}
	}
	return true
}

type coverageRoundTripper struct {
	base http.RoundTripper
	test string
}

func (rt coverageRoundTripper) RoundTrip(req *http.Request) (*http.Response, error) {
	resp, err := rt.base.RoundTrip(req)
	if err == nil && resp != nil {
		if op := matchAPIOperation(req.Method, req.URL.Path); op != "" {
			recordFeatureForTest(rt.test, "op", op)
		}
	}
	return resp, err
}

func coverageViolations(required []coverageFeature, rows []featureCoverageRow, exemptions map[coverageFeature]string) []string {
	covered := make(map[coverageFeature]bool, len(rows))
	for _, row := range rows {
		covered[coverageFeature{Kind: row.Kind, Name: row.Name}] = true
	}
	var violations []string
	for feature, reason := range exemptions {
		if strings.TrimSpace(reason) == "" {
			violations = append(violations, fmt.Sprintf("%s %s has an empty exemption reason", feature.Kind, feature.Name))
		}
	}
	for _, feature := range required {
		if covered[feature] {
			if _, exempt := exemptions[feature]; exempt {
				violations = append(violations, fmt.Sprintf("%s %s is both exercised and exempt", feature.Kind, feature.Name))
			}
			continue
		}
		if _, exempt := exemptions[feature]; !exempt {
			violations = append(violations, fmt.Sprintf("%s %s has no integration-test ledger row or reasoned exemption", feature.Kind, feature.Name))
		}
	}
	for feature := range exemptions {
		found := false
		for _, candidate := range required {
			if candidate == feature {
				found = true
				break
			}
		}
		if !found {
			violations = append(violations, fmt.Sprintf("exemption names unknown %s %s", feature.Kind, feature.Name))
		}
	}
	sort.Strings(violations)
	return violations
}

func TestFeatureCoverageGuardFailsForFabricatedGap(t *testing.T) {
	required := []coverageFeature{{Kind: "op", Name: "fabricated_uncovered_operation"}}
	if got := coverageViolations(required, nil, nil); len(got) != 1 || !strings.Contains(got[0], "fabricated_uncovered_operation") {
		t.Fatalf("fabricated uncovered feature should fail the guard, got %v", got)
	}
}

func TestFeatureCoverageLedgerWritesJSONLines(t *testing.T) {
	path := filepath.Join(t.TempDir(), "ledger.jsonl")
	rows := []featureCoverageRow{
		{Test: "TestA", Kind: "op", Name: "status"},
		{Test: "TestB", Kind: "cli", Name: "ls"},
	}
	if err := writeFeatureCoverageLedger(path, rows); err != nil {
		t.Fatal(err)
	}
	body, err := os.ReadFile(path)
	if err != nil {
		t.Fatal(err)
	}
	lines := strings.Split(strings.TrimSpace(string(body)), "\n")
	if len(lines) != len(rows) {
		t.Fatalf("got %d JSON lines, want %d: %s", len(lines), len(rows), body)
	}
	for i, line := range lines {
		var got featureCoverageRow
		if err := json.Unmarshal([]byte(line), &got); err != nil {
			t.Fatalf("line %d is not JSON: %v", i+1, err)
		}
		if got != rows[i] {
			t.Errorf("line %d = %+v, want %+v", i+1, got, rows[i])
		}
	}
}

func TestFeatureCoverageMatchesParameterizedAPIOperation(t *testing.T) {
	if got := matchAPIOperation(http.MethodGet, "/v1/ask/example-id"); got != "ask_poll" {
		t.Fatalf("matched operation %q, want ask_poll", got)
	}
}

func TestResolveCLICommandAfterGlobalFlags(t *testing.T) {
	if got := resolveCLICommand([]string{"--json", "--profile", "dev", "list"}, t.TempDir()); got != "list" {
		t.Fatalf("resolved command %q, want alias list", got)
	}
	if got := resolveCLICommand([]string{"--plumbing-strict-unknown-keys", "ls"}, t.TempDir()); got != "ls" {
		t.Fatalf("resolved command %q after boolean setting flag, want ls", got)
	}
}

// featureExemption is one checked-in reason a feature has no behavioural
// integration test yet. Kind is one of "op" or "cli"; settings reuse the
// reasoned EffectExemptions in internal/settings, which the effect guard
// already ratchets (#1971), rather than keeping a second copy here.
type featureExemption struct {
	Kind   string `json:"kind"`
	Name   string `json:"name"`
	Reason string `json:"reason"`
}

type featureExemptionDoc struct {
	Exemptions []featureExemption `json:"exemptions"`
}

func readFeatureCoverageLedger(path string) ([]featureCoverageRow, error) {
	f, err := os.Open(path)
	if err != nil {
		return nil, fmt.Errorf("open feature coverage ledger: %w", err)
	}
	defer f.Close()
	var rows []featureCoverageRow
	dec := json.NewDecoder(f)
	for {
		var row featureCoverageRow
		if err := dec.Decode(&row); err != nil {
			if errors.Is(err, io.EOF) {
				break
			}
			return nil, fmt.Errorf("decode feature coverage ledger: %w", err)
		}
		if row.Kind == "" || row.Name == "" {
			continue
		}
		rows = append(rows, row)
	}
	return rows, nil
}

func loadFeatureExemptions(t *testing.T) map[coverageFeature]string {
	t.Helper()
	body, err := os.ReadFile(featureExemptionsPath)
	if err != nil {
		t.Fatalf("%s should exist and carry one reason per uncovered feature: %v", featureExemptionsPath, err)
	}
	var doc featureExemptionDoc
	if err := json.Unmarshal(body, &doc); err != nil {
		t.Fatalf("%s is not valid JSON: %v", featureExemptionsPath, err)
	}
	out := make(map[coverageFeature]string, len(doc.Exemptions)+len(settings.EffectExemptions))
	for _, e := range doc.Exemptions {
		key := coverageFeature{Kind: e.Kind, Name: e.Name}
		if _, dup := out[key]; dup {
			t.Fatalf("%s lists %s %s twice", featureExemptionsPath, e.Kind, e.Name)
		}
		out[key] = e.Reason
	}
	for path, reason := range settings.EffectExemptions {
		out[coverageFeature{Kind: "setting", Name: path}] = reason
	}
	return out
}

// requiredFeatures is the inventory the guard enforces: every /v1
// operation, every CLI command and alias the dispatcher resolves, and
// every setting in the registry. Aliases count because the ledger records
// the word the test typed.
func requiredFeatures() []coverageFeature {
	seen := make(map[coverageFeature]bool)
	add := func(kind, name string) {
		if name != "" {
			seen[coverageFeature{Kind: kind, Name: name}] = true
		}
	}
	for _, op := range api.Ops() {
		add("op", op.Name)
	}
	for _, command := range cli.Commands() {
		add("cli", command.Name)
		for _, alias := range command.Aliases {
			add("cli", alias)
		}
	}
	for name := range (&cli.App{}).Handlers() {
		add("cli", name)
	}
	for _, setting := range settings.Registry() {
		add("setting", setting.Path)
	}
	out := make([]coverageFeature, 0, len(seen))
	for feature := range seen {
		out = append(out, feature)
	}
	sort.Slice(out, func(i, j int) bool {
		if out[i].Kind != out[j].Kind {
			return out[i].Kind < out[j].Kind
		}
		return out[i].Name < out[j].Name
	})
	return out
}

// TestFeatureCoverageGuard is the enforcement half of Approach C (#2691,
// #1978): a feature in the inventory that no e2e test exercised at run
// time — and that carries no reasoned exemption — fails CI. The ledger it
// reads is written by TestMain from the hooks in this file (CLI dispatch,
// socket client calls, settings effect probes); the CI job runs the full
// suite first and this test second, so the ledger is complete.
func TestFeatureCoverageGuard(t *testing.T) {
	if os.Getenv(featureGuardEnv) != "1" {
		// The full-suite invocation cannot see its own ledger: TestMain
		// writes it after m.Run returns. CI therefore runs this test in a
		// second, cheap invocation with featureGuardEnv set; in the full
		// suite it is inert rather than a skip, so the repository's
		// skip-record check (MCPX_GO_TESTS) stays accurate.
		t.Log("feature-coverage gate: enforced by the dedicated CI step with " + featureGuardEnv + "=1")
		return
	}
	path := os.Getenv(featureLedgerEnv)
	if path == "" {
		t.Fatalf("%s must name the ledger the full suite wrote", featureLedgerEnv)
	}
	rows, err := readFeatureCoverageLedger(path)
	if err != nil {
		t.Fatalf("the guard needs a ledger from a full e2e run: %v", err)
	}
	if len(rows) == 0 {
		t.Fatalf("ledger %s has no rows; run the full e2e suite with %s set before the guard", path, featureLedgerEnv)
	}
	violations := coverageViolations(requiredFeatures(), rows, loadFeatureExemptions(t))
	if len(violations) > 0 {
		t.Fatalf("%d feature(s) have neither a runtime ledger row nor a reasoned exemption:\n  %s",
			len(violations), strings.Join(violations, "\n  "))
	}
}

// TestFeatureCoveragePluginOpsAreCurrent keeps the plugin op surface in the
// same guard: the generated ops.gen.ts is one of the four surfaces the
// inventory enumerates, so a stale generation is a coverage gap too.
func TestFeatureCoveragePluginOpsAreCurrent(t *testing.T) {
	path := filepath.Join("..", "..", "plugin", "opencode", "mcpx", "ops.gen.ts")
	got, err := os.ReadFile(path)
	if err != nil {
		t.Fatalf("%s should exist: %v", path, err)
	}
	if string(got) != api.PluginTS() {
		t.Errorf("%s is stale; regenerate it with\n  go test ./internal/api -run TestPluginOpsAreGenerated -update", path)
	}
}

// TestCoverageViolationsEdges pins the guard's refusal modes, so an
// exemption cannot quietly become a bypass.
func TestCoverageViolationsEdges(t *testing.T) {
	mini := []coverageFeature{{Kind: "op", Name: "status"}}
	if v := coverageViolations(mini, []featureCoverageRow{{Test: "TestA", Kind: "op", Name: "status"}}, nil); len(v) != 0 {
		t.Fatalf("a covered feature should pass, got %v", v)
	}
	if v := coverageViolations(mini, nil, map[coverageFeature]string{{Kind: "op", Name: "status"}: "reason"}); len(v) != 0 {
		t.Fatalf("an exempt feature with a reason should pass, got %v", v)
	}
	cases := []struct {
		name string
		rows []featureCoverageRow
		ex   map[coverageFeature]string
		want string
	}{
		{"uncovered", nil, nil, "no integration-test ledger row"},
		{"empty reason", nil, map[coverageFeature]string{{Kind: "op", Name: "status"}: "  "}, "empty exemption reason"},
		{"covered and exempt", []featureCoverageRow{{Kind: "op", Name: "status"}}, map[coverageFeature]string{{Kind: "op", Name: "status"}: "reason"}, "both exercised and exempt"},
		{"unknown exemption", nil, map[coverageFeature]string{{Kind: "op", Name: "gone"}: "reason"}, "unknown op gone"},
	}
	for _, tc := range cases {
		t.Run(tc.name, func(t *testing.T) {
			v := coverageViolations(mini, tc.rows, tc.ex)
			if len(v) == 0 || !strings.Contains(strings.Join(v, "\n"), tc.want) {
				t.Fatalf("violations %v do not mention %q", v, tc.want)
			}
		})
	}
}
