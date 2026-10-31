package tasks_test

import (
	"context"
	"errors"
	"strings"
	"testing"
	"time"

	"github.com/dezren39/mcpx/internal/tasks"
)

func TestAResultIsWaitedForRatherThanPolledFor(t *testing.T) {
	st := tasks.New()
	task := st.Start(0, func(ctx context.Context) (any, *tasks.Fault) {
		time.Sleep(20 * time.Millisecond)
		return map[string]any{"ok": true}, nil
	})
	if task.Status != tasks.Working {
		t.Fatalf("a fresh task is working, got %q", task.Status)
	}
	result, fault, err := st.Result(context.Background(), task.TaskID)
	if err != nil || fault != nil {
		t.Fatalf("err=%v fault=%v", err, fault)
	}
	if m, _ := result.(map[string]any); m["ok"] != true {
		t.Errorf("result = %v", result)
	}
	snap, ok := st.Get(task.TaskID)
	if !ok || snap.Status != tasks.Completed {
		t.Errorf("after collection the task is completed, got %+v", snap)
	}
}

func TestAToolResultThatIsItselfAnErrorIsAFailedTask(t *testing.T) {
	// The specification's own note on TaskStatus. Reporting it as completed
	// makes a client that checks only the status miss every failure.
	st := tasks.New()
	task := st.Start(0, func(ctx context.Context) (any, *tasks.Fault) {
		return map[string]any{"isError": true}, nil
	})
	if _, _, err := st.Result(context.Background(), task.TaskID); err != nil {
		t.Fatal(err)
	}
	snap, _ := st.Get(task.TaskID)
	if snap.Status != tasks.Failed {
		t.Errorf("status = %q", snap.Status)
	}
}

func TestCancellingStopsTheWorkAndStands(t *testing.T) {
	st := tasks.New()
	released := make(chan struct{})
	task := st.Start(0, func(ctx context.Context) (any, *tasks.Fault) {
		<-ctx.Done()
		close(released)
		return "too late", nil
	})
	snap, ok := st.Cancel(task.TaskID)
	if !ok || snap.Status != tasks.Cancelled {
		t.Fatalf("cancel returned %+v", snap)
	}
	select {
	case <-released:
	case <-time.After(time.Second):
		t.Fatal("cancelling should cancel the context the work runs under")
	}
	_, fault, err := st.Result(context.Background(), task.TaskID)
	if err != nil {
		t.Fatal(err)
	}
	if fault == nil {
		t.Fatal("collecting a cancelled task should report the cancellation")
	}
	if after, _ := st.Get(task.TaskID); after.Status != tasks.Cancelled {
		t.Errorf("a cancellation stands even though the work returned: %q", after.Status)
	}
}

func TestAnUnknownHandleSaysItMayHaveExpired(t *testing.T) {
	// Tasks are deleted at their TTL, so "no such task" is usually "you were
	// too slow" rather than "you made that up", and the message has to say
	// which.
	st := tasks.New()
	_, _, err := st.Result(context.Background(), "tsk-nothing")
	var missing tasks.ErrNoTask
	if !errors.As(err, &missing) {
		t.Fatalf("err = %v", err)
	}
	if got := missing.Error(); got == "" || !strings.Contains(got, "expired") {
		t.Errorf("message = %q", got)
	}
}

func TestWaitingIsBoundedByTheCallersContext(t *testing.T) {
	st := tasks.New()
	task := st.Start(0, func(ctx context.Context) (any, *tasks.Fault) {
		<-ctx.Done()
		return nil, nil
	})
	ctx, cancel := context.WithTimeout(context.Background(), 30*time.Millisecond)
	defer cancel()
	if _, _, err := st.Result(ctx, task.TaskID); !errors.Is(err, context.DeadlineExceeded) {
		t.Fatalf("a caller that waited long enough should stop waiting, not the task: %v", err)
	}
	if snap, _ := st.Get(task.TaskID); snap.Status != tasks.Working {
		t.Errorf("the task keeps running: %q", snap.Status)
	}
}

