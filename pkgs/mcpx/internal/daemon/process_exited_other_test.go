//go:build !linux && !darwin

package daemon

import "syscall"

// processExited falls back to signal 0 where there is neither /proc nor
// the darwin sysctl; a zombie reads as alive here, so callers wait for it
// to be reaped. Same helper as internal/e2e's exited, in this package
// because the daemon tests are package daemon, not e2e_test.
func processExited(pid int) bool {
	return syscall.Kill(pid, 0) != nil
}
