package mcpserver_test

import (
	"context"
	"encoding/json"
	"sync"
	"testing"
	"time"

	"github.com/developing-today/code/pkgs/mcpx/internal/mcpserver"
)

// hangingAsker is an Asker whose call never finishes and never asks:
// Poll blocks until its context ends, the way the daemon-backed Asker
// behaves while a call is still running upstream. It records Abandon
// calls, which in production are the POST to /v1/ask/{id}/abandon that
// cancels the daemon task through the task store's Cancel (#2497).
type hangingAsker struct {
	mu        sync.Mutex
	abandoned int
	polled    chan struct{}
	once      sync.Once
}

func newHangingAsker() *hangingAsker {
	return &hangingAsker{polled: make(chan struct{})}
}

func (a *hangingAsker) Begin(context.Context, string, json.RawMessage) (string, error) {
	return "call-1", nil
}

func (a *hangingAsker) Poll(ctx context.Context, _ string, _ time.Duration) (mcpserver.Outcome, error) {
	a.once.Do(func() { close(a.polled) })
	<-ctx.Done()
	return mcpserver.Outcome{}, ctx.Err()
}

func (a *hangingAsker) Reply(context.Context, string, map[string]json.RawMessage) error {
	return nil
}

func (a *hangingAsker) Abandon(string) {
	a.mu.Lock()
	a.abandoned++
	a.mu.Unlock()
}

func (a *hangingAsker) abandons() int {
	a.mu.Lock()
	defer a.mu.Unlock()
	return a.abandoned
}

// driveAskCall starts an ask-path tools/call whose upstream call hangs,
// waits until the server is polling for it, then ends the request the
// way the test's ending dictates and returns what the request was
// released with.
func driveAskCall(t *testing.T, end func(srv *mcpserver.Server, cancel context.CancelFunc)) (*hangingAsker, error) {
	t.Helper()
	asker := newHangingAsker()
	srv := mcpserver.New(newBackend(), "mcpx", "test")
	srv.Ask = asker

	ctx, cancel := context.WithCancel(context.Background())
	released := make(chan error, 1)
	go func() {
		resp := srv.Handle(ctx, mcpserver.Request(9, "tools/call", modernCall(9, nil)))
		if resp == nil {
			released <- nil
			return
		}
		if resp.Error != nil {
			released <- resp.Error
			return
		}
		released <- nil
	}()

	select {
	case <-asker.polled:
	case <-time.After(10 * time.Second):
		t.Fatal("the ask-path call never reached its poll")
	}
	end(srv, cancel)
	select {
	case err := <-released:
		return asker, err
	case <-time.After(10 * time.Second):
		t.Fatal("the ask-path call was not released")
		return asker, nil
	}
}

// TestAskPathClientCancellationCancelsTheDaemonTask is #1864 item 2: a
// notifications/cancelled naming an in-flight ask-path request routes
// to the task cancel path, so the daemon task behind the call stops
// instead of running on, and the request is released with an error.
func TestAskPathClientCancellationCancelsTheDaemonTask(t *testing.T) {
	asker, err := driveAskCall(t, func(srv *mcpserver.Server, cancel context.CancelFunc) {
		// The notification lands first, as it does on the wire:
		// Conn.cancel records it, then cancels the request context.
		srv.Handle(context.Background(), mcpserver.Request(0, "notifications/cancelled",
			map[string]any{"requestId": 9, "reason": "user"}))
		cancel()
	})
	if err == nil {
		t.Fatal("a cancelled ask-path call must be released with an error")
	}
	if got := asker.abandons(); got != 1 {
		t.Fatalf("the daemon task must be cancelled exactly once; Abandon called %d times", got)
	}
}

// TestAskPathTransportDeathLeavesTheDaemonTaskRunning is the other
// half of the distinction: a request context that ends with no
// notifications/cancelled behind it says nothing about the work, so
// the task keeps running for whoever collects it from /v1/tasks.
func TestAskPathTransportDeathLeavesTheDaemonTaskRunning(t *testing.T) {
	asker, err := driveAskCall(t, func(_ *mcpserver.Server, cancel context.CancelFunc) {
		cancel()
	})
	if err == nil {
		t.Fatal("a request whose transport died is still released with an error")
	}
	if got := asker.abandons(); got != 0 {
		t.Fatalf("no cancellation was sent, so the task must keep running; Abandon called %d times", got)
	}
}
