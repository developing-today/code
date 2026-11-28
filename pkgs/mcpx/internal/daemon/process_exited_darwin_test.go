package daemon

import "golang.org/x/sys/unix"

// processExited reports whether a process is gone or a zombie; a signal-0
// probe alone counts a zombie as alive.
//
// Darwin has no /proc. The kernel's own process table is read instead,
// the way mcpclient's procStat does (and the way internal/e2e's exited
// does). Same helper as internal/e2e's exited, in this package because
// the daemon tests are package daemon, not e2e_test.
func processExited(pid int) bool {
	kp, err := unix.SysctlKinfoProc("kern.proc.pid", pid)
	if err != nil {
		return true
	}
	// SZOMB from <sys/proc.h>; x/sys/unix does not export it for darwin.
	const szomb = 5
	return kp.Proc.P_stat == szomb
}
