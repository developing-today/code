//go:build linux

package e2e_test

import (
	"os"
	"strconv"
	"strings"
	"time"

	"golang.org/x/sys/unix"
)

// installSandboxReaper makes this test binary a child subreaper so daemon
// processes it auto-started (cli/client.go Setsid+Release) reparent to it
// instead of the build sandbox's PID 1, which never waits and lets them pile
// up as zombies until the 30m timeout burns (#2686). A background waiter
// reaps whatever gets adopted; the call is a no-op if prctl fails.
//
// The reaper is session-selective (#8003), exactly as in internal/cli: a
// blanket Wait4(-1) also reaps tracked children — subprocesses the tests
// themselves are waiting on via exec.Cmd.Wait — and those Waits then lose
// the exit status. Adopted daemons are session leaders in their OWN
// session (Setsid), while tracked children run in this process's session,
// so poll /proc and Wait4 only zombie children of this process whose
// session differs from this process's session; never call Wait4(-1).
func installSandboxReaper() {
	if err := unix.Prctl(unix.PR_SET_CHILD_SUBREAPER, 1, 0, 0, 0); err != nil {
		return
	}
	go reapAdoptedDaemons(os.Getpid())
}

// reapAdoptedDaemons reaps zombie children of pid that belong to a foreign
// session (adopted daemons), leaving same-session children for their real
// waiters.
func reapAdoptedDaemons(self int) {
	ownSession := procSession(self)
	for {
		entries, err := os.ReadDir("/proc")
		if err == nil {
			for _, e := range entries {
				pid, err := strconv.Atoi(e.Name())
				if err != nil || pid == self {
					continue
				}
				st, ok := readProcStat(pid)
				if !ok || st.ppid != self || st.state != "Z" {
					continue
				}
				if ownSession != 0 && st.session == ownSession {
					continue
				}
				var status unix.WaitStatus
				_, _ = unix.Wait4(pid, &status, unix.WNOHANG, nil)
			}
		}
		time.Sleep(100 * time.Millisecond)
	}
}

type procStat struct {
	state   string
	ppid    int
	session int
}

// readProcStat parses /proc/<pid>/stat. The comm field (in parentheses) may
// contain spaces or parentheses, so fields are counted from the last ')':
// the tokens after it are state (field 3), ppid (4), pgrp (5), session (6).
func readProcStat(pid int) (procStat, bool) {
	raw, err := os.ReadFile("/proc/" + strconv.Itoa(pid) + "/stat")
	if err != nil {
		return procStat{}, false
	}
	s := string(raw)
	i := strings.LastIndex(s, ")")
	if i < 0 {
		return procStat{}, false
	}
	f := strings.Fields(s[i+1:])
	if len(f) < 4 {
		return procStat{}, false
	}
	ppid, err1 := strconv.Atoi(f[1])
	session, err2 := strconv.Atoi(f[3])
	if err1 != nil || err2 != nil {
		return procStat{}, false
	}
	return procStat{state: f[0], ppid: ppid, session: session}, true
}

func procSession(pid int) int {
	st, ok := readProcStat(pid)
	if !ok {
		return 0
	}
	return st.session
}