func TestTaskPersistenceAcrossReopen(t *testing.T) {
	dbPath := t.TempDir() + "/tasks.db"
	st1, err := tasks.Open(dbPath)
	if err != nil {
		t.Fatalf("Open failed: %v", err)
	}

	// 1. Completed task
	t1 := st1.Start(60000, func(ctx context.Context) (any, *tasks.Fault) {
		return map[string]any{"data": "persisted-result"}, nil
	})
	// Wait for t1 to complete
	_, _, err = st1.Result(context.Background(), t1.TaskID)
	if err != nil {
		t.Fatalf("t1 result: %v", err)
	}

	// 2. Working task that was interrupted
	t2 := st1.Start(60000, func(ctx context.Context) (any, *tasks.Fault) {
		<-ctx.Done()
		return nil, nil
	})

	_ = st1.Close()

	// Reopen in new store instance
	st2, err := tasks.Open(dbPath)
	if err != nil {
		t.Fatalf("Open reopen failed: %v", err)
	}
	defer st2.Close()

	// t1 should still exist and be completed with its result
	snap1, ok := st2.Get(t1.TaskID)
	if !ok {
		t.Fatalf("t1 missing after reopen")
	}
	if snap1.Status != tasks.Completed {
		t.Errorf("t1 status = %q, want completed", snap1.Status)
	}
	res, fault, err := st2.Result(context.Background(), t1.TaskID)
	if err != nil || fault != nil {
		t.Errorf("t1 result err=%v fault=%v", err, fault)
	}
	if m, _ := res.(map[string]any); m["data"] != "persisted-result" {
		t.Errorf("t1 result data = %v", res)
	}

	// t2 (which was working when st1 was closed) should be marked failed due to daemon restart
	snap2, ok := st2.Get(t2.TaskID)
	if !ok {
		t.Fatalf("t2 missing after reopen")
	}
	if snap2.Status != tasks.Failed {
		t.Errorf("t2 status = %q, want failed", snap2.Status)
	}
	if !strings.Contains(snap2.StatusMessage, "daemon restarted") {
		t.Errorf("t2 statusMessage = %q", snap2.StatusMessage)
	}
}

func TestCooperativeCancellationHandshake(t *testing.T) {
	st := tasks.New()
	cooperativeCalled := false
	var cancelReason string

	task := st.Start(0, func(ctx context.Context) (any, *tasks.Fault) {
		<-ctx.Done()
		return nil, nil
	})

	st.SetTaskCancel(task.TaskID, func(ctx context.Context, reason string) {
		cooperativeCalled = true
		cancelReason = reason
	})

	snap, ok := st.CancelWithReason(task.TaskID, "user aborted run")
	if !ok || snap.Status != tasks.Cancelled {
		t.Fatalf("cancel returned %+v", snap)
	}
	if !cooperativeCalled {
		t.Error("expected cooperative cancel hook to be called")
	}
	if cancelReason != "user aborted run" {
		t.Errorf("expected reason 'user aborted run', got %q", cancelReason)
	}
}

func TestAWorkingTaskOutlivesItsTTLAndItsResultStaysCollectable(t *testing.T) {
	// Regression (#1653, Bug A): the TTL is retention from completion,
	// not a runtime deadline — a task still working when its TTL passes
	// must not be deleted, and its context must not be cancelled by the
	// retention clock.
	st := tasks.New()
	task := st.Start(80, func(ctx context.Context) (any, *tasks.Fault) {
		time.Sleep(300 * time.Millisecond)
		if ctx.Err() != nil {
			return nil, &tasks.Fault{Code: -32603, Message: "the retention clock cancelled the work"}
		}
		return map[string]any{"ok": true}, nil
	})
	time.Sleep(160 * time.Millisecond) // past the TTL, still working
	snap, ok := st.Get(task.TaskID)
	if !ok {
		t.Fatal("a working task was deleted at its TTL")
	}
	if snap.Status != tasks.Working {
		t.Fatalf("status while working past the TTL = %q", snap.Status)
	}
	result, fault, err := st.Result(context.Background(), task.TaskID)
	if err != nil || fault != nil {
		t.Fatalf("err=%v fault=%v", err, fault)
	}
	if m, _ := result.(map[string]any); m["ok"] != true {
		t.Errorf("result = %v", result)
	}
	if _, ok := st.Get(task.TaskID); !ok {
		t.Error("a just-finished task vanished instead of starting its retention")
	}
}

