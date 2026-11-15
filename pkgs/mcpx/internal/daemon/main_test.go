package daemon_test

import (
	"os"
	"testing"

	"github.com/developing-today/code/pkgs/mcpx/internal/testsupport"
)

// TestMain applies the suite-wide build-temp-dir sweep (#1971, #10554):
// this suite uses the testsupport fixture builders, whose temp dirs are
// removed when the run ends; a process still executing from one of them
// fails the run.
func TestMain(m *testing.M) {
	os.Exit(testsupport.MainWithSweep(m))
}
