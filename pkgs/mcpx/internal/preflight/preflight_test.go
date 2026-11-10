package preflight_test

import (
	"context"
	"os"
	"path/filepath"
	"strings"
	"testing"
	"time"

	"github.com/developing-today/code/pkgs/mcpx/internal/preflight"
)

func TestEveryBadPathIsReportedInOnePass(t *testing.T) {
	// A run with three bad paths should report three problems, not the first
	// one and then two more runs.
	r := preflight.CheckPaths([]preflight.PathCheck{
		{Path: "/definitely/not/a.ts", Where: "script.prefix", MustExist: true, WantFile: true},
		{Path: "/definitely/not/b.ts", Where: "script.suffix", MustExist: true, WantFile: true},
		{Path: "/definitely/not/c.ts", Where: "script.launcher", MustExist: true, WantFile: true},
	})
	if len(r.Problems) != 3 {
		t.Fatalf("all three should be reported at once, got %d: %v", len(r.Problems), r.Problems)
	}
	err := r.Err()
	if err == nil {
		t.Fatal("missing required files should be fatal")
	}
	for _, want := range []string{"script.prefix", "script.suffix", "script.launcher"} {
		if !strings.Contains(err.Error(), want) {
			t.Errorf("the error should name %s: %v", want, err)
		}
	}
}

func TestAFileWhereADirectoryWasExpectedIsRefused(t *testing.T) {
	dir := t.TempDir()
	file := filepath.Join(dir, "f")
	if err := os.WriteFile(file, []byte("x"), 0o644); err != nil {
		t.Fatal(err)
	}
	r := preflight.CheckPaths([]preflight.PathCheck{
		{Path: file, Where: "logging.dir", WantDir: true},
	})
	if r.Err() == nil {
		t.Fatal("a file cannot serve as a log directory")
	}
	if !strings.Contains(r.Err().Error(), "directory was expected") {
		t.Errorf("the message should say what was wrong: %v", r.Err())
	}
}

func TestAMissingWritableDirectoryIsCreatedRatherThanRefused(t *testing.T) {
	// Refusing to start because a log directory does not exist yet would be
	// pedantic; the intent is unambiguous.
	dir := filepath.Join(t.TempDir(), "logs", "nested")
	r := preflight.CheckPaths([]preflight.PathCheck{
		{Path: dir, Where: "logging.dir", WantDir: true, Writable: true},
	})
	if err := r.Err(); err != nil {
		t.Fatalf("should have been created: %v", err)
	}
	if st, err := os.Stat(dir); err != nil || !st.IsDir() {
		t.Errorf("directory not created: %v", err)
	}
}

func TestAMissingOptionalPathIsAWarningNotAFailure(t *testing.T) {
	r := preflight.CheckPaths([]preflight.PathCheck{
		{Path: "/not/here", Where: "paths.scripts", WantDir: true},
	})
	if err := r.Err(); err != nil {
		t.Errorf("an optional missing path should not stop the run: %v", err)
	}
	if len(r.Warnings()) != 1 {
		t.Errorf("but it should still be mentioned: %v", r.Warnings())
	}
}

func TestASuggestedFixIsOfferedForAMissingDirectory(t *testing.T) {
	r := preflight.CheckPaths([]preflight.PathCheck{
		{Path: "/not/here", Where: "paths.scripts", WantDir: true},
	})
	if !strings.Contains(r.Warnings()[0].String(), "mkdir -p /not/here") {
		t.Errorf("the obvious remedy should be offered: %s", r.Warnings()[0])
	}
}

func TestMalformedEnvironmentPairsAreCaught(t *testing.T) {
	// Ignored, a typo silently fails to set the variable and the script
	// behaves as though it was never asked for.
	r := preflight.CheckEnvPairs([]string{"GOOD=1", "BROKEN", "=novalue", "HAS SPACE=1"}, "--env")
	if len(r.Problems) != 3 {
		t.Fatalf("three of the four are wrong, got %d: %v", len(r.Problems), r.Problems)
	}
	if r.Err() == nil {
		t.Fatal("a malformed pair should stop the run")
	}
}

func TestTypeCheckIsSkippedWhenOff(t *testing.T) {
	r := preflight.TypeCheck(context.TODO(), "/bin/deno", "/nope.ts", preflight.TypeCheckOff, 0)
	if len(r.Problems) != 0 {
		t.Errorf("off means off: %v", r.Problems)
	}
}

func TestScriptCheckModesSelectTypesAndStrictCommands(t *testing.T) {
	dir := t.TempDir()
	entry := filepath.Join(dir, "main.ts")
	if err := os.WriteFile(entry, []byte("export {};\n"), 0o644); err != nil {
		t.Fatal(err)
	}
	marker := filepath.Join(dir, "args")
	stub := filepath.Join(dir, "deno-stub")
	script := "#!/bin/sh\nprintf '%s\\n' \"$@\" > '" + marker + "'\n"
	if err := os.WriteFile(stub, []byte(script), 0o755); err != nil {
		t.Fatal(err)
	}

	for _, tc := range []struct {
		mode preflight.TypeCheckMode
		want []string
	}{
		{preflight.TypeCheckTypes, []string{"check", entry}},
		{preflight.TypeCheckStrict, []string{"check", "--all", entry}},
	} {
		t.Run(string(tc.mode), func(t *testing.T) {
			rep := preflight.TypeCheckResolved(context.Background(), stub, entry, tc.mode, 0, nil, dir)
			if err := rep.Err(); err != nil {
				t.Fatalf("check failed: %v", err)
			}
			got, err := os.ReadFile(marker)
			if err != nil {
				t.Fatal(err)
			}
			if fields := strings.Fields(string(got)); strings.Join(fields, "\x00") != strings.Join(tc.want, "\x00") {
				t.Errorf("arguments = %q, want %q", fields, tc.want)
			}
		})
	}
}

