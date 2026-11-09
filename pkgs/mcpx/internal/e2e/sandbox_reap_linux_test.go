//go:build linux

package e2e_test

import (
	"time"

	"golang.org/x/sys/unix"
)

// installSandboxReaper makes this test binary a child subreaper so daemon
// processes it auto-started (cli/client.go Setsid+Release) reparent to it
// instead of the build sandbox's PID 1, which never waits and lets them pile
// up as zombies until the 30m timeout burns (#2686). A background waiter
// reaps whatever gets adopted; the call is a no-op if prctl fails.
func installSandboxReaper() {
	if err := unix.Prctl(unix.PR_SET_CHILD_SUBREAPER, 1, 0, 0, 0); err != nil {
		return
	}
	go func() {
		for {
			var status unix.WaitStatus
			pid, err := unix.Wait4(-1, &status, 0, nil)
			if err != nil {
				if err == unix.ECHILD {
					time.Sleep(100 * time.Millisecond)
					continue
				}
				if err == unix.EINTR {
					continue
				}
				time.Sleep(100 * time.Millisecond)
				continue
			}
			if pid <= 0 {
				time.Sleep(100 * time.Millisecond)
			}
		}
	}()
}
