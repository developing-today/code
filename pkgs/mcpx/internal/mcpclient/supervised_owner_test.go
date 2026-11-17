package mcpclient_test

import (
	"errors"
	"os"
	"os/exec"
	"strconv"
	"syscall"
	"testing"
	"time"

	"github.com/developing-today/code/pkgs/mcpx/internal/mcpclient"
)

// TestSupervisorOwnerDeathReapsChild is the #10774 regression test.
// Oracle: a supervisor whose owner process dies terminates its
// child and removes its socket within a bounded time, with no
// terminate handshake — a spawner killed mid-run (a test binary
// under a timeout or OOM, the pool.test runaway) must not leak
// detached supervisors for the child's whole lifetime.
func TestSupervisorOwnerDeathReapsChild(t *testing.T) {
	owner := exec.Command("sleep", "60")
	if err := owner.Start(); err != nil {
		t.Fatalf("start owner: %v", err)
	}
	t.Setenv(mcpclient.SupervisorOwnerEnv, strconv.Itoa(owner.Process.Pid))

	opts := supervisedOpts(t, "ownerwatch", "cat")
	tr, err := mcpclient.NewSupervised(opts)
	if err != nil {
		t.Fatalf("attach: %v", err)
	}
	childPID := tr.PID()
	if childPID <= 0 {
		t.Fatalf("child pid = %d", childPID)
	}
	sock := mcpclient.SupervisorSocketPath(opts.StateDir, opts.Name)
	if _, err := os.Stat(sock); err != nil {
		t.Fatalf("socket before owner death: %v", err)
	}

	// Kill and reap the owner. Reaping matters: a zombie owner
	// still answers kill(pid, 0) — exactly why the watch keys on
	// ESRCH, and why a dead test binary (reaped by go test) is
	// detectable in the leak scenario.
	_ = owner.Process.Kill()
	_ = owner.Wait()

	deadline := time.Now().Add(20 * time.Second)
	for {
		_, sockErr := os.Stat(sock)
		childErr := syscall.Kill(childPID, 0)
		if errors.Is(sockErr, os.ErrNotExist) && errors.Is(childErr, syscall.ESRCH) {
			break
		}
		if time.Now().After(deadline) {
			_ = tr.Close()
			t.Fatalf("supervisor leaked after owner death: socket stat err=%v, child kill err=%v", sockErr, childErr)
		}
		time.Sleep(100 * time.Millisecond)
	}
	_ = tr.Close()
}
