package cli

import (
	"fmt"
	"os"
	"testing"

	"github.com/developing-today/code/pkgs/mcpx/internal/stateguard"
)

// TestMain pins the suite's state directories into a per-run temp dir and
// fails the run if the caller's real daemon records changed (#1992): this
// suite writes records (client_socket_test), and `go test ./...` runs its
// binary beside e2e's, where a record mirrored into the caller's real
// default dir (#278) would land inside e2e's before/after window.
func TestMain(m *testing.M) {
	guard := stateguard.GuardCallerDaemons()
	cleanup, err := stateguard.PinSuiteStateDirs()
	if err != nil {
		fmt.Fprintln(os.Stderr, "cli: pinning the suite's state dirs:", err)
		os.Exit(1)
	}
	code := m.Run()
	cleanup()
	if err := guard(); err != nil {
		fmt.Fprintln(os.Stderr, "cli:", err)
		code = 1
	}
	os.Exit(code)
}
