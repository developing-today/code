package e2e_test

import (
	"encoding/json"
	"fmt"
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

const featureLedgerEnv = "MCPX_FEATURE_COVERAGE_LEDGER"

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
