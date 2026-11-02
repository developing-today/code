package pool

import (
	"errors"
	"fmt"
	"os"
	"syscall"
	"testing"
)

// #2336: keyPIDAlive treated an EPERM pid as dead, so a live foreign-owned
// caller's pid-scoped instances were reaped, never replaced, and skipped on
// restart. The probe seam makes every signal outcome deterministic.
func TestKeyPIDAliveProbed(t *testing.T) {
	cases := []struct {
		name  string
		key   string
		probe func(int) error
		want  bool
	}{
		{"nil error is alive", "pid:4242", func(int) error { return nil }, true},
		{"EPERM is alive", "pid:4242", func(int) error { return syscall.EPERM }, true},
		{"wrapped EPERM is alive", "pid:4242", func(int) error { return fmt.Errorf("signal 0: %w", syscall.EPERM) }, true},
		{"ESRCH is dead", "pid:4242", func(int) error { return syscall.ESRCH }, false},
		{"other error is dead", "pid:4242", func(int) error { return errors.New("boom") }, false},
		{"unparseable key is alive", "global", func(int) error { return syscall.ESRCH }, true},
		{"unparseable pid is alive", "pid:abc", func(int) error { return syscall.ESRCH }, true},
	}
	for _, c := range cases {
		t.Run(c.name, func(t *testing.T) {
			if got := keyPIDAliveProbed(c.key, c.probe); got != c.want {
				t.Errorf("keyPIDAliveProbed(%q) = %v, want %v", c.key, got, c.want)
			}
		})
	}
}

// The real probe against this test process itself: signal 0 succeeds for a
// live same-owner pid, so the un-injected path reports alive.
func TestKeyPIDAliveSelf(t *testing.T) {
	key := fmt.Sprintf("pid:%d", os.Getpid())
	if !keyPIDAlive(key) {
		t.Errorf("keyPIDAlive(%q) = false, want true for the running test process", key)
	}
}
