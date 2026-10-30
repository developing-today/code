package cli

import (
	"bytes"
	"context"
	"encoding/json"
	"errors"
	"io"
	"os"
	osexec "os/exec"
	"path/filepath"
	"strings"
	"testing"

	"github.com/dezren39/mcpx/internal/config"
	"github.com/dezren39/mcpx/internal/execsvc"
	"github.com/dezren39/mcpx/internal/testsupport"
)

// Stage 1 of #1980: Run is the dispatch main() used to own, reachable
// in-process. These tests pin the exit-code contract the binary suite
// asserts through a process boundary; they deliberately avoid commands
// that need a daemon or a terminal.

func TestRunVersion(t *testing.T) {
	var out, errOut bytes.Buffer
	code := Run(context.Background(), []string{"--version"}, Env{}, Stdio{Out: &out, Err: &errOut})
	if code != 0 {
		t.Fatalf("exit code = %d, want 0 (stderr: %s)", code, errOut.String())
	}
	if got, want := out.String(), "mcpx "+Version+"\n"; got != want {
		t.Fatalf("stdout = %q, want %q", got, want)
	}
}

func TestRunGlobalFlagThenVersion(t *testing.T) {
	var out, errOut bytes.Buffer
	code := Run(context.Background(), []string{"--json", "--version"}, Env{}, Stdio{Out: &out, Err: &errOut})
	if code != 0 {
		t.Fatalf("exit code = %d, want 0 (stderr: %s)", code, errOut.String())
	}
	if !strings.HasPrefix(out.String(), "mcpx ") {
		t.Fatalf("stdout = %q, want the version line", out.String())
	}
}

func TestRunUnknownGlobalFlag(t *testing.T) {
	var out, errOut bytes.Buffer
	code := Run(context.Background(), []string{"--no-such-flag"}, Env{}, Stdio{Out: &out, Err: &errOut})
	if code != 2 {
		t.Fatalf("exit code = %d, want 2", code)
	}
	if !strings.Contains(errOut.String(), `unknown global flag "--no-such-flag"`) {
		t.Fatalf("stderr = %q, want the unknown-flag message", errOut.String())
	}
}

func TestRunUnknownCommand(t *testing.T) {
	var errOut bytes.Buffer
	var code int
	// CmdHelp still writes to os.Stdout in stage 1; capture it so the
	// help text does not land in the test output.
	_, err := captureStdout(t, func() error {
		code = Run(context.Background(), []string{"frobnicate-xyz"}, Env{}, Stdio{Err: &errOut})
		return nil
	})
	if err != nil {
		t.Fatal(err)
	}
	if code != 2 {
		t.Fatalf("exit code = %d, want 2", code)
	}
	if !strings.Contains(errOut.String(), `unknown command "frobnicate-xyz"`) {
		t.Fatalf("stderr = %q, want the unknown-command message", errOut.String())
	}
}

func TestRunHelpCommandDispatches(t *testing.T) {
	var code int
	out, err := captureStdout(t, func() error {
		code = Run(context.Background(), []string{"help"}, Env{}, Stdio{})
		return nil
	})
	if err != nil {
		t.Fatal(err)
	}
	if code != 0 {
		t.Fatalf("exit code = %d, want 0", code)
	}
	if out == "" {
		t.Fatal("help command printed nothing")
	}
}

func TestEnvLookup(t *testing.T) {
	env := Env{"A=1", "B=2=3", "A=9"}
	if got := env.Get("A"); got != "1" {
		t.Fatalf("Get(A) = %q, want the first entry %q", got, "1")
	}
	if got, ok := env.Lookup("B"); !ok || got != "2=3" {
		t.Fatalf("Lookup(B) = %q, %v; want %q, true", got, ok, "2=3")
	}
	if _, ok := env.Lookup("MISSING"); ok {
		t.Fatal("Lookup(MISSING) reported present")
	}
}

func TestAppEnvironFallsBackToOS(t *testing.T) {
	t.Setenv("MCPX_RUN_TEST_SENTINEL", "present")
	if got := (&App{}).Environ().Get("MCPX_RUN_TEST_SENTINEL"); got != "present" {
		t.Fatalf("Environ().Get(sentinel) = %q, want the os environment", got)
	}
	given := (&App{environ: Env{"MCPX_RUN_TEST_SENTINEL=from-run"}}).Environ()
	if got := given.Get("MCPX_RUN_TEST_SENTINEL"); got != "from-run" {
		t.Fatalf("Environ().Get(sentinel) = %q, want the Run-supplied env", got)
	}
}

