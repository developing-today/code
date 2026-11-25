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

// waitTerminal waits until the task reaches a terminal status,
// letting the bubble advance virtual time past each pending timer in
// turn (deadline first, then the grace timer it arms).
func waitTerminal(t *testing.T, st *tasks.Store, id string) tasks.Task {
	t.Helper()
	for range 10 {
		synctest.Wait()
		if snap, ok := st.Get(id); ok && tasks.Terminal(snap.Status) {
			return snap
		}
	}
	snap, _ := st.Get(id)
	t.Fatalf("task never reached a terminal status, got %+v", snap)
	return snap
}

func TestCancelGraceExpiresDeterministicallyInSynctest(t *testing.T) {
	synctest.Test(t, func(t *testing.T) {
		st := tasks.New()
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
		got := waitTerminal(t, st, task.TaskID)
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
		st := tasks.New()
		release := make(chan struct{})
		defer close(release)
		task, err := st.StartWithDeadline(tasks.Caller("tester"), 0, 30*time.Minute, func(ctx context.Context) (any, *tasks.Fault) {
			<-release // never finishes on its own, so the deadline must end it
			return "too late", nil
		})
		if err != nil {
			t.Fatal(err)
		}
		got := waitTerminal(t, st, task.TaskID)
		if got.Status != tasks.Cancelled {
			t.Fatalf("status = %q, want %q", got.Status, tasks.Cancelled)
		}
		if got.StatusMessage != "deadline exceeded before the task completed" {
			t.Errorf("statusMessage = %q, want the deadline reason", got.StatusMessage)
		}
	})
}
