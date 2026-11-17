//go:build linux

package e2e_test

// The explore exercise for the #8463 inspection-group burn-down.
// explore refuses to run without a terminal on stdin (see
// TestExploreRefusesWithoutATerminal), and the harness runners only
// ever offer it a pipe — so this file gives the env a runner whose
// stdin is a real pty. Linux-only: the pty is allocated through
// /dev/ptmx ioctls.

import (
	"context"
	"fmt"
	"io"
	"os"
	"os/exec"
	"strings"
	"syscall"
	"testing"
	"time"
	"unsafe"

	"golang.org/x/sys/unix"
)

// ptyRunner runs the env's binary with a pty slave as its standard
// input: the one arrangement under which the terminal-only commands
// will run at all.
type ptyRunner struct {
	slave *os.File
}

func (ptyRunner) name() string       { return "pty" }
func (ptyRunner) needsProcess() bool { return true }

func (r ptyRunner) run(ctx context.Context, e *env, _ io.Reader, args ...string) (string, error) {
	ctx, cancel := context.WithTimeout(ctx, 90*time.Second)
	defer cancel()
	cmd := exec.CommandContext(ctx, e.mcpx, args...)
	cmd.Dir = e.dir
	cmd.Env = e.envVars
	cmd.Stdin = r.slave
	out, err := cmd.CombinedOutput()
	return string(out), harnessTimeout(ctx, err)
}

func openPTY() (master, slave *os.File, err error) {
	master, err = os.OpenFile("/dev/ptmx", os.O_RDWR, 0)
	if err != nil {
		return nil, nil, err
	}
	// TIOCGPTN and TIOCSPTLCK both take a pointer to an int, so they
	// cannot go through the by-value ioctl helpers.
	var n uint32
	if _, _, errno := unix.Syscall(syscall.SYS_IOCTL, master.Fd(),
		uintptr(unix.TIOCGPTN), uintptr(unsafe.Pointer(&n))); errno != 0 {
		_ = master.Close()
		return nil, nil, errno
	}
	var unlock int32
	if _, _, errno := unix.Syscall(syscall.SYS_IOCTL, master.Fd(),
		uintptr(unix.TIOCSPTLCK), uintptr(unsafe.Pointer(&unlock))); errno != 0 {
		_ = master.Close()
		return nil, nil, errno
	}
	slave, err = os.OpenFile(fmt.Sprintf("/dev/pts/%d", n), os.O_RDWR, 0)
	if err != nil {
		_ = master.Close()
		return nil, nil, err
	}
	return master, slave, nil
}

func TestExploreBrowsesUnderATerminal(t *testing.T) {
	master, slave, err := openPTY()
	if err != nil {
		t.Fatalf("allocate pty: %v", err)
	}
	// The pty must outlive the env's own cleanup, which stops the
	// daemon by running one more command through this runner — so
	// the close is registered before newEnv registers its cleanup
	// (cleanups run last-in, first-out).
	t.Cleanup(func() {
		_ = master.Close()
		_ = slave.Close()
	})
	// Answer the prompt loop's first read with "quit", up front: the
	// pty buffers it until explore reads.
	if _, err := master.WriteString("q\n"); err != nil {
		t.Fatalf("queue the quit command: %v", err)
	}

	e := newEnv(t, oneServer)
	e.runner = ptyRunner{slave: slave}
	out := e.run("explore")
	if !strings.Contains(out, "mcpx explore") {
		t.Fatalf("explore did not open with its banner:\n%s", out)
	}
	if !strings.Contains(out, "demo") {
		t.Fatalf("explore's first screen does not list the namespace:\n%s", out)
	}
}
