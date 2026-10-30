package cli

import (
	"bytes"
	"context"
	"strings"
	"testing"
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