func TestAFinishedTaskKeepsItsResultForTheFullTTLAfterFinishing(t *testing.T) {
	// Regression (#1653, Bug B): retention is the full TTL measured from
	// the terminal transition, not what is left of a creation clock.
	st := tasks.New()
	task := st.Start(400, func(ctx context.Context) (any, *tasks.Fault) {
		time.Sleep(250 * time.Millisecond)
		return "done", nil
	})
	if _, _, err := st.Result(context.Background(), task.TaskID); err != nil {
		t.Fatal(err)
	}
	time.Sleep(250 * time.Millisecond) // 500ms from creation, 250ms from finishing
	snap, ok := st.Get(task.TaskID)
	if !ok || snap.Status != tasks.Completed {
		t.Fatalf("a finished task's retention shrank by its runtime: %+v (present=%v)", snap, ok)
	}
	time.Sleep(350 * time.Millisecond) // 600ms from finishing: past the full TTL
	if _, ok := st.Get(task.TaskID); ok {
		t.Error("retention should lapse a full TTL after finishing")
	}
}

func TestRetentionAcrossReopenRunsFromCompletion(t *testing.T) {
	// Regression (#1653, reload path): a reopened store computes expiry
	// from when the task finished, not from when it was created, so a
	// task that ran most of its TTL is not deleted on sight at restart.
	dbPath := t.TempDir() + "/tasks.db"
	st1, err := tasks.Open(dbPath)
	if err != nil {
		t.Fatalf("Open failed: %v", err)
	}
	t1 := st1.Start(400, func(ctx context.Context) (any, *tasks.Fault) {
		time.Sleep(300 * time.Millisecond)
		return map[string]any{"data": "kept"}, nil
	})
	if _, _, err := st1.Result(context.Background(), t1.TaskID); err != nil {
		t.Fatalf("t1 result: %v", err)
	}
	_ = st1.Close()
	time.Sleep(200 * time.Millisecond) // 500ms from creation, 200ms from finishing

	st2, err := tasks.Open(dbPath)
	if err != nil {
		t.Fatalf("Open reopen failed: %v", err)
	}
	defer st2.Close()
	snap, ok := st2.Get(t1.TaskID)
	if !ok {
		t.Fatal("t1 was deleted at reopen: its retention was clocked from creation")
	}
	if snap.Status != tasks.Completed {
		t.Errorf("t1 status = %q, want completed", snap.Status)
	}
	res, fault, err := st2.Result(context.Background(), t1.TaskID)
	if err != nil || fault != nil {
		t.Errorf("t1 result err=%v fault=%v", err, fault)
	}
	if m, _ := res.(map[string]any); m["data"] != "kept" {
		t.Errorf("t1 result data = %v", res)
	}
}

func TestAStartDeadlineCancelsWaitingWork(t *testing.T) {
	// The askTTL shape (#1653 ruling 3): StartWithDeadline bounds the
	// work itself through the ordinary cancel path, so a parked call
	// nobody answers does not wait forever now that the retention clock
	// no longer kills running tasks.
	st := tasks.New()
	task := st.StartWithDeadline(60000, 50*time.Millisecond, func(ctx context.Context) (any, *tasks.Fault) {
		<-ctx.Done()
		return nil, nil
	})
	deadline := time.Now().Add(2 * time.Second)
	for {
		snap, ok := st.Get(task.TaskID)
		if ok && snap.Status == tasks.Cancelled {
			return
		}
		if time.Now().After(deadline) {
			t.Fatalf("the deadline never cancelled the waiting work: %+v", snap)
		}
		time.Sleep(10 * time.Millisecond)
	}
}
