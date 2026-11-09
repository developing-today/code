//go:build !linux

package e2e_test

// installSandboxReaper is a no-op off Linux: darwin builders have a reaping
// init, and prctl is Linux-only.
func installSandboxReaper() {}