func TestTypeCheckSaysSoWhenTheRuntimeCannotDoIt(t *testing.T) {
	// Node and Bun strip types rather than checking them. Saying so is
	// better than pretending the check ran.
	r := preflight.TypeCheck(context.TODO(), "/usr/bin/node", "/x.ts", preflight.TypeCheckOn, 0)
	if len(r.Problems) != 1 {
		t.Fatalf("expected one note, got %v", r.Problems)
	}
	if r.Err() != nil {
		t.Error("not being able to check is not a reason to refuse to run")
	}
	if !strings.Contains(r.Problems[0].What, "only available under Deno") {
		t.Errorf("got %q", r.Problems[0].What)
	}
}

func TestTypeCheckErrorPolicies(t *testing.T) {
	dir := t.TempDir()
	entry := filepath.Join(dir, "main.ts")
	if err := os.WriteFile(entry, []byte("export {}\n"), 0o644); err != nil {
		t.Fatal(err)
	}
	deno := filepath.Join(dir, "deno")
	if err := os.WriteFile(deno, []byte("#!/bin/sh\necho type error >&2\nexit 1\n"), 0o755); err != nil {
		t.Fatal(err)
	}
	for _, tc := range []struct {
		policy   string
		fatal    bool
		problems int
	}{
		{policy: "fail", fatal: true, problems: 1},
		{policy: "warn", fatal: false, problems: 1},
		{policy: "run", problems: 0},
	} {
		t.Run(tc.policy, func(t *testing.T) {
			rep := preflight.TypeCheckWithPolicy(context.Background(), deno, entry,
				preflight.TypeCheckTypes, 0, nil, dir, tc.policy, "warn")
			if len(rep.Problems) != tc.problems {
				t.Fatalf("problems = %v, want %d", rep.Problems, tc.problems)
			}
			if (rep.Err() != nil) != tc.fatal {
				t.Errorf("fatal error = %v, want fatal=%v", rep.Err(), tc.fatal)
			}
		})
	}
}

func TestUnsupportedRuntimePolicies(t *testing.T) {
	dir := t.TempDir()
	entry := filepath.Join(dir, "main.ts")
	if err := os.WriteFile(entry, []byte("export {}\n"), 0o644); err != nil {
		t.Fatal(err)
	}
	deno := filepath.Join(dir, "deno")
	marker := filepath.Join(dir, "args")
	if err := os.WriteFile(deno, []byte("#!/bin/sh\nprintf '%s\\n' \"$@\" > '"+marker+"'\n"), 0o755); err != nil {
		t.Fatal(err)
	}
	t.Setenv("PATH", dir)
	for _, tc := range []struct {
		policy   string
		problems int
		checked  bool
	}{
		{policy: "skip", problems: 0},
		{policy: "warn", problems: 1},
		{policy: "switch", problems: 0, checked: true},
	} {
		t.Run(tc.policy, func(t *testing.T) {
			_ = os.Remove(marker)
			rep := preflight.TypeCheckWithPolicy(context.Background(), "/usr/bin/node", entry,
				preflight.TypeCheckTypes, 0, nil, dir, "fail", tc.policy)
			if len(rep.Problems) != tc.problems || rep.Err() != nil {
				t.Fatalf("report = %+v, err = %v", rep, rep.Err())
			}
			_, err := os.Stat(marker)
			if (err == nil) != tc.checked {
				t.Errorf("Deno check marker exists = %v, want %v", err == nil, tc.checked)
			}
		})
	}
}

func TestMergeCombinesReports(t *testing.T) {
	a := preflight.CheckEnvPairs([]string{"BROKEN"}, "--env")
	b := preflight.CheckPaths([]preflight.PathCheck{
		{Path: "/no", Where: "x", MustExist: true},
	})
	if got := len(preflight.Merge(a, b, nil).Problems); got != 2 {
		t.Errorf("got %d", got)
	}
}

// Codex round (PR #2589): the type-check subprocess runs where the
// script will execute, so a relative --config in the runtime arguments
// resolves against the same directory at check time as at execution.
func TestTypeCheckRunsInExecutionWorkingDirectory(t *testing.T) {
	dir := t.TempDir()
	execDir := t.TempDir()
	entry := filepath.Join(dir, "main.ts")
	if err := os.WriteFile(entry, []byte("export {};\n"), 0o644); err != nil {
		t.Fatal(err)
	}
	marker := filepath.Join(dir, "cwd")
	stub := filepath.Join(dir, "deno-stub")
	script := "#!/bin/sh\npwd -P > '" + marker + "'\n"
	if err := os.WriteFile(stub, []byte(script), 0o755); err != nil {
		t.Fatal(err)
	}
	rep := preflight.TypeCheckResolved(context.Background(), stub, entry,
		preflight.TypeCheckOn, 10*time.Second,
		[]string{"--config", "./deno.json"}, execDir)
	if err := rep.Err(); err != nil {
		t.Fatalf("unexpected report: %v", err)
	}
	got, err := os.ReadFile(marker)
	if err != nil {
		t.Fatal(err)
	}
	want, err := filepath.EvalSymlinks(execDir)
	if err != nil {
		t.Fatal(err)
	}
	if strings.TrimSpace(string(got)) != want {
		t.Fatalf("type check ran in %q, want the execution directory %q",
			strings.TrimSpace(string(got)), want)
	}
}
