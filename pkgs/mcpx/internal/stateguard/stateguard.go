// Package stateguard is the #1992 backstop the e2e suite runs against
// itself, ported to the suites whose own tests write daemon records.
//
// It is a package of its own, not part of testsupport, because the
// daemon suite's internal tests import testsupport while this package
// imports daemon -- an import cycle in either direction.
package stateguard

import (
	"fmt"
	"os"
	"path/filepath"
	"slices"
	"strconv"
	"strings"

	"github.com/developing-today/code/pkgs/mcpx/internal/daemon"
)

// callerRecordDirs are the daemon-record directories the caller's real
// environment names: the default one a custom state directory mirrors its
// record into (#278), plus the MCPX_STATE_DIR one a plain invocation
// resolves to (#305). A suite that writes a record into either one is
// touching the caller's machine, not its own sandbox.
func callerRecordDirs() []string {
	dirs := []string{daemon.DefaultStateDir()}
	if env := os.Getenv("MCPX_STATE_DIR"); env != "" && !slices.Contains(dirs, env) {
		dirs = append(dirs, env)
	}
	return dirs
}

// callerDaemonSet lists the daemon records under dir the way the e2e
// suite's #1992 guard compares them: "socket pid=..." strings, sorted.
func callerDaemonSet(dir string) []string {
	var out []string
	p := daemon.Paths{State: dir}
	for _, i := range p.ListDaemons() {
		out = append(out, i.Socket+" pid="+strconv.Itoa(i.PID))
	}
	slices.Sort(out)
	return out
}

// GuardCallerDaemons snapshots the caller's real daemon records and
// returns a check that fails when the suite disturbed them: the #1992
// backstop the e2e suite runs against itself, ported to the suites whose
// own tests write records. `go test ./...` runs every package's binary
// side by side, so a record this suite leaks into the caller's real
// directories can also land inside e2e's before/after window and fail
// that suite; the stress runs caught the daemon and cli suites doing
// exactly that -- the mxh* mirrors of the in-process test servers, and
// the /tmp/sock pid=42 record TestInfoFileNamesBothDigests writes. Call
// it before PinSuiteStateDirs: the pin hides the real directories from
// the suite, and this guard is what proves it stayed away.
func GuardCallerDaemons() func() error {
	type dirSet struct {
		dir string
		set []string
	}
	before := make([]dirSet, 0, 2)
	for _, dir := range callerRecordDirs() {
		before = append(before, dirSet{dir, callerDaemonSet(dir)})
	}
	return func() error {
		for _, s := range before {
			if after := callerDaemonSet(s.dir); !slices.Equal(s.set, after) {
				return fmt.Errorf(
					"the suite disturbed the caller's real daemon records (#1992): %s; before=[%s] after=[%s]. "+
						"Every record this suite writes belongs in its own state directory; see PinSuiteStateDirs in internal/stateguard. "+
						"A daemon you started or stopped yourself mid-suite reads the same way.",
					s.dir, strings.Join(s.set, " "), strings.Join(after, " "))
			}
		}
		return nil
	}
}

// PinSuiteStateDirs points XDG_STATE_HOME and MCPX_STATE_DIR at a per-run
// temp directory for the rest of this process, so a test that writes a
// daemon record without its own override lands in the temp directory,
// never the caller's real one. It returns the cleanup that removes the
// temp directory. Call GuardCallerDaemons first: the pin hides the real
// directories from the suite, and the guard is what proves it stayed away.
func PinSuiteStateDirs() (func(), error) {
	dir, err := os.MkdirTemp("", "mcpx-suite-state-")
	if err != nil {
		return nil, err
	}
	cleanup := func() { _ = os.RemoveAll(dir) }
	if err := os.Setenv("XDG_STATE_HOME", dir); err != nil {
		cleanup()
		return nil, err
	}
	if err := os.Setenv("MCPX_STATE_DIR", filepath.Join(dir, "state")); err != nil {
		cleanup()
		return nil, err
	}
	return cleanup, nil
}
