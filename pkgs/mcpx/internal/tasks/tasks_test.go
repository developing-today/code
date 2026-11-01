package tasks_test

import (
	"context"
	"errors"
	"strings"
	"sync"
	"testing"
	"time"

	"github.com/developing-today/code/pkgs/mcpx/internal/tasks"
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
	task, _ := st.Start(tasks.Caller("tester"), 0, func(ctx context.Context) (any, *tasks.Fault) {
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

	task, _ := st.Start(tasks.Caller("tester"), 0, func(ctx context.Context) (any, *tasks.Fault) {
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
	task, _ := st.StartWithDeadline(tasks.Caller(""), 60000, 50*time.Millisecond, func(ctx context.Context) (any, *tasks.Fault) {
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
