package e2e_test

// Dual-runner harness (#1980, approach C, stage 2).
//
// Every e2e test drives mcpx through a runner: the binary runner, which
// execs the built mcpx binary as a child process, or the in-process
// runner, which calls cli.Run in this process. The binary suite is the
// superset — it runs every test, because signals, real process exit
// codes, and argv exactly as the shell delivers it only exist across a
// process boundary. A test that does not need a process may opt into
// the in-process runner with newEnvInproc; a test that needs one says
// so by using newEnv (the default), which is its needsProcess
// declaration. Tests that exec the binary directly (serve sessions,
// upgrade builds, process takeover) bypass the runner entirely and are
// process tests by construction.
//
// The daemon is a real process in BOTH suites: it is never run as a
// goroutine in-process (rejected on #1980 — a goroutine daemon leaks
// listeners and goroutines between tests). The in-process runner pins
// autostart.bin to the built binary (MCPX_AUTOSTART_BIN), because the
// default, os.Executable(), is the test binary in-process, not mcpx.

import (
	"bytes"
	"context"
	"errors"
	"fmt"
	"io"
	"os"
	"os/exec"
	"strings"
	"sync"
	"testing"
	"time"

	"github.com/developing-today/code/pkgs/mcpx/internal/cli"
)

// runner is one way of invoking mcpx for an env: a child process, or
// cli.Run in-process. run returns combined output and an error whose
// message names the exit status, mirroring exec's contract so callers
// (env.try, env.run, env.runStdin) do not care which runner served
// them. A non-nil stdin is fed to the invocation's standard input.
type runner interface {
	name() string
	needsProcess() bool
	run(ctx context.Context, e *env, stdin io.Reader, args ...string) (string, error)
}

// binaryRunner execs a real mcpx binary (the env's built binary, or
// bin when a test drives a different build, e.g. an upgrade target).
type binaryRunner struct {
	bin string
}

func (binaryRunner) name() string       { return "binary" }
func (binaryRunner) needsProcess() bool { return true }

func (r binaryRunner) run(ctx context.Context, e *env, stdin io.Reader, args ...string) (string, error) {
	bin := r.bin
	if bin == "" {
		bin = e.mcpx
	}
	ctx, cancel := context.WithTimeout(ctx, 90*time.Second)
	defer cancel()
	cmd := exec.CommandContext(ctx, bin, args...)
	cmd.Dir = e.dir
	cmd.Env = e.envVars
	cmd.Stdin = stdin
	out, err := cmd.CombinedOutput()
	return string(out), harnessTimeout(ctx, err)
}

// inprocRunner invokes cli.Run in this process. Until stage 2's
// environment threading lands in cli internals (they still read
// os.Getenv), the env's variables are applied to the process for the
// duration of the call and restored after — which is why in-process
// tests must not use t.Parallel (no e2e test does). The working
// directory is likewise scoped to the env's directory, matching the
// binary runner's cmd.Dir.
type inprocRunner struct{}

func (inprocRunner) name() string       { return "inproc" }
func (inprocRunner) needsProcess() bool { return false }

func (inprocRunner) run(ctx context.Context, e *env, stdin io.Reader, args ...string) (string, error) {
	restore := applyProcessEnv(e)
	defer restore()
	wd, err := os.Getwd()
	if err != nil {
		return "", err
	}
	if err := os.Chdir(e.dir); err != nil {
		return "", err
	}
	defer func() { _ = os.Chdir(wd) }()

	// Two buffers, never one: Run's Stdio writes and the process-stream
	// drain in captureProcessStdio happen concurrently, and a
	// bytes.Buffer is not safe for concurrent writers — sharing one
	// lost output (an empty --version) whenever a command also wrote
	// to the os streams during the same invocation.
	var direct, piped bytes.Buffer
	code, err := captureProcessStdio(stdin, &piped, func() int {
		return cli.Run(ctx, args, cli.Env(e.envVars), cli.Stdio{In: stdin, Out: &direct, Err: &direct})
	})
	out := direct.String() + piped.String()
	if err != nil {
		return out, err
	}
	if code != 0 {
		return out, &exitError{code: code}
	}
	return out, nil
}

// exitError is the in-process analogue of a non-zero process exit: the
// invocation ran to completion and reported this exit code.
type exitError struct{ code int }

func (e *exitError) Error() string { return fmt.Sprintf("exit status %d", e.code) }
func (e *exitError) ExitCode() int { return e.code }

