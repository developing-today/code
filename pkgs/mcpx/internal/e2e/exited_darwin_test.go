package e2e_test

import "golang.org/x/sys/unix"

// exited reports whether a process is gone or a zombie; processExists alone
// counts a zombie as alive.
//
// Darwin has no /proc. The Linux reading of /proc/<pid>/stat failed there for
// every pid, so awaitExit returned at once while the daemon was still
// running, and TestShutdownStopsTheDaemonOverTheSocket found its socket still
// answering on every Mac and in the aarch64-darwin nix build. The kernel's
// own process table is read instead, the way mcpclient's procStat does.
func exited(pid int) bool {
	kp, err := unix.SysctlKinfoProc("kern.proc.pid", pid)
	if err != nil {
		return true
	}
	// SZOMB from <sys/proc.h>; x/sys/unix does not export it for darwin.
	const szomb = 5
	return kp.Proc.P_stat == szomb
}
