package tasks_test

import (
	"context"
	"testing"
	"testing/synctest"
	"time"

	"github.com/developing-today/code/pkgs/mcpx/internal/tasks"
)

// The #8242 evaluation's smallest first step: extend the #2639
// synctest pilot from elicit to tasks. The store's two timers — the
// StartWithDeadline bound and the CancelWithReason grace timer — are
// plain time.AfterFunc timers, so inside a synctest bubble they run
// on virtual time: an hour of grace costs microseconds of wall clock,
// the firing order is fixed, and the outcome reproduces exactly on
// every run, with no seed and no sleep-as-scheduler-hint.

// runOutClock lets virtual time run past every store timer armed so
// far, then waits for the fired callbacks to finish.
//
// The order matters, and the first version of these tests had it
// backwards. Per the synctest package docs, when every goroutine in
// a bubble is durably blocked, a pending synctest.Wait returns —
// time advances only when Wait has NOT been called. A loop of Wait
// calls therefore consumes each idle moment by returning from Wait
// instead of letting the clock move, and the timers never fire.
// Blocking the test goroutine on time.Sleep is what advances the
// clock (the pool budget pilot's shape); the single Wait afterwards
// only synchronizes with the timer callbacks, which by then have
// either run or are running to their own blocked state.
func runOutClock(d time.Duration) {
	time.Sleep(d)
	synctest.Wait()
}

// newTestStore is a store whose retention cannot delete the task
// mid-test: the built-in TTL is ten minutes from the terminal
// transition, so over a two-hour virtual window a cancelled task
// would be retention-deleted before the test reads it back. The
// knobs are the store's own exported configuration; nothing about
// the timers under test changes.
func newTestStore() *tasks.Store {
	st := tasks.New()
	st.Configure(24*time.Hour, 24*time.Hour)
	st.MaxTTL = 24 * time.Hour
	return st
}

// terminalAfter runs the clock out and returns the task's snapshot,
// failing unless it reached a terminal status.
func terminalAfter(t *testing.T, st *tasks.Store, id string, d time.Duration) tasks.Task {
	t.Helper()
	runOutClock(d)
	snap, ok := st.Get(id)
	if !ok {
		t.Fatalf("task %s not found after the clock ran out", id)
	}
	if !tasks.Terminal(snap.Status) {
		t.Fatalf("task never reached a terminal status, got %+v", snap)
	}
	return snap
}

func TestCancelGraceExpiresDeterministicallyInSynctest(t *testing.T) {
	synctest.Test(t, func(t *testing.T) {
		st := newTestStore()
		st.CancelGrace = time.Hour // virtual: the grace is an hour, the test is not
		release := make(chan struct{})
		defer close(release) // let the work goroutine out at the end
		task, err := st.Start(tasks.Caller("tester"), 0, func(ctx context.Context) (any, *tasks.Fault) {
			<-release // ignores its context: the uncooperative-work case
			return "too late", nil
		})
		if err != nil {
			t.Fatal(err)
		}
		snap, ok := st.CancelWithReason(task.TaskID, "operator stopped it")
		if !ok || snap.Status != tasks.Working {
			t.Fatalf("cancel should return the pre-cancel wire state, got %+v", snap)
		}
		// The grace timer fires one virtual hour after the cancel;
		// run the clock to two hours so the expiry has landed.
		got := terminalAfter(t, st, task.TaskID, 2*time.Hour)
		if got.Status != tasks.Cancelled {
			t.Fatalf("status = %q, want %q", got.Status, tasks.Cancelled)
		}
		if got.StatusMessage != "operator stopped it" {
			t.Errorf("statusMessage = %q, want the cancel reason", got.StatusMessage)
		}
	})
}

func TestDeadlineCancelsDeterministicallyInSynctest(t *testing.T) {
	synctest.Test(t, func(t *testing.T) {
		st := newTestStore()
		// Pinned so the timer chain is fully determined by this test:
		// the deadline fires at 30 minutes and starts a one-hour
		// grace; the cancel lands when the grace expires.
		st.CancelGrace = time.Hour
		release := make(chan struct{})
		defer close(release)
		task, err := st.StartWithDeadline(tasks.Caller("tester"), 0, 30*time.Minute, func(ctx context.Context) (any, *tasks.Fault) {
			<-release // never finishes on its own, so the deadline must end it
			return "too late", nil
		})
		if err != nil {
			t.Fatal(err)
		}
		// Deadline (30m) + grace (1h) = 90 virtual minutes to the
		// terminal transition; run the clock to two hours.
		got := terminalAfter(t, st, task.TaskID, 2*time.Hour)
		if got.Status != tasks.Cancelled {
			t.Fatalf("status = %q, want %q", got.Status, tasks.Cancelled)
		}
		if got.StatusMessage != "deadline exceeded before the task completed" {
			t.Errorf("statusMessage = %q, want the deadline reason", got.StatusMessage)
		}
	})
}
