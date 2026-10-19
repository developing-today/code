package cli

import (
	"context"
	"errors"
	"os"
	"path/filepath"
	"strings"
	"testing"
	"time"

	"github.com/dezren39/mcpx/internal/daemon"
)

// ensureTestClient builds a client whose autostart "daemon" is a fake
// binary that only records that it was spawned, so tests can observe
// whether EnsureDaemon started anything without a real daemon.
func ensureTestClient(t *testing.T, connectTimeout string) (*Client, string) {
	t.Helper()
	dir := t.TempDir()
	marker := filepath.Join(dir, "spawned")
	bin := filepath.Join(dir, "fakebin.sh")
	script := "#!/bin/sh\ntouch \"" + marker + "\"\n"
	if err := os.WriteFile(bin, []byte(script), 0o755); err != nil {
		t.Fatal(err)
	}
	c := NewClient(daemon.PathsAt(filepath.Join(dir, "state"), filepath.Join(dir, "cache")), "")
	for path, raw := range map[string]string{
		"autostart.connectTimeout": connectTimeout,
		"autostart.pollInterval":   "50ms",
		"autostart.bin":            bin,
	} {
		if err := c.set.SetRuntime(path, raw); err != nil {
			t.Fatal(err)
		}
	}
	return c, marker
}

// Regression for #284: an already-cancelled context was read as "no
// daemon", so EnsureDaemon spawned a daemon and waited out the whole
// connect timeout before failing with the wrong error.
func TestEnsureDaemonAlreadyCancelledContext(t *testing.T) {
	c, marker := ensureTestClient(t, "3s")
	ctx, cancel := context.WithCancel(context.Background())
	cancel()
	start := time.Now()
	err := c.EnsureDaemon(ctx)
	if !errors.Is(err, context.Canceled) {
		t.Fatalf("err = %v, want context.Canceled", err)
	}
	if elapsed := time.Since(start); elapsed > time.Second {
		t.Fatalf("took %v with a cancelled context, want a prompt return", elapsed)
	}
	if _, statErr := os.Stat(marker); statErr == nil {
		t.Fatal("spawned a daemon for a cancelled caller")
	}
}

// Regression for #284: cancelling while EnsureDaemon waits for the
// started daemon must end the wait promptly, not at the deadline.
func TestEnsureDaemonCancelledWhileWaiting(t *testing.T) {
	c, _ := ensureTestClient(t, "30s")
	ctx, cancel := context.WithCancel(context.Background())
	go func() {
		time.Sleep(200 * time.Millisecond)
		cancel()
	}()
	start := time.Now()
	err := c.EnsureDaemon(ctx)
	if !errors.Is(err, context.Canceled) {
		t.Fatalf("err = %v, want context.Canceled", err)
	}
	if elapsed := time.Since(start); elapsed > 5*time.Second {
		t.Fatalf("took %v after cancellation, want a prompt return", elapsed)
	}
}

// The uncancelled path is unchanged: a daemon that never becomes ready
// still fails with "did not become ready" after the connect timeout.
func TestEnsureDaemonNeverReadyStillTimesOut(t *testing.T) {
	c, marker := ensureTestClient(t, "1s")
	start := time.Now()
	err := c.EnsureDaemon(context.Background())
	if err == nil || !strings.Contains(err.Error(), "daemon did not become ready") {
		t.Fatalf("err = %v, want the did-not-become-ready timeout error", err)
	}
	if elapsed := time.Since(start); elapsed < 900*time.Millisecond {
		t.Fatalf("gave up after %v, before the 1s connect timeout", elapsed)
	}
	if _, statErr := os.Stat(marker); statErr != nil {
		t.Fatal("the fake daemon was never spawned")
	}
}

// A cancelled context against a remote endpoint reports the cancellation,
// not the generic "no daemon there" error.
func TestEnsureDaemonRemoteCancelledContext(t *testing.T) {
	c := NewClientAt(daemon.Paths{}, "", "http://127.0.0.1:1")
	ctx, cancel := context.WithCancel(context.Background())
	cancel()
	if err := c.EnsureDaemon(ctx); !errors.Is(err, context.Canceled) {
		t.Fatalf("EnsureDaemon(remote, cancelled ctx) = %v, want context.Canceled", err)
	}
}
