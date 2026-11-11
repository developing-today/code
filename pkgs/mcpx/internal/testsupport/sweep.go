package testsupport

import (
	"fmt"
	"os"
	"path/filepath"
	"sort"
	"strconv"
	"strings"
	"sync"
	"time"
)

// The build temp dirs this test binary created (mcpx-bin-* for the mcpx
// binary, mcpx-fake-* for the fake MCP server). The builders register
// every dir they make here so the suite-wide sweep (#1971 remainder
// item 2) can remove exactly this binary's dirs at the end of the run —
// and only this binary's: other suites running concurrently on the same
// machine create sibling mcpx-bin-*/mcpx-fake-* dirs in the same temp
// root, and a name-pattern sweep over the whole temp root would remove
// a live neighbour's binaries mid-run.
var (
	buildTempDirsMu sync.Mutex
	buildTempDirs   []string
)

func registerBuildTempDir(dir string) {
	buildTempDirsMu.Lock()
	defer buildTempDirsMu.Unlock()
	for _, existing := range buildTempDirs {
		if existing == dir {
			return
		}
	}
	buildTempDirs = append(buildTempDirs, dir)
}

// BuildTempDirs returns the build temp dirs registered by this test
// binary's builders, in creation order.
func BuildTempDirs() []string {
	buildTempDirsMu.Lock()
	defer buildTempDirsMu.Unlock()
	return append([]string(nil), buildTempDirs...)
}

// LeftoverProcess is a process still executing a binary that lives in
// one of this test binary's build temp dirs after the suite has run: a
// leaked test daemon or fake server, not a process the suite owns any
// more (its tests have all returned).
type LeftoverProcess struct {
	PID int
	Exe string
}

func (p LeftoverProcess) String() string {
	return fmt.Sprintf("pid=%d exe=%s", p.PID, p.Exe)
}

// processesExecutingFrom returns the processes whose executable resolves
// inside one of dirs, excluding this process. Detection is Linux /proc
// based; on a platform without /proc it reports nothing (the temp-dir
// half of the sweep still runs there).
func processesExecutingFrom(dirs []string) []LeftoverProcess {
	if len(dirs) == 0 {
		return nil
	}
	resolved := make([]string, 0, len(dirs))
	for _, dir := range dirs {
		if real, err := filepath.EvalSymlinks(dir); err == nil {
			dir = real
		}
		resolved = append(resolved, dir)
	}
	links, err := filepath.Glob("/proc/[0-9]*/exe")
	if err != nil {
		return nil
	}
	self := os.Getpid()
	var out []LeftoverProcess
	for _, link := range links {
		pidText := filepath.Base(filepath.Dir(link))
		pid, err := strconv.Atoi(pidText)
		if err != nil || pid == self {
			continue
		}
		exe, err := os.Readlink(link)
		if err != nil {
			continue
		}
		// A binary unlinked while running reads back with this suffix;
		// the path still names the dir it was executed from.
		exe = strings.TrimSuffix(exe, " (deleted)")
		for _, dir := range resolved {
			if within(exe, dir) {
				out = append(out, LeftoverProcess{PID: pid, Exe: exe})
				break
			}
		}
	}
	sort.Slice(out, func(i, j int) bool { return out[i].PID < out[j].PID })
	return out
}

// SweepBuildTempDirs is the suite-wide sweep of #1971 remainder item 2:
// it removes this test binary's build temp dirs and reports any process
// still executing from them. Detection runs before removal, and a
// process seen once gets one short grace re-check, so a child caught in
// the act of exiting is not a failure; a process still there after the
// grace window is a leak and is reported for the caller (the e2e
// TestMain) to fail on.
func SweepBuildTempDirs() (removed []string, leftover []LeftoverProcess) {
	dirs := BuildTempDirs()
	leftover = processesExecutingFrom(dirs)
	if len(leftover) > 0 {
		time.Sleep(250 * time.Millisecond)
		leftover = processesExecutingFrom(dirs)
	}
	for _, dir := range dirs {
		if err := os.RemoveAll(dir); err == nil {
			removed = append(removed, dir)
		}
	}
	return removed, leftover
}
