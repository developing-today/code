//go:build !linux && !darwin

package e2e_test

// exited falls back to signal 0 where there is neither /proc nor the darwin
// sysctl; a zombie reads as alive here, so awaitExit waits for it to be reaped.
func exited(pid int) bool {
	return !processExists(pid)
}
