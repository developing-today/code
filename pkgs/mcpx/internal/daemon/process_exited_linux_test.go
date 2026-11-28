package daemon

import (
	"fmt"
	"os"
	"strings"
)

// processExited reports whether a process is gone or a zombie; a signal-0
// probe alone counts a zombie as alive.
//
// In a container whose PID 1 does not reap orphans, a killed orphan stays
// a zombie and syscall.Kill(pid, 0) keeps succeeding on it, so a test
// that polls signal 0 to confirm a kill never sees the process die
// (#13713). The state field of /proc/<pid>/stat tells a zombie from a
// live process. Same helper as internal/e2e's exited, in this package
// because the daemon tests are package daemon, not e2e_test.
func processExited(pid int) bool {
	b, err := os.ReadFile(fmt.Sprintf("/proc/%d/stat", pid))
	if err != nil {
		return true
	}
	i := strings.LastIndexByte(string(b), ')')
	return i+2 < len(b) && b[i+2] == 'Z'
}
