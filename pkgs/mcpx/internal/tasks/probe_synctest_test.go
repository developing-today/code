package tasks_test

// Temporary diagnostic probe for the #8242 synctest failures.
// DELETE before opening the PR.

import (
	"context"
	"sync/atomic"
	"testing"
	"testing/synctest"
	"time"

	"github.com/developing-today/code/pkgs/mcpx/internal/tasks"
)

// Variant 1: minimal AfterFunc + channel-blocked goroutine + Wait loop.
func TestProbeAfterFuncWaitLoop(t *testing.T) {
	synctest.Test(t, func(t *testing.T) {
		release := make(chan struct{})
		defer close(release)
		go func() { <-release }()
		var fired atomic.Bool
		time.AfterFunc(time.Hour, func() { fired.Store(true) })
		for range 10 {
			synctest.Wait()
			if fired.Load() {
				return
			}
		}
		t.Fatal("AfterFunc never fired under Wait loop")
	})
}

// Variant 2: same, but the test goroutine sleeps past the deadline.
func TestProbeAfterFuncSleep(t *testing.T) {
	synctest.Test(t, func(t *testing.T) {
		release := make(chan struct{})
		defer close(release)
		go func() { <-release }()
		var fired atomic.Bool
		time.AfterFunc(time.Hour, func() { fired.Store(true) })
		time.Sleep(2 * time.Hour)
		if !fired.Load() {
			t.Fatal("AfterFunc never fired under Sleep")
		}
	})
}

// Variant 3: blocked goroutine holds the only channel reference.
func TestProbeAfterFuncWaitLoopOrphanChannel(t *testing.T) {
	synctest.Test(t, func(t *testing.T) {
		go func() {
			ch := make(chan struct{})
			<-ch
		}()
		var fired atomic.Bool
		time.AfterFunc(time.Hour, func() { fired.Store(true) })
		for range 10 {
			synctest.Wait()
			if fired.Load() {
				return
			}
		}
		t.Fatal("AfterFunc never fired under Wait loop (orphan channel)")
	})
}

// Variant 4: the real store, but wait by sleeping instead of Wait.
func TestProbeStoreSleep(t *testing.T) {
	synctest.Test(t, func(t *testing.T) {
		st := tasks.New()
		st.CancelGrace = time.Hour
		release := make(chan struct{})
		defer close(release)
		task, err := st.Start(tasks.Caller("tester"), 0, func(ctx context.Context) (any, *tasks.Fault) {
			<-release
			return "too late", nil
		})
		if err != nil {
			t.Fatal(err)
		}
		if _, ok := st.CancelWithReason(task.TaskID, "operator stopped it"); !ok {
			t.Fatal("cancel refused")
		}
		time.Sleep(2 * time.Hour)
		snap, ok := st.Get(task.TaskID)
		if !ok {
			t.Fatal("task gone")
		}
		if snap.Status != tasks.Cancelled {
			t.Fatalf("status = %q, want cancelled; snap=%+v", snap.Status, snap)
		}
	})
}

// Variant 5: the real store with a much longer Wait loop.
func TestProbeStoreWaitLoopLong(t *testing.T) {
	synctest.Test(t, func(t *testing.T) {
		st := tasks.New()
		st.CancelGrace = time.Hour
		release := make(chan struct{})
		defer close(release)
		task, err := st.Start(tasks.Caller("tester"), 0, func(ctx context.Context) (any, *tasks.Fault) {
			<-release
			return "too late", nil
		})
		if err != nil {
			t.Fatal(err)
		}
		if _, ok := st.CancelWithReason(task.TaskID, "operator stopped it"); !ok {
			t.Fatal("cancel refused")
		}
		for range 1000 {
			synctest.Wait()
			if snap, ok := st.Get(task.TaskID); ok && tasks.Terminal(snap.Status) {
				if snap.Status != tasks.Cancelled {
					t.Fatalf("status = %q, want cancelled", snap.Status)
				}
				return
			}
		}
		snap, _ := st.Get(task.TaskID)
		t.Fatalf("never terminal after 1000 Waits: %+v", snap)
	})
}
