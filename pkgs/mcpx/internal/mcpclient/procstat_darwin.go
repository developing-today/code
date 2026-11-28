package mcpclient

import (
	"fmt"

	"golang.org/x/sys/unix"
)

// procStat reads a process's start time and state from the kernel via
// sysctl kern.proc.pid — darwin has no /proc (#7063). The start time is
// microseconds since the epoch (p_starttime). Its units differ from the
// Linux clock-tick value, which is fine: a start time is only ever
// compared for equality against another reading taken on the same
// machine and platform, to tell a recycled pid from the original
// process.
func procStat(pid int) (uint64, byte, error) {
	kp, err := unix.SysctlKinfoProc("kern.proc.pid", pid)
	if err != nil {
		return 0, 0, fmt.Errorf("sysctl kern.proc.pid %d: %w", pid, err)
	}
	start := uint64(kp.Proc.P_starttime.Sec)*1_000_000 + uint64(kp.Proc.P_starttime.Usec)
	state := byte('R')
	// SZOMB from <sys/proc.h>: the process has exited and is awaiting
	// collection by its parent. x/sys/unix does not export the
	// constant for darwin, so it is spelled out here.
	const szomb = 5
	if kp.Proc.P_stat == szomb {
		state = 'Z'
	}
	return start, state, nil
}
