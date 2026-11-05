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

func completedTask(t *testing.T, st *tasks.Store, ttl int64, runtime time.Duration) tasks.Task {
	t.Helper()
	task := st.Start(ttl, func(context.Context) (any, *tasks.Fault) {
		time.Sleep(runtime)
		return "done", nil
	})
	if _, _, err := st.Result(context.Background(), task.TaskID); err != nil {
		t.Fatal(err)
	}
	return task
}

func TestPublicItemReadRenewsTerminalRetentionButInternalReadsDoNot(t *testing.T) {
	st := tasks.New()
	task := completedTask(t, st, 500, 0)
	initial, ok := st.Get(task.TaskID)
	if !ok || initial.ExpiresAt == nil || initial.LastTouched == nil {
		t.Fatalf("terminal metadata missing: %+v", initial)
	}
	initialExpiry, initialTouch := *initial.ExpiresAt, *initial.LastTouched

	time.Sleep(300 * time.Millisecond)
	if _, ok := st.Get(task.TaskID); !ok {
		t.Fatal("internal Get lost a live task")
	}
	_ = st.List()
	if _, _, err := st.Result(context.Background(), task.TaskID); err != nil {
		t.Fatal(err)
	}
	still, ok := st.Get(task.TaskID)
	if !ok || !still.ExpiresAt.Equal(initialExpiry) || !still.LastTouched.Equal(initialTouch) {
		t.Fatalf("internal reads renewed retention: before=%+v after=%+v", initial, still)
	}

	read, ok := st.Read(task.TaskID)
	if !ok || read.ExpiresAt == nil || !read.ExpiresAt.After(initialExpiry) || !read.LastTouched.After(initialTouch) {
		t.Fatalf("public item read did not renew retention: %+v", read)
	}
	time.Sleep(250 * time.Millisecond) // past the original expiry, before the renewed one
	if _, ok := st.Get(task.TaskID); !ok {
		t.Fatal("task expired at its pre-touch deadline")
	}
}

func TestTaskMaxAgeStartsAtTerminalTransitionAndReadCannotReviveExpiry(t *testing.T) {
	st := tasks.New()
	st.Configure(500*time.Millisecond, 250*time.Millisecond)
	task := completedTask(t, st, 500, 300*time.Millisecond)
	terminal, ok := st.Get(task.TaskID)
	if !ok || terminal.ExpiresAt == nil {
		t.Fatalf("long-running task was not retained at completion: %+v", terminal)
	}
	if !terminal.ExpiresAt.After(time.Now()) {
		t.Fatalf("maxAge was anchored at creation, not terminal transition: %+v", terminal)
	}
	if terminal.ExpiresAt.After(terminal.LastUpdatedAt.Add(250 * time.Millisecond)) {
		t.Fatalf("expiry exceeded terminal-time maxAge ceiling: %+v", terminal)
	}

	// A separate short-lived result proves that a late public read cannot
	// bring an already-expired terminal result back into the store.
	short := tasks.New()
	shortTask := completedTask(t, short, 100, 0)
	snap, _ := short.Get(shortTask.TaskID)
	time.Sleep(time.Until(*snap.ExpiresAt) + 20*time.Millisecond)
	if _, ok := short.Read(shortTask.TaskID); ok {
		t.Fatal("a read revived an expired result")
	}
	if _, ok := short.Get(shortTask.TaskID); ok {
		t.Fatal("expired result remains in the store")
	}
}

func TestPublicItemReadNeverShortensPromisedRetention(t *testing.T) {
	st := tasks.New()
	st.Configure(2*time.Second, 5*time.Second)
	task := completedTask(t, st, 2_000, 0)
	initial, ok := st.Get(task.TaskID)
	if !ok || initial.ExpiresAt == nil {
		t.Fatalf("terminal task has no expiry: %+v", initial)
	}
	promised := *initial.ExpiresAt

	// A live configuration change may lower the maxAge. It must not revoke
	// an expiry already promised to this task.
	st.Configure(2*time.Second, 50*time.Millisecond)
	time.Sleep(10 * time.Millisecond)
	read, ok := st.Read(task.TaskID)
	if !ok || read.ExpiresAt == nil || !read.ExpiresAt.Equal(promised) {
		t.Fatalf("touch shortened the promised expiry: got %+v, want %v", read, promised)
	}
}

func TestTaskTouchMetadataPersistsAcrossReopen(t *testing.T) {
	dbPath := t.TempDir() + "/tasks.db"
	st1, err := tasks.Open(dbPath)
	if err != nil {
		t.Fatal(err)
	}
	st1.Configure(700*time.Millisecond, time.Second)
	task := completedTask(t, st1, 700, 0)
	time.Sleep(400 * time.Millisecond)
	touched, ok := st1.Read(task.TaskID)
	if !ok || touched.ExpiresAt == nil || touched.LastTouched == nil {
		t.Fatalf("touch failed: %+v", touched)
	}
	wantExpiry, wantTouch := *touched.ExpiresAt, *touched.LastTouched
	if err := st1.Close(); err != nil {
		t.Fatal(err)
	}
	time.Sleep(400 * time.Millisecond) // past the original expiry, before the renewed expiry

	st2, err := tasks.Open(dbPath)
	if err != nil {
		t.Fatal(err)
	}
	defer st2.Close()
	got, ok := st2.Get(task.TaskID)
	if !ok || got.ExpiresAt == nil || got.LastTouched == nil {
		t.Fatalf("touched task did not survive reopen: %+v", got)
	}
	if got.ExpiresAt.UnixMilli() != wantExpiry.UnixMilli() || got.LastTouched.UnixMilli() != wantTouch.UnixMilli() {
		t.Fatalf("touch metadata changed across reopen: got=%+v want expiry=%v touch=%v", got, wantExpiry, wantTouch)
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
