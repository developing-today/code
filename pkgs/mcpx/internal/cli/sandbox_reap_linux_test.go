//go:build linux

package cli

import (
	"os"
	"testing"
	"time"

	"golang.org/x/sys/unix"
)

// TestMain makes this test binary a child subreaper so daemon processes it
// auto-started (client.go Setsid+Release) reparent to it instead of the
// build sandbox's PID 1, which never waits and lets them pile up as zombies
// until the 30m timeout burns (#2686).
func TestMain(m *testing.M) {
	if err := unix.Prctl(unix.PR_SET_CHILD_SUBREAPER, 1, 0, 0, 0); err == nil {
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
	os.Exit(m.Run())
}