// ---- Regression tests for the Codex findings on PR #2714 ----
//
// The first Codex round found two bugs the stage-1 tests could not see:
// script handlers called os.Exit on a nonzero script status (terminating
// an in-process caller of Run), and the --paths-config seed leaked
// between sequential Run calls. The tests below execute both bugs' exact
// shapes: if either os.Exit comes back, this test binary dies instead of
// failing cleanly, and if the seed scoping comes out, the chain
// resolution after a Run still reports the previous invocation's seed.

// runTestConfig gives a Run invocation a hermetic home: a temp HOME whose
// only config declares no servers, so the in-process daemon answers at
// once and nothing outside the temp tree is read or written.
func runTestConfig(t *testing.T) string {
	t.Helper()
	root := testsupport.TempDir(t)
	t.Setenv("HOME", root)
	t.Setenv("XDG_CONFIG_HOME", filepath.Join(root, "xdg"))
	t.Setenv("XDG_STATE_HOME", filepath.Join(root, "state"))
	t.Setenv("XDG_CACHE_HOME", filepath.Join(root, "cache"))
	t.Setenv("MCPX_CONFIG", "")
	t.Setenv("MCPX_PATHS_CONFIG", "")
	t.Setenv("MCPX_STATE_DIR", "")
	// No daemon binary exists beside a test binary to spawn, and none
	// should be: the in-process daemon is the one a Run caller gets.
	t.Setenv("MCPX_DAEMON_AUTOSTART", "false")
	t.Setenv("MCPX_DAEMON_INLINE", "true")
	cfg := filepath.Join(root, "mcpx.json")
	if err := os.WriteFile(cfg, []byte(`{"mcpServers":{}}`), 0o600); err != nil {
		t.Fatal(err)
	}
	return cfg
}

// scriptRuntime picks the first JS runtime this machine has, in the
// order the e2e runtime tests exercise them. A machine with none skips:
// the binary suite on a runner that has one remains the coverage.
func scriptRuntime(t *testing.T) string {
	t.Helper()
	for _, rt := range []string{"node", "deno", "bun"} {
		if _, err := osexec.LookPath(rt); err == nil {
			return rt
		}
	}
	t.Skip("no JS runtime (node, deno, or bun) on PATH")
	return ""
}

// exitThree is a snippet that ends its run with status 3 on every
// runtime: the launcher gives bun and node the Deno global deno has
// natively (the same shape TestDenoExitWorksOnEveryRuntime runs through
// the binary).
const exitThree = `console.log("before-exit"); (globalThis as any).Deno.exit(3); console.log("after-exit");`

func TestRunScriptFailureReturnsStatusText(t *testing.T) {
	cfg := runTestConfig(t)
	rt := scriptRuntime(t)
	var code int
	out, err := captureStdout(t, func() error {
		code = Run(context.Background(),
			[]string{"--config", cfg, "exec", "--runtime", rt, exitThree},
			Env{}, Stdio{Err: io.Discard})
		return nil
	})
	if err != nil {
		t.Fatal(err)
	}
	if code != 3 {
		t.Fatalf("Run(exec, script exits 3) = %d, want 3 (stdout: %s)", code, out)
	}
	if !strings.Contains(out, "before-exit") || strings.Contains(out, "after-exit") {
		t.Fatalf("the script did not stop where it exited:\n%s", out)
	}
}

func TestRunScriptFailureReturnsStatusJSON(t *testing.T) {
	cfg := runTestConfig(t)
	rt := scriptRuntime(t)
	var code int
	out, err := captureStdout(t, func() error {
		code = Run(context.Background(),
			[]string{"--json", "--config", cfg, "exec", "--runtime", rt, exitThree},
			Env{}, Stdio{Err: io.Discard})
		return nil
	})
	if err != nil {
		t.Fatal(err)
	}
	if code != 3 {
		t.Fatalf("Run(--json exec, script exits 3) = %d, want 3 (stdout: %s)", code, out)
	}
	var doc struct {
		OK       bool `json:"ok"`
		ExitCode int  `json:"exitCode"`
	}
	if jerr := json.Unmarshal([]byte(jsonLineOf(t, out)), &doc); jerr != nil {
		t.Fatalf("the JSON envelope should survive a failing script: %v\n%s", jerr, out)
	}
	if doc.OK || doc.ExitCode != 3 {
		t.Fatalf("envelope = %+v, want ok=false exitCode=3", doc)
	}
}

