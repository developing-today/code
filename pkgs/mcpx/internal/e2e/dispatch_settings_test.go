package e2e_test

// Effect probes for the dispatch settings (#2167): the two knobs that
// decide whether a script runs under an interpreter and where it runs
// must reach the CLI from the environment like every other setting, or
// the settings registry entry is decoration.

import (
	"os"
	"path/filepath"
	"strings"
	"testing"
)

// MCPX_SCRIPT_INTERPRETER makes `mcpx exec` run the source under that
// command line instead of a JavaScript runtime. The source is not a
// TypeScript program, so without the setting the run cannot produce
// this output.
func TestScriptInterpreterIsReachableFromTheEnvironment(t *testing.T) {
	e := newEnv(t, oneServer)
	saved := e.envVars
	e.envVars = append(append([]string{}, saved...), "MCPX_SCRIPT_INTERPRETER=sh")
	defer func() { e.envVars = saved }()

	out := e.run("exec", "echo from-the-configured-interpreter")
	if !strings.Contains(out, "from-the-configured-interpreter") {
		t.Fatalf("MCPX_SCRIPT_INTERPRETER should run the source under sh:\n%s", out)
	}
}

// MCPX_SCRIPT_CWD=script moves a dispatched run to the script's own
// directory; the default (caller) leaves it where mcpx was invoked, so
// the two are distinguishable from the script's own pwd.
func TestScriptCwdIsReachableFromTheEnvironment(t *testing.T) {
	e := newEnv(t, oneServer)
	sub := filepath.Join(e.dir, "scripts")
	if err := os.MkdirAll(sub, 0o755); err != nil {
		t.Fatal(err)
	}
	script := filepath.Join(sub, "where.sh")
	if err := os.WriteFile(script, []byte("#!/bin/sh\npwd\n"), 0o755); err != nil {
		t.Fatal(err)
	}
	saved := e.envVars
	e.envVars = append(append([]string{}, saved...), "MCPX_SCRIPT_CWD=script")
	defer func() { e.envVars = saved }()

	out := e.run("run", script)
	// The dispatch notice shares the stream and is written to stderr, so
	// its position against the script's own stdout output is not
	// ordered (observed in the flake-binary leg of run 45251429763,
	// where the notice followed the pwd). Find the pwd by value instead
	// of assuming it is the last line: a run that stayed in the caller's
	// directory prints that directory and matches nothing here.
	want, _ := filepath.EvalSymlinks(sub)
	var got string
	for _, line := range strings.Split(strings.TrimSpace(out), "\n") {
		cand := strings.TrimSpace(line)
		if !filepath.IsAbs(cand) {
			continue
		}
		resolved, err := filepath.EvalSymlinks(cand)
		if err != nil || resolved != want {
			continue
		}
		got = resolved
		break
	}
	if got != want {
		t.Fatalf("MCPX_SCRIPT_CWD=script: no output line is the script directory %q\n%s", want, out)
	}
}
