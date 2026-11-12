//go:build !linux && !darwin

package mcpclient

import (
	"errors"
	"runtime"
)

// procStat has no implementation on this platform: process start-time
// identity is available on Linux (/proc) and darwin (sysctl kern.proc)
// only, so handoff detach/adopt fails explicitly here instead of
// reading a /proc that does not exist (#7063).
func procStat(pid int) (uint64, byte, error) {
	return 0, 0, errors.New("process start time unavailable on " + runtime.GOOS)
}
