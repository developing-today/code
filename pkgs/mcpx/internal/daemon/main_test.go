package daemon_test

import (
	"fmt"
	"os"
	"testing"

	"github.com/developing-today/code/pkgs/mcpx/internal/testsupport"
)

// TestMain applies the suite-wide build-temp-dir sweep (#1971, #10554):
// this suite uses the testsupport fixture builders, whose temp dirs are
// removed when the run ends; a process still executing from one of them
// fails the run.
func TestMain(m *testing.M) {
	testsupport.SuperviseReExec()
	stateDir, err := os.MkdirTemp("", "mcpx-daemon-state-")
	if err != nil {
		fmt.Fprintf(os.Stderr, "daemon tests: making an isolated state dir: %v\n", err)
		os.Exit(1)
	}
	if err := os.Setenv("XDG_STATE_HOME", stateDir); err != nil {
		fmt.Fprintf(os.Stderr, "daemon tests: isolating XDG_STATE_HOME: %v\n", err)
		_ = os.RemoveAll(stateDir)
		os.Exit(1)
	}
	code := testsupport.MainWithSweep(m)
	if err := os.RemoveAll(stateDir); err != nil {
		fmt.Fprintf(os.Stderr, "daemon tests: removing isolated state dir: %v\n", err)
		code = 1
	}
	os.Exit(code)
}