// jsonLineOf returns the first line of out that parses as a JSON object,
// so a test can find the envelope among anything else a run printed.
func jsonLineOf(t *testing.T, out string) string {
	t.Helper()
	for _, line := range strings.Split(out, "\n") {
		line = strings.TrimSpace(line)
		if strings.HasPrefix(line, "{") {
			var v map[string]any
			if json.Unmarshal([]byte(line), &v) == nil {
				return line
			}
		}
	}
	// The envelope is pretty-printed by App.out, so it spans lines; fall
	// back to the whole capture and let the caller's unmarshal judge it.
	return out
}

func TestRenderExecResultReturnsStatusNotExit(t *testing.T) {
	// The remote path (execRemote -> renderExecResult): a daemon-side
	// script failure arrives as a Result with a nonzero ExitCode and must
	// come back as an ExitCoder error, in both render shapes.
	for _, tc := range []struct {
		name string
		app  *App
	}{
		{"text", &App{}},
		{"json", &App{JSON: true}},
	} {
		t.Run(tc.name, func(t *testing.T) {
			res := &execsvc.Result{ExitCode: 7, Stdout: "remote-out\n"}
			var err error
			out, cerr := captureStdout(t, func() error {
				err = tc.app.renderExecResult(res, execOptions{})
				return nil
			})
			if cerr != nil {
				t.Fatal(cerr)
			}
			var ec ExitCoder
			if !errors.As(err, &ec) || ec.ExitCode() != 7 {
				t.Fatalf("renderExecResult(ExitCode 7) err = %v, want an ExitCoder with code 7", err)
			}
			if !strings.Contains(out, "remote-out") {
				t.Fatalf("the result's stdout was not rendered:\n%s", out)
			}
		})
	}
}

func TestRunPathsConfigSeedDoesNotLeak(t *testing.T) {
	root := filepath.Dir(runTestConfig(t))
	seed := filepath.Join(root, "seed.json")
	if err := os.WriteFile(seed, []byte(`{"paths":{"config":[]}}`), 0o600); err != nil {
		t.Fatal(err)
	}
	seeded := func() []string {
		_, info, err := config.ResolveChainFrom(root)
		if err != nil {
			t.Fatal(err)
		}
		return info.Seeded
	}
	// Sanity: the probe can see a seed when one is set, so an empty
	// result below means the seed is gone, not that the probe is blind.
	config.SetPathsConfigSeed([]string{seed})
	if got := seeded(); len(got) != 1 || got[0] != seed {
		t.Fatalf("with a seed set, Seeded = %v, want [%s]", got, seed)
	}
	// A Run that never mentions --paths-config must clear a seed it
	// inherited (entry scoping), and leave none behind (exit scoping).
	var out, errOut bytes.Buffer
	if code := Run(context.Background(), []string{"--version"}, Env{}, Stdio{Out: &out, Err: &errOut}); code != 0 {
		t.Fatalf("Run(--version) = %d, want 0 (stderr: %s)", code, errOut.String())
	}
	if got := seeded(); len(got) != 0 {
		t.Fatalf("after a Run without --paths-config, Seeded = %v, want none", got)
	}
	// A Run that takes the flag uses it for that invocation only: the
	// next flagless invocation must not inherit it.
	out.Reset()
	errOut.Reset()
	if code := Run(context.Background(), []string{"--paths-config", seed, "--version"}, Env{}, Stdio{Out: &out, Err: &errOut}); code != 0 {
		t.Fatalf("Run(--paths-config seed --version) = %d, want 0 (stderr: %s)", code, errOut.String())
	}
	if got := seeded(); len(got) != 0 {
		t.Fatalf("after a Run with --paths-config, Seeded = %v, want none (the seed leaked into the next invocation)", got)
	}
	t.Cleanup(func() { config.SetPathsConfigSeed(nil) })
}
