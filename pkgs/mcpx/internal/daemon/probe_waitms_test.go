package daemon

import (
	"context"
	"net/http"
	"net/http/httptest"
	"testing"
	"time"

	"github.com/dezren39/mcpx/internal/settings"
	"github.com/dezren39/mcpx/internal/tasks"
)

// Probe (lane A, 2026-10-29): both long-poll handlers take waitMs verbatim —
// no cap against the operator's own defaults (tasks.resultWait=2m,
// proto.askPoll=500ms), and time.Duration(ms)*time.Millisecond overflows
// for ms > ~9.22e12, wrapping negative so a huge wait returns instantly.
func probeWaitServer(t *testing.T) *Server {
	t.Helper()
	sch, err := settings.New(settings.Registry())
	if err != nil {
		t.Fatal(err)
	}
	return &Server{set: settings.NewSet(sch), reg: &Registry{asks: newAskTable()}}
}

func TestProbeWaitMsTaskResult(t *testing.T) {
	s := probeWaitServer(t)
	release := make(chan struct{})
	task := s.taskStore().Start(0, func(ctx context.Context) (any, *tasks.Fault) {
		<-release
		return "done", nil
	})
	call := func(waitMs string) (int, time.Duration) {
		rec := httptest.NewRecorder()
		req := httptest.NewRequest("GET", "/v1/tasks/"+task.TaskID+"/result?waitMs="+waitMs, nil)
		req.SetPathValue("id", task.TaskID)
		start := time.Now()
		s.handleTaskResult(rec, req)
		return rec.Code, time.Since(start)
	}

	// Control: a small waitMs is honoured — 300ms, then 408 (still running).
	code, elapsed := call("300")
	t.Logf("waitMs=300 -> code=%d elapsed=%v", code, elapsed)
	if code != http.StatusRequestTimeout || elapsed < 250*time.Millisecond || elapsed > 5*time.Second {
		t.Fatalf("control failed: code=%d elapsed=%v", code, elapsed)
	}

	// Overflow: 1e13 ms (~317 years requested) wraps the Duration negative,
	// so the handler gives up instantly instead of waiting or erroring.
	code, elapsed = call("10000000000000")
	t.Logf("waitMs=10000000000000 -> code=%d elapsed=%v", code, elapsed)
	if code != http.StatusRequestTimeout || elapsed > 2*time.Second {
		t.Fatalf("overflow case: code=%d elapsed=%v, want immediate 408", code, elapsed)
	}

	// No cap: waitMs=3600000 (1h, 30x the 2m operator default) is accepted —
	// the handler is still blocked after 2s and only returns when the
	// task finishes.
	done := make(chan int, 1)
	go func() {
		c, _ := call("3600000")
		done <- c
	}()
	select {
	case c := <-done:
		t.Fatalf("waitMs=3600000 returned early with code=%d; want it still waiting at 2s", c)
	case <-time.After(2 * time.Second):
		t.Logf("waitMs=3600000 -> still blocked after 2s (accepted, no cap)")
	}
	close(release)
	select {
	case c := <-done:
		t.Logf("after task release -> code=%d", c)
		if c != http.StatusOK {
			t.Fatalf("after release: code=%d, want 200", c)
		}
	case <-time.After(5 * time.Second):
		t.Fatal("handler did not return after the task finished")
	}
}

func TestProbeWaitMsAskPoll(t *testing.T) {
	s := probeWaitServer(t)
	release := make(chan struct{})
	defer close(release)
	task := s.taskStore().Start(0, func(ctx context.Context) (any, *tasks.Fault) {
		<-release
		return "done", nil
	})
	s.reg.asks.begin(task.TaskID, "srv", "k", "sess")
	call := func(waitMs string) (int, time.Duration) {
		rec := httptest.NewRecorder()
		req := httptest.NewRequest("GET", "/v1/ask/"+task.TaskID+"?waitMs="+waitMs, nil)
		req.SetPathValue("id", task.TaskID)
		start := time.Now()
		s.handleAskPoll(rec, req)
		return rec.Code, time.Since(start)
	}

	// Control: 300ms wait is honoured (default would be 500ms).
	code, elapsed := call("300")
	t.Logf("ask waitMs=300 -> code=%d elapsed=%v", code, elapsed)
	if elapsed < 250*time.Millisecond || elapsed > 5*time.Second {
		t.Fatalf("ask control failed: code=%d elapsed=%v", code, elapsed)
	}

	// Overflow: 1e13 ms wraps negative — returns instantly, no error.
	code, elapsed = call("10000000000000")
	t.Logf("ask waitMs=10000000000000 -> code=%d elapsed=%v", code, elapsed)
	if elapsed > 2*time.Second {
		t.Fatalf("ask overflow case: elapsed=%v, want immediate return", elapsed)
	}
}
