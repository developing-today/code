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
//
// It also isolates the default daemon state directory (#14332): fixtures
// that publish daemon records from custom state dirs mirror them into
// DefaultStateDir, which resolves from the caller's XDG_STATE_HOME/HOME.
// Without isolation those mirrors land in the real caller directory, where
// the e2e suite's before/after daemon inventory guard observes them as
// drift even though every test passes.
func TestMain(m *testing.M) {
	testsupport.SuperviseReExec()
	dir, err := os.MkdirTemp("", "daemon-suite-state")
	if err != nil {
		fmt.Fprintln(os.Stderr, err)
		os.Exit(1)
	}
	if err := os.Setenv("XDG_STATE_HOME", dir); err != nil {
		fmt.Fprintln(os.Stderr, err)
		_ = os.RemoveAll(dir)
		os.Exit(1)
	}
	if err := os.Unsetenv("MCPX_STATE_DIR"); err != nil {
		fmt.Fprintln(os.Stderr, err)
		_ = os.RemoveAll(dir)
		os.Exit(1)
	}
	code := testsupport.MainWithSweep(m)
	_ = os.RemoveAll(dir)
	os.Exit(code)
}