// applyProcessEnv sets the env's variables on this process (plus the
// autostart pin, below), unsets MCPX_* variables the env does not
// carry so a caller's own mcpx settings cannot leak into an
// in-process invocation, and returns a function restoring the prior
// process environment exactly.
func applyProcessEnv(e *env) func() {
	want := make(map[string]string, len(e.envVars)+1)
	for _, kv := range e.envVars {
		k, v, _ := strings.Cut(kv, "=")
		want[k] = v
	}
	// The daemon stays a real process, started from the built binary —
	// never os.Executable(), which in-process is the test binary.
	want["MCPX_AUTOSTART_BIN"] = e.mcpx

	type prior struct {
		v  string
		ok bool
	}
	saved := map[string]prior{}
	save := func(k string) {
		if _, done := saved[k]; !done {
			v, ok := os.LookupEnv(k)
			saved[k] = prior{v, ok}
		}
	}
	for k := range want {
		save(k)
	}
	for _, kv := range os.Environ() {
		k, _, _ := strings.Cut(kv, "=")
		if strings.HasPrefix(k, "MCPX_") {
			if _, kept := want[k]; !kept {
				save(k)
				_ = os.Unsetenv(k)
			}
		}
	}
	for k, v := range want {
		_ = os.Setenv(k, v)
	}
	return func() {
		for k, p := range saved {
			if p.ok {
				_ = os.Setenv(k, p.v)
			} else {
				_ = os.Unsetenv(k)
			}
		}
	}
}

// captureProcessStdio runs fn with the process's stdout and stderr
// redirected into w and its stdin redirected from in (when non-nil).
// Commands still write to the os streams directly (stage 1's Stdio
// covers dispatch-level output only), so capturing an in-process
// invocation means capturing the process streams, the same technique
// cli's own Run tests use. Output is drained concurrently: a pipe
// buffer is far smaller than a chatty command's output.
func captureProcessStdio(in io.Reader, w io.Writer, fn func() int) (int, error) {
	oldOut, oldErr, oldIn := os.Stdout, os.Stderr, os.Stdin
	rOut, wOut, err := os.Pipe()
	if err != nil {
		return 0, err
	}
	rIn, wIn, err := os.Pipe()
	if err != nil {
		_ = rOut.Close()
		_ = wOut.Close()
		return 0, err
	}
	os.Stdout, os.Stderr = wOut, wOut
	os.Stdin = rIn
	var wg sync.WaitGroup
	wg.Add(1)
	go func() {
		defer wg.Done()
		_, _ = io.Copy(w, rOut)
	}()
	wg.Add(1)
	go func() {
		defer wg.Done()
		if in != nil {
			_, _ = io.Copy(wIn, in)
		}
		_ = wIn.Close()
	}()
	code := fn()
	os.Stdout, os.Stderr, os.Stdin = oldOut, oldErr, oldIn
	_ = wOut.Close()
	_ = rIn.Close()
	wg.Wait()
	_ = rOut.Close()
	return code, nil
}

// newEnvInproc is newEnv with the in-process runner: the test declares
// it does not need a process. The daemon the test reaches is still a
// real process, autostarted from the built binary.
func newEnvInproc(t *testing.T, cfgBody string) *env {
	t.Helper()
	e := newEnv(t, cfgBody)
	e.runner = inprocRunner{}
	return e
}

func TestRunnerDefaultIsBinarySuperset(t *testing.T) {
	e := newEnv(t, oneServer)
	if e.runner == nil || e.runner.name() != "binary" || !e.runner.needsProcess() {
		t.Fatalf("default runner = %v, want the binary runner (superset, needsProcess)", e.runner)
	}
}

func TestInprocRunnerAnswersDispatchLevelCommands(t *testing.T) {
	e := newEnvInproc(t, oneServer)
	out, err := e.try("--version")
	if err != nil {
		t.Fatalf("in-process --version failed: %v\n%s", err, out)
	}
	if !strings.Contains(out, cli.Version) {
		t.Fatalf("in-process --version = %q, want it to name cli.Version %q", out, cli.Version)
	}
	if _, err := e.try("frobnicate-xyz"); err == nil {
		t.Fatal("in-process unknown command succeeded, want an exit error")
	} else {
		var ee *exitError
		if !errors.As(err, &ee) || ee.ExitCode() == 0 {
			t.Fatalf("in-process unknown command error = %v, want a non-zero exit status", err)
		}
	}
}

// TestInprocRunnerLsStartsRealDaemon is the stage-2 acceptance test:
// an in-process `ls` autostarts the daemon — a real process, from the
// built binary — and lists the configured namespace through it.
func TestInprocRunnerLsStartsRealDaemon(t *testing.T) {
	e := newEnvInproc(t, oneServer)
	out := e.run("ls")
	if !strings.Contains(out, "demo") {
		t.Fatalf("namespace missing from in-process ls:\n%s", out)
	}
	if !strings.Contains(out, "a fake server") {
		t.Fatalf("description missing from in-process ls:\n%s", out)
	}
}
