package cli

import (
	"context"
	"errors"
	"os"
	"path/filepath"
	"strings"
	"testing"
	"time"

	"github.com/developing-today/code/pkgs/mcpx/internal/daemon"
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
	c, marker := ensureTestClient(t, "30s")
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
	// The prompt return above is only the right behaviour if the daemon
	// was actually started and the cancellation unwound the wait for it:
	// a context.Canceled return with no spawn at all (a cancellation
	// check ahead of the spawn, say) would pass both assertions while
	// never starting anything. The spawn is cmd.Start + Release
	// (client.go): Start returns once the process exists, before the
	// child has run any code, and this test's whole point is that
	// EnsureDaemon returns the instant cancellation lands — so the
	// child's touch can legitimately land after the return. Asserting
	// the marker instantly races the child's scheduling (the flake that
	// got the dezren39/nix copy of this test skipped in its darwin nix
	// build, dezren39/nix#428). The child is Setsid and Released, so it
	// still runs: wait for the marker instead, and fail only if it never
	// appears. Ported from the dezren39/nix copy in #361; its
	// TestEnsureDaemonCancelWhilePolling has asserted this all along.
	deadline := time.Now().Add(5 * time.Second)
	for {
		if _, statErr := os.Stat(marker); statErr == nil {
			break
		}
		if time.Now().After(deadline) {
			t.Fatal("expected the daemon spawn to have happened before cancellation")
		}
		time.Sleep(10 * time.Millisecond)
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
	// The spawn is cmd.Start + Release (client.go): Start returns once the
	// process exists, before the child has run any code, so the child's
	// touch can legitimately land after EnsureDaemon returns at the
	// connect timeout. Asserting the marker instantly races the child's
	// scheduling — the same flake as dezren39/nix#428, in the test this
	// repo's #437 port added without that pattern (dezren39/nix#445,
	// origin issue for this set). The child is Setsid and Released, so it
	// still runs: wait for the marker instead, and fail only if it never
	// appears.
	deadline := time.Now().Add(5 * time.Second)
	for {
		if _, statErr := os.Stat(marker); statErr == nil {
			break
		}
		if time.Now().After(deadline) {
			t.Fatal("the fake daemon was never spawned")
		}
		time.Sleep(10 * time.Millisecond)
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

// errAfterContext reports context.Canceled from Err once its deadline
// passes, but its Done channel never fires. It models a cancellation that
// only becomes visible as a poll loop exits on its own deadline: an
// in-loop select on Done cannot catch it, only a post-loop Err check can.
type errAfterContext struct {
	context.Context
	at time.Time
}

func (c errAfterContext) Done() <-chan struct{} { return nil }

func (c errAfterContext) Err() error {
	if time.Now().Before(c.at) {
		return nil
	}
	return context.Canceled
}

// Regression for #328: a cancellation visible only at the poll loop's
// deadline exit was reported as "daemon did not become ready" instead of
// context.Canceled, because EnsureDaemon never re-checked ctx.Err()
// after the loop (the dezren39/nix copy has had that check since #233).
func TestEnsureDaemonCancelVisibleOnlyAtDeadline(t *testing.T) {
	c, _ := ensureTestClient(t, "300ms")
	ctx := errAfterContext{Context: context.Background(), at: time.Now().Add(100 * time.Millisecond)}
	err := c.EnsureDaemon(ctx)
	if !errors.Is(err, context.Canceled) {
		t.Fatalf("err = %v, want context.Canceled", err)
	}
}
