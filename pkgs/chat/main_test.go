package main

import (
	"context"
	"fmt"
	"sync"
	"testing"
	"time"

	tea "github.com/charmbracelet/bubbletea"
)

// fakeProg records every message sent to it. It satisfies sender without
// running a real terminal program.
type fakeProg struct {
	mu   sync.Mutex
	msgs []tea.Msg
}

func (f *fakeProg) Send(msg tea.Msg) {
	f.mu.Lock()
	defer f.mu.Unlock()
	f.msgs = append(f.msgs, msg)
}

func (f *fakeProg) count() int {
	f.mu.Lock()
	defer f.mu.Unlock()
	return len(f.msgs)
}

func waitFor(t *testing.T, what string, cond func() bool) {
	t.Helper()
	for i := 0; i < 200; i++ {
		if cond() {
			return
		}
		time.Sleep(5 * time.Millisecond)
	}
	t.Fatalf("timed out waiting for %s", what)
}

func TestSendReachesOnlyLivePrograms(t *testing.T) {
	a := &app{}
	live, dead := &fakeProg{}, &fakeProg{}

	a.mu.Lock()
	a.progs = append(a.progs, live, dead)
	a.mu.Unlock()

	a.send(chatMsg{id: "u1", text: "hello"})
	waitFor(t, "initial fan-out", func() bool {
		return live.count() == 1 && dead.count() == 1
	})

	a.remove(dead)
	if n := len(a.progs); n != 1 {
		t.Fatalf("registry has %d programs after remove, want 1", n)
	}

	a.send(chatMsg{id: "u1", text: "again"})
	waitFor(t, "second fan-out", func() bool { return live.count() == 2 })
	if got := dead.count(); got != 1 {
		t.Fatalf("removed program received %d messages, want 1", got)
	}
}

func TestRemoveUnknownIsNoOp(t *testing.T) {
	a := &app{}
	kept := &fakeProg{}
	a.progs = append(a.progs, kept)

	a.remove(&fakeProg{})
	if n := len(a.progs); n != 1 {
		t.Fatalf("registry has %d programs after removing unknown, want 1", n)
	}
}

// TestWatchSessionRemovesOnSessionEnd covers the ProgramHandler waiter
// lifecycle: the watcher is spawned before wish runs the program, so it
// must deregister off the session context — never p.Wait(), whose channel
// Run initializes only after the handler returns.
func TestWatchSessionRemovesOnSessionEnd(t *testing.T) {
	a := &app{}
	p := &fakeProg{}

	a.mu.Lock()
	a.progs = append(a.progs, p)
	a.mu.Unlock()

	ctx, cancel := context.WithCancel(context.Background())
	done := make(chan struct{})
	go func() {
		a.watchSession(ctx, p)
		close(done)
	}()

	a.mu.RLock()
	n := len(a.progs)
	a.mu.RUnlock()
	if n != 1 {
		cancel()
		t.Fatalf("registry has %d programs while session is live, want 1", n)
	}

	cancel()
	select {
	case <-done:
	case <-time.After(5 * time.Second):
		t.Fatal("watchSession did not return after session context ended")
	}

	a.mu.RLock()
	n = len(a.progs)
	a.mu.RUnlock()
	if n != 0 {
		t.Fatalf("registry has %d programs after session end, want 0", n)
	}
}
func TestRegistryConcurrentRegisterSendRemove(t *testing.T) {
	a := &app{}
	var wg sync.WaitGroup
	for i := 0; i < 16; i++ {
		wg.Add(1)
		go func(i int) {
			defer wg.Done()
			p := &fakeProg{}
			a.mu.Lock()
			a.progs = append(a.progs, p)
			a.mu.Unlock()
			for j := 0; j < 25; j++ {
				a.send(chatMsg{id: fmt.Sprint(i), text: "m"})
			}
			a.remove(p)
		}(i)
	}
	wg.Wait()

	a.mu.RLock()
	n := len(a.progs)
	a.mu.RUnlock()
	if n != 0 {
		t.Fatalf("registry has %d programs after concurrent use, want 0", n)
	}
}
