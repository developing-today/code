package e2e_test

import (
	"os/exec"
	"testing"
)

// TestExitedTellsALiveProcessFromAGoneOne is the only coverage of the
// exited helper itself, which every awaitExit caller trusts. On darwin it
// used to read /proc, which does not exist there, so it called every
// process exited and awaitExit never waited (TestShutdownStopsTheDaemon
// OverTheSocket then found the daemon's socket still answering).
//
// Oracle: the test owns the process's lifecycle. A started, unsignalled
// child is alive; once killed it is a zombie until Wait reaps it; after Wait
// its pid is gone. exited must read alive, exited, exited.
func TestExitedTellsALiveProcessFromAGoneOne(t *testing.T) {
	cmd := exec.Command("sleep", "60")
	if err := cmd.Start(); err != nil {
		t.Fatal(err)
	}
	pid := cmd.Process.Pid
	t.Cleanup(func() { _ = cmd.Process.Kill(); _ = cmd.Wait() })

	if exited(pid) {
		t.Fatalf("exited(%d) reports a running child as exited", pid)
	}
	if err := cmd.Process.Kill(); err != nil {
		t.Fatal(err)
	}
	awaitExit(t, pid, "the killed, unreaped child")
	if err := cmd.Wait(); err == nil {
		t.Fatal("a killed child should report a non-nil wait status")
	}
	if !exited(pid) {
		t.Fatalf("exited(%d) reports a reaped child as alive", pid)
	}
}
