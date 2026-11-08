package tasks_test

import (
	"context"
	"database/sql"
	"errors"
	"strings"
	"sync"
	"testing"
	"time"

	"github.com/developing-today/code/pkgs/mcpx/internal/tasks"
	_ "modernc.org/sqlite"
)

func TestAResultIsWaitedForRatherThanPolledFor(t *testing.T) {
	st := tasks.New()
	task, _ := st.Start(tasks.Caller("tester"), 0, func(ctx context.Context) (any, *tasks.Fault) {
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
	task, _ := st.Start(tasks.Caller("tester"), 0, func(ctx context.Context) (any, *tasks.Fault) {
		return map[string]any{"isError": true, "content": []any{
			map[string]any{"type": "text", "text": "tool failed"},
		}}, nil
	})
	if _, _, err := st.Result(context.Background(), task.TaskID); err != nil {
		t.Fatal(err)
	}
	snap, _ := st.Get(task.TaskID)
	if snap.Status != tasks.Failed {
		t.Errorf("status = %q", snap.Status)
	}
	if snap.StatusMessage != "tool failed" {
		t.Errorf("statusMessage = %q, want first text content", snap.StatusMessage)
	}
}

func TestCancellingStopsTheWorkAndStands(t *testing.T) {
	st := tasks.New()
	st.CancelGrace = time.Second
	released := make(chan struct{})
	task, _ := st.Start(tasks.Caller("tester"), 0, func(ctx context.Context) (any, *tasks.Fault) {
		<-ctx.Done()
		close(released)
		return "too late", nil
	})
	snap, ok := st.Cancel(task.TaskID)
	if !ok || snap.Status != tasks.Working {
		t.Fatalf("cancel should return the pre-cancel wire state, got %+v", snap)
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
	if fault != nil {
		t.Fatalf("work that finished within the grace period should keep its result, fault=%v", fault)
	}
	if after, _ := st.Get(task.TaskID); after.Status != tasks.Completed {
		t.Errorf("work finishing during cancellation should be completed, got %q", after.Status)
	} else if after.StatusMessage != "completed during cancellation" {
		t.Errorf("statusMessage = %q, want cancellation completion note", after.StatusMessage)
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
	task, _ := st.Start(tasks.Caller("tester"), 0, func(ctx context.Context) (any, *tasks.Fault) {
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
	t1, _ := st1.Start(tasks.Caller("tester"), 60000, func(ctx context.Context) (any, *tasks.Fault) {
		return map[string]any{"data": "persisted-result"}, nil
	})
	// Wait for t1 to complete
	_, _, err = st1.Result(context.Background(), t1.TaskID)
	if err != nil {
		t.Fatalf("t1 result: %v", err)
	}

	// 2. Working task that was interrupted
	t2, _ := st1.Start(tasks.Caller("tester"), 60000, func(ctx context.Context) (any, *tasks.Fault) {
		<-ctx.Done()
		return nil, nil
	})

	_ = st1.Close()

	// Reopen in new store instance
	st2, err := tasks.Open(dbPath)
	if err != nil {
		t.Fatalf("Open reopen failed: %v", err)
	}
	defer func() { _ = st2.Close() }()

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
	st.CancelGrace = time.Second
	cooperativeCalled := false
	var cancelReason string

	task, _ := st.Start(tasks.Caller("tester"), 0, func(ctx context.Context) (any, *tasks.Fault) {
		<-ctx.Done()
		return nil, nil
	})

	st.SetTaskCancel(task.TaskID, func(ctx context.Context, reason string) {
		cooperativeCalled = true
		cancelReason = reason
	})

	snap, ok := st.CancelWithReason(task.TaskID, "user aborted run")
	if !ok || snap.Status != tasks.Working {
		t.Fatalf("cancel should preserve the pre-cancel status, got %+v", snap)
	}
	if !cooperativeCalled {
		t.Error("expected cooperative cancel hook to be called")
	}
	if cancelReason != "user aborted run" {
		t.Errorf("expected reason 'user aborted run', got %q", cancelReason)
	}
	if _, _, err := st.Result(context.Background(), task.TaskID); err != nil {
		t.Fatal(err)
	}
	if after, _ := st.Get(task.TaskID); after.Status != tasks.Completed {
		t.Errorf("cooperative work should be allowed to finish, got %q", after.Status)
	}
}

func TestCancelImmediateIsTerminalAndSticky(t *testing.T) {
	// The legacy 2025-11-25 path (#2499, Codex P1 on PR #3699): even
	// with a grace window configured, an immediate cancel records
	// Cancelled before replying, and the work finishing afterwards
	// cannot change that ending.
	st := tasks.New()
	st.CancelGrace = time.Second
	task, _ := st.Start(tasks.Caller("tester"), 0, func(ctx context.Context) (any, *tasks.Fault) {
		<-ctx.Done()
		return "too late", nil
	})
	snap, ok := st.CancelImmediate(task.TaskID, "cancelled")
	if !ok || snap.Status != tasks.Cancelled {
		t.Fatalf("immediate cancel should return the terminal snapshot, got %+v", snap)
	}
	if _, _, err := st.Result(context.Background(), task.TaskID); err != nil {
		t.Fatal(err)
	}
	if after, _ := st.Get(task.TaskID); after.Status != tasks.Cancelled {
		t.Errorf("a legacy cancellation must stand, got %q", after.Status)
	}
}

// --- Phase 1 of #2497 (#2498): task-store limits ---

func TestStartClampsTTLToTheStoreMax(t *testing.T) {
	st := tasks.New()
	st.MaxTTL = 90 * time.Second
	task, err := st.Start(tasks.Caller("alice"), 10*60*1000, func(ctx context.Context) (any, *tasks.Fault) {
		return "ok", nil
	})
	if err != nil {
		t.Fatalf("start: %v", err)
	}
	if task.TTL != 90*1000 {
		t.Errorf("a TTL above protoTasks.maxTTL is clamped on the returned task, got %d ms", task.TTL)
	}
	under, err := st.Start(tasks.Caller("alice"), 1000, func(ctx context.Context) (any, *tasks.Fault) {
		return "ok", nil
	})
	if err != nil {
		t.Fatalf("start: %v", err)
	}
	if under.TTL != 1000 {
		t.Errorf("a TTL under the max is untouched, got %d ms", under.TTL)
	}
}

func TestStartRefusesBeyondThePerRequestorCap(t *testing.T) {
	st := tasks.New()
	st.MaxPerRequestor = 2
	block := make(chan struct{})
	defer close(block)
	fn := func(ctx context.Context) (any, *tasks.Fault) {
		<-block
		return "ok", nil
	}
	for i := 0; i < 2; i++ {
		if _, err := st.Start(tasks.Caller("alice"), 60000, fn); err != nil {
			t.Fatalf("start %d: %v", i, err)
		}
	}
	_, err := st.Start(tasks.Caller("alice"), 60000, fn)
	if !errors.Is(err, tasks.ErrPerRequestorCap) {
		t.Fatalf("the 3rd non-terminal task is refused with the named sentinel, got %v", err)
	}
	if !strings.Contains(err.Error(), "maxPerRequestor") {
		t.Errorf("the refusal names the limit, got %q", err.Error())
	}
	// Another requestor is unaffected, and the unbound bucket is capped
	// the same way rather than being an unlimited escape hatch.
	if _, err := st.Start(tasks.Caller("bob"), 60000, fn); err != nil {
		t.Errorf("another requestor still starts: %v", err)
	}
	for i := 0; i < 2; i++ {
		if _, err := st.Start(tasks.Caller(""), 60000, fn); err != nil {
			t.Fatalf("unbound start %d: %v", i, err)
		}
	}
	if _, err := st.Start(tasks.Caller(""), 60000, fn); !errors.Is(err, tasks.ErrPerRequestorCap) {
		t.Errorf("the unbound bucket is capped too, got %v", err)
	}
}

func TestACompletedTaskFreesItsCapSlot(t *testing.T) {
	st := tasks.New()
	st.MaxPerRequestor = 1
	first, err := st.Start(tasks.Caller("alice"), 60000, func(ctx context.Context) (any, *tasks.Fault) {
		return "ok", nil
	})
	if err != nil {
		t.Fatalf("start: %v", err)
	}
	if _, _, err := st.Result(context.Background(), first.TaskID); err != nil {
		t.Fatalf("result: %v", err)
	}
	if _, err := st.Start(tasks.Caller("alice"), 60000, func(ctx context.Context) (any, *tasks.Fault) {
		return "ok", nil
	}); err != nil {
		t.Errorf("a terminal task no longer counts against the cap: %v", err)
	}
}

func TestNewIDIs128Bit(t *testing.T) {
	id := tasks.NewID()
	hexPart, ok := strings.CutPrefix(id, "tsk-")
	if !ok || len(hexPart) != 32 {
		t.Errorf("task ids carry 128 bits (tsk- + 32 hex chars), got %q", id)
	}
}

func TestAdmitCountsAgainstThePerMinuteWindow(t *testing.T) {
	st := tasks.New()
	st.PerMinute = 3
	for i := 0; i < 3; i++ {
		if !st.Admit(tasks.Caller("alice")) {
			t.Fatalf("call %d is admitted", i)
		}
	}
	if st.Admit(tasks.Caller("alice")) {
		t.Error("the 4th call in the minute is refused")
	}
	if !st.Admit(tasks.Caller("bob")) {
		t.Error("another requestor has its own window")
	}
	st.PerMinute = -1
	for i := 0; i < 10; i++ {
		if !st.Admit(tasks.Caller("alice")) {
			t.Fatal("protoTasks.perMinute = 0 disables the window")
		}
	}
}

func TestConcurrentStartsRespectThePerRequestorCap(t *testing.T) {
	st := tasks.New()
	st.MaxPerRequestor = 2
	block := make(chan struct{})
	defer close(block)
	fn := func(ctx context.Context) (any, *tasks.Fault) {
		<-block
		return "ok", nil
	}
	const racers = 10
	var wg sync.WaitGroup
	won := make(chan struct{}, racers)
	for i := 0; i < racers; i++ {
		wg.Add(1)
		go func() {
			defer wg.Done()
			if _, err := st.Start(tasks.Caller("alice"), 60000, fn); err == nil {
				won <- struct{}{}
			} else if !errors.Is(err, tasks.ErrPerRequestorCap) {
				t.Errorf("a burst refusal is still the named sentinel, got %v", err)
			}
		}()
	}
	wg.Wait()
	close(won)
	got := 0
	for range won {
		got++
	}
	if got != 2 {
		t.Errorf("a concurrent burst admits exactly the cap, got %d", got)
	}
}

func TestAskAndUnboundBucketsAreSeparateLedgers(t *testing.T) {
	// One logical ask operation on the embedded MCP surface creates two
	// store tasks — the /v1/ask backing (AskRequestor) and the MCP
	// wrapper (UnboundRequestor) — so at a per-requestor cap of 1 each
	// ledger must admit its own half, while still capping its own kind.
	st := tasks.New()
	st.MaxPerRequestor = 1
	block := make(chan struct{})
	defer close(block)
	fn := func(ctx context.Context) (any, *tasks.Fault) {
		<-block
		return "ok", nil
	}
	if _, err := st.Start(tasks.AskRequestor, 60000, fn); err != nil {
		t.Fatalf("ask backing start: %v", err)
	}
	if _, err := st.Start(tasks.UnboundRequestor, 60000, fn); err != nil {
		t.Fatalf("the MCP wrapper is a separate ledger and still starts at cap 1: %v", err)
	}
	if _, err := st.Start(tasks.AskRequestor, 60000, fn); !errors.Is(err, tasks.ErrPerRequestorCap) {
		t.Errorf("a second ask backing is refused, got %v", err)
	}
	if _, err := st.Start(tasks.UnboundRequestor, 60000, fn); !errors.Is(err, tasks.ErrPerRequestorCap) {
		t.Errorf("a second unbound task is refused, got %v", err)
	}
}

// Codex + CodeRabbit round (PR #2589/#2869 lineage, PR #2869): the
// requestor key is composite (role + identity). An identified ask
// backing and its MCP wrapper for the SAME identity charge different
// ledgers, so a per-requestor cap of 1 leaves room for one logical
// operation; and a caller identity that happens to spell a reserved
// bucket name keys its own bucket, never the reserved one.
func TestCompositeRequestorSeparatesRolesAndReservedNames(t *testing.T) {
	st := tasks.New()
	st.MaxPerRequestor = 1
	block := make(chan struct{})
	t.Cleanup(func() { close(block) })
	fn := func(ctx context.Context) (any, *tasks.Fault) {
		<-block
		return nil, nil
	}
	// One logical ask operation: backing (ask role) + wrapper
	// (caller role) under one client-named identity.
	if _, err := st.Start(tasks.Ask("client-x"), 60000, fn); err != nil {
		t.Fatalf("ask backing: %v", err)
	}
	if _, err := st.Start(tasks.Caller("client-x"), 60000, fn); err != nil {
		t.Fatalf("wrapper for the same identity must not be charged to the ask ledger: %v", err)
	}
	// Each ledger is now full at cap 1.
	if _, err := st.Start(tasks.Ask("client-x"), 60000, fn); !errors.Is(err, tasks.ErrPerRequestorCap) {
		t.Fatalf("second ask backing should hit the cap, got %v", err)
	}
	if _, err := st.Start(tasks.Caller("client-x"), 60000, fn); !errors.Is(err, tasks.ErrPerRequestorCap) {
		t.Fatalf("second wrapper should hit the cap, got %v", err)
	}
	// Reserved-name collisions: a session named "unbound" is an
	// identified caller with its own bucket.
	if _, err := st.Start(tasks.UnboundRequestor, 60000, fn); err != nil {
		t.Fatalf("unbound: %v", err)
	}
	if _, err := st.Start(tasks.Caller("unbound"), 60000, fn); err != nil {
		t.Fatalf("a session named unbound must not share the unbound bucket: %v", err)
	}
	if _, err := st.Start(tasks.Ask(""), 60000, fn); err != nil {
		t.Fatalf("anonymous ask: %v", err)
	}
	if _, err := st.Start(tasks.Caller("unbound-ask"), 60000, fn); err != nil {
		t.Fatalf("a session named unbound-ask must not share the ask bucket: %v", err)
	}
}

// Codex round (PR #2869): an execution wrapper and its reporting
// calls are two ledgers. A /v1/exec wrapper task (caller role) stays
// non-terminal for the whole run while the script's tool calls create
// reporting tasks (report role) under the same session identity, so
// at a per-requestor cap of 1 the wrapper must leave the report
// ledger's slot free -- and the report ledger still caps its own kind.
func TestReportAndCallerBucketsAreSeparateLedgers(t *testing.T) {
	st := tasks.New()
	st.MaxPerRequestor = 1
	block := make(chan struct{})
	defer close(block)
	fn := func(ctx context.Context) (any, *tasks.Fault) {
		<-block
		return "ok", nil
	}
	if _, err := st.Start(tasks.Caller("sess-1"), 60000, fn); err != nil {
		t.Fatalf("exec wrapper start: %v", err)
	}
	if _, err := st.Start(tasks.Report("sess-1"), 60000, fn); err != nil {
		t.Fatalf("the wrapper's first reporting call is a separate ledger and still starts at cap 1: %v", err)
	}
	if _, err := st.Start(tasks.Report("sess-1"), 60000, fn); !errors.Is(err, tasks.ErrPerRequestorCap) {
		t.Errorf("a second reporting call for the session is refused, got %v", err)
	}
	if _, err := st.Start(tasks.Caller("sess-1"), 60000, fn); !errors.Is(err, tasks.ErrPerRequestorCap) {
		t.Errorf("a second caller task for the session is refused, got %v", err)
	}
	if _, err := st.Start(tasks.Report("sess-2"), 60000, fn); err != nil {
		t.Errorf("another session's reporting call is unaffected: %v", err)
	}
}

func TestAWorkingTaskOutlivesItsTTLAndItsResultStaysCollectable(t *testing.T) {
	// Regression (#1653, Bug A): the TTL is retention from completion,
	// not a runtime deadline — a task still working when its TTL passes
	// must not be deleted, and its context must not be cancelled by the
	// retention clock.
	st := tasks.New()
	task, _ := st.Start(tasks.Caller(""), 80, func(ctx context.Context) (any, *tasks.Fault) {
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
	task, _ := st.Start(tasks.Caller(""), 400, func(ctx context.Context) (any, *tasks.Fault) {
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
	t1, _ := st1.Start(tasks.Caller(""), 400, func(ctx context.Context) (any, *tasks.Fault) {
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
	defer func() { _ = st2.Close() }()
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
	task, err := st.Start(tasks.UnboundRequestor, ttl, func(context.Context) (any, *tasks.Fault) {
		time.Sleep(runtime)
		return "done", nil
	})
	if err != nil {
		t.Fatal(err)
	}
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
	defer func() { _ = st2.Close() }()
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
	st.CancelGrace = 20 * time.Millisecond
	task, _ := st.StartWithDeadline(tasks.Caller(""), 60000, 50*time.Millisecond, func(ctx context.Context) (any, *tasks.Fault) {
		<-ctx.Done()
		// Simulate work that does not finish during the cooperative grace.
		time.Sleep(100 * time.Millisecond)
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

// A deadline task's retention is floored at two poll intervals, under
// the maxTTL clamp: the deadline seams are polled by a wrapper working
// to the advertised interval, and a clamped TTL below one poll cycle
// would delete a finished result before the wrapper's next look
// (Codex on #2869, round 6 -- the daemon-backed ask path losing a
// successful call's result to a 100ms maxTTL).
func TestADeadlineTasksResultSurvivesAPollCycleBeyondTheClamp(t *testing.T) {
	st := tasks.New()
	st.PollInterval = 400 * time.Millisecond
	st.MaxTTL = 50 * time.Millisecond
	task, err := st.StartWithDeadline(tasks.Caller("asker"), 60000, time.Minute, func(ctx context.Context) (any, *tasks.Fault) {
		return "the answer", nil
	})
	if err != nil {
		t.Fatalf("start: %v", err)
	}
	if task.TTL != 800 {
		t.Errorf("a deadline task's TTL is floored at two poll intervals, got %d ms", task.TTL)
	}
	res, fault, err := st.Result(context.Background(), task.TaskID)
	if err != nil || fault != nil || res != "the answer" {
		t.Fatalf("result: %v %v %v", res, fault, err)
	}
	// Past the 50ms clamp, inside the 800ms floor: still collectable.
	time.Sleep(200 * time.Millisecond)
	if _, ok := st.Get(task.TaskID); !ok {
		t.Error("the finished deadline task expired inside its first poll cycle")
	}
	// A plain Start carries no such floor: the clamp is its whole story.
	plain, err := st.Start(tasks.Caller("asker"), 60000, func(ctx context.Context) (any, *tasks.Fault) {
		return "ok", nil
	})
	if err != nil {
		t.Fatalf("start: %v", err)
	}
	if plain.TTL != 50 {
		t.Errorf("a plain task's TTL stays clamped, got %d ms", plain.TTL)
	}
}

func TestCancelGraceExpiryWinsOverLateResult(t *testing.T) {
	st := tasks.New()
	st.CancelGrace = 15 * time.Millisecond
	started := make(chan struct{})
	release := make(chan struct{})
	task, err := st.Start(tasks.Caller("tester"), 0, func(ctx context.Context) (any, *tasks.Fault) {
		<-ctx.Done()
		close(started)
		<-release
		return "late result", nil
	})
	if err != nil {
		t.Fatal(err)
	}
	snap, ok := st.CancelWithReason(task.TaskID, "user aborted run")
	if !ok || snap.Status != tasks.Working {
		t.Fatalf("cancel should return the pre-cancel state, got %+v", snap)
	}
	<-started
	if during, _ := st.Get(task.TaskID); during.Status != tasks.Working {
		t.Fatalf("wire status changed during the grace window: %q", during.Status)
	}
	deadline := time.Now().Add(time.Second)
	for {
		current, _ := st.Get(task.TaskID)
		if current.Status == tasks.Cancelled {
			if current.StatusMessage != "user aborted run" {
				t.Errorf("cancellation reason = %q", current.StatusMessage)
			}
			break
		}
		if time.Now().After(deadline) {
			t.Fatalf("grace expiry did not cancel the task: %+v", current)
		}
		time.Sleep(time.Millisecond)
	}
	close(release)
	if _, _, err := st.Result(context.Background(), task.TaskID); err != nil {
		t.Fatal(err)
	}
	if after, _ := st.Get(task.TaskID); after.Status != tasks.Cancelled {
		t.Errorf("late work overwrote terminal cancellation: %q", after.Status)
	}
}

func TestFaultWithinCancelGraceKeepsItsFailure(t *testing.T) {
	st := tasks.New()
	st.CancelGrace = time.Second
	task, err := st.Start(tasks.Caller("tester"), 0, func(ctx context.Context) (any, *tasks.Fault) {
		<-ctx.Done()
		return nil, &tasks.Fault{Code: -32603, Message: "work failed while stopping"}
	})
	if err != nil {
		t.Fatal(err)
	}
	if _, ok := st.CancelWithReason(task.TaskID, "caller cancelled"); !ok {
		t.Fatal("cancel lost the task")
	}
	_, fault, err := st.Result(context.Background(), task.TaskID)
	if err != nil || fault == nil || fault.Message != "work failed while stopping" {
		t.Fatalf("work fault should win inside the grace period: fault=%v err=%v", fault, err)
	}
	got, _ := st.Get(task.TaskID)
	if got.Status != tasks.Failed || got.StatusMessage != "work failed while stopping" {
		t.Errorf("got status=%q statusMessage=%q", got.Status, got.StatusMessage)
	}
}

func TestCancelGraceCompletionOrExpiryWinsAtMostOnce(t *testing.T) {
	for i := 0; i < 12; i++ {
		st := tasks.New()
		st.CancelGrace = 5 * time.Millisecond
		release := make(chan struct{})
		task, err := st.Start(tasks.Caller("tester"), 0, func(ctx context.Context) (any, *tasks.Fault) {
			<-ctx.Done()
			<-release
			return "finished", nil
		})
		if err != nil {
			t.Fatal(err)
		}
		if _, ok := st.CancelWithReason(task.TaskID, "cancel race"); !ok {
			t.Fatal("cancel lost the task")
		}
		if i%2 == 0 {
			time.Sleep(5 * time.Millisecond)
		}
		close(release)
		if _, _, err := st.Result(context.Background(), task.TaskID); err != nil {
			t.Fatal(err)
		}
		got, _ := st.Get(task.TaskID)
		if got.Status != tasks.Completed && got.Status != tasks.Cancelled {
			t.Fatalf("iteration %d has no terminal winner: %+v", i, got)
		}
	}
}

func TestIsErrorStatusMessageUsesFirstTextAndFallsBack(t *testing.T) {
	long := strings.Repeat("é", 513)
	for _, tc := range []struct {
		name    string
		content []any
		want    string
	}{
		{name: "first text block", content: []any{
			map[string]any{"type": "image"},
			map[string]any{"type": "text", "text": "first"},
			map[string]any{"type": "text", "text": "second"},
		}, want: "first"},
		{name: "rune truncation", content: []any{
			map[string]any{"type": "text", "text": long},
		}, want: strings.Repeat("é", 512)},
		{name: "fallback", content: []any{
			map[string]any{"type": "image"},
		}, want: "tool returned an error result"},
	} {
		t.Run(tc.name, func(t *testing.T) {
			st := tasks.New()
			task, err := st.Start(tasks.Caller("tester"), 0, func(context.Context) (any, *tasks.Fault) {
				return map[string]any{"isError": true, "content": tc.content}, nil
			})
			if err != nil {
				t.Fatal(err)
			}
			if _, _, err := st.Result(context.Background(), task.TaskID); err != nil {
				t.Fatal(err)
			}
			got, _ := st.Get(task.TaskID)
			if got.Status != tasks.Failed || got.StatusMessage != tc.want {
				t.Errorf("got status=%q statusMessage=%q, want failed with %q", got.Status, got.StatusMessage, tc.want)
			}
		})
	}
}

// seedLegacyTaskRow writes a pre-expires_ms terminal row directly, as an
// older daemon version would have left it: expires_ms is 0, so the load
// path must derive the deadline from the stored TTL and the store's
// configured maxAge.
func seedLegacyTaskRow(t *testing.T, dbPath, id string, completedAt time.Time, ttl time.Duration, result string) {
	t.Helper()
	db, err := sql.Open("sqlite", dbPath)
	if err != nil {
		t.Fatalf("seed open: %v", err)
	}
	defer func() { _ = db.Close() }()
	ms := completedAt.UnixMilli()
	_, err = db.Exec(`INSERT INTO tasks (task_id, status, status_message, created_at_ms, updated_at_ms, ttl_ms, last_touched_ms, expires_ms, poll_interval, result_json, fault_json)
		VALUES (?, 'completed', '', ?, ?, ?, ?, 0, 0, ?, NULL)`,
		id, ms, ms, ttl.Milliseconds(), ms, result)
	if err != nil {
		t.Fatalf("seed insert %s: %v", id, err)
	}
}

// Regression for issue #6248: startup migration must not delete a legacy
// terminal row before its stored TTL-derived deadline under the
// configured retention. Oracle: with tasks.maxAge=4h, a row completed
// 2h ago with a stored 4h TTL survives Open and its result stays
// readable, while a row already expired under that same 4h retention
// (completed 5h ago, TTL 4h) is still pruned. The control case reopens
// an identically seeded database with the built-in defaults (plain
// Open, maxAge 1h) and the 2h-old row is gone — the original loss.
func TestOpenWithRetentionPreservesLegacyRowUntilStoredTTLDeadline(t *testing.T) {
	now := time.Now()
	seed := func(dbPath string) {
		st, err := tasks.Open(dbPath)
		if err != nil {
			t.Fatalf("Open (schema) failed: %v", err)
		}
		if err := st.Close(); err != nil {
			t.Fatalf("Close: %v", err)
		}
		seedLegacyTaskRow(t, dbPath, "legacy-keep", now.Add(-2*time.Hour), 4*time.Hour, `{"data":"legacy-result"}`)
		seedLegacyTaskRow(t, dbPath, "legacy-expired", now.Add(-5*time.Hour), 4*time.Hour, `{"data":"expired-result"}`)
	}

	dbPath := t.TempDir() + "/tasks.db"
	seed(dbPath)
	st, err := tasks.OpenWithRetention(dbPath, 0, 4*time.Hour)
	if err != nil {
		t.Fatalf("OpenWithRetention failed: %v", err)
	}
	defer func() { _ = st.Close() }()
	snap, ok := st.Get("legacy-keep")
	if !ok {
		t.Fatalf("legacy-keep missing after OpenWithRetention with maxAge=4h: startup migration deleted a result inside its configured retention")
	}
	if snap.Status != tasks.Completed {
		t.Errorf("legacy-keep status = %q, want completed", snap.Status)
	}
	res, fault, err := st.Result(context.Background(), "legacy-keep")
	if err != nil || fault != nil {
		t.Errorf("legacy-keep result err=%v fault=%v", err, fault)
	}
	if m, _ := res.(map[string]any); m["data"] != "legacy-result" {
		t.Errorf("legacy-keep result data = %v", res)
	}
	if _, ok := st.Get("legacy-expired"); ok {
		t.Errorf("legacy-expired survived: rows expired under the effective retention must still be pruned")
	}

	controlPath := t.TempDir() + "/tasks.db"
	seed(controlPath)
	plain, err := tasks.Open(controlPath)
	if err != nil {
		t.Fatalf("Open (control) failed: %v", err)
	}
	defer func() { _ = plain.Close() }()
	if _, ok := plain.Get("legacy-keep"); ok {
		t.Errorf("control: legacy-keep survived plain Open with the built-in 1h maxAge; the seeded row does not reproduce the pre-fix loss")
	}
}
