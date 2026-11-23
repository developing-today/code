package daemon_test

import (
	"fmt"
	"os"
	"testing"

	"github.com/developing-today/code/pkgs/mcpx/internal/stateguard"
	"github.com/developing-today/code/pkgs/mcpx/internal/testsupport"
)

// TestMain applies the suite-wide build-temp-dir sweep (#1971, #10554) and
// the #1992 isolation the e2e suite demands of its neighbours: this suite
// writes real daemon records, so its state directories are pinned into a
// per-run temp dir, and the run fails if the caller's real records changed
// anyway. `go test ./...` runs this binary beside e2e, and a record mirrored
// into the caller's real default dir (#278) lands inside e2e's before/after
// window and fails that suite too.
func TestMain(m *testing.M) {
	testsupport.SuperviseReExec()
	guard := stateguard.GuardCallerDaemons()
	cleanup, err := stateguard.PinSuiteStateDirs()
	if err != nil {
		fmt.Fprintln(os.Stderr, "daemon: pinning the suite's state dirs:", err)
		os.Exit(1)
	}
	code := testsupport.MainWithSweep(m)
	cleanup()
	if err := guard(); err != nil {
		fmt.Fprintln(os.Stderr, "daemon:", err)
		code = 1
	}
	os.Exit(code)
}
