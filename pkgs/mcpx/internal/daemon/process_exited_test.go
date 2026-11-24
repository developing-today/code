package daemon

import (
	"os/exec"
	"testing"
	"time"
)

// TestProcessExitedTellsALiveProcessFromAGoneOne is the only coverage of
// the processExited helper itself, which
// TestScriptViewCommandTimeoutKillsSpawnedWorkers trusts.
//
// Oracle: the test owns the process's lifecycle. A started, unsignalled
// child is alive; once killed it is a zombie until Wait reaps it; after
// Wait its pid is gone. processExited must read alive, exited, exited.
// A signal-0 probe reads the middle step as alive, which is exactly the
// #13713 failure: in a container whose PID 1 does not reap orphans, the
// killed worker stays a zombie and the timeout test never sees it die.
func TestProcessExitedTellsALiveProcessFromAGoneOne(t *testing.T) {
	cmd := exec.Command("sleep", "60")
	if err := cmd.Start(); err != nil {
		t.Fatal(err)
	}
	pid := cmd.Process.Pid
	t.Cleanup(func() { _ = cmd.Process.Kill(); _ = cmd.Wait() })

	if processExited(pid) {
		t.Fatalf("processExited(%d) reports a running child as exited", pid)
	}
	if err := cmd.Process.Kill(); err != nil {
		t.Fatal(err)
	}
	deadline := time.Now().Add(5 * time.Second)
	for !processExited(pid) {
		if time.Now().After(deadline) {
			t.Fatalf("processExited(%d) reports a killed, unreaped child as alive", pid)
		}
		time.Sleep(50 * time.Millisecond)
	}
	if err := cmd.Wait(); err == nil {
		t.Fatal("a killed child should report a non-nil wait status")
	}
	if !processExited(pid) {
		t.Fatalf("processExited(%d) reports a reaped child as alive", pid)
	}
}
