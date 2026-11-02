package main

import (
	"fmt"
	"runtime"
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

func (f *fakeProg) messages() []tea.Msg {
	f.mu.Lock()
	defer f.mu.Unlock()
	return append([]tea.Msg(nil), f.msgs...)
}

type gatedProg struct {
	mu      sync.Mutex
	msgs    []tea.Msg
	calls   int
	started chan struct{}
	release chan struct{}
}

func newGatedProg() *gatedProg {
	return &gatedProg{
		started: make(chan struct{}),
		release: make(chan struct{}),
	}
}

func (p *gatedProg) Send(msg tea.Msg) {
	p.mu.Lock()
	p.calls++
	first := p.calls == 1
	p.mu.Unlock()
	if first {
		close(p.started)
		<-p.release
	}
	p.mu.Lock()
	p.msgs = append(p.msgs, msg)
	p.mu.Unlock()
}

func (p *gatedProg) messages() []tea.Msg {
	p.mu.Lock()
	defer p.mu.Unlock()
	return append([]tea.Msg(nil), p.msgs...)
}

type blockedProg struct {
	started chan struct{}
	release chan struct{}
	once    sync.Once
}

func newBlockedProg() *blockedProg {
	return &blockedProg{started: make(chan struct{}), release: make(chan struct{})}
}

func (p *blockedProg) Send(tea.Msg) {
	p.once.Do(func() { close(p.started) })
	<-p.release
}

type channelProg struct {
	msgs  chan tea.Msg
	mu    sync.Mutex
	calls int
}

func (p *channelProg) Send(msg tea.Msg) {
	p.mu.Lock()
	p.calls++
	p.mu.Unlock()
	p.msgs <- msg
}

func (p *channelProg) sendCount() int {
	p.mu.Lock()
	defer p.mu.Unlock()
	return p.calls
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

func registeredCount(a *app) int {
	a.mu.RLock()
	defer a.mu.RUnlock()
	return len(a.progs)
}

func TestSendReachesOnlyLivePrograms(t *testing.T) {
	a := &app{}
	live, dead := &fakeProg{}, &fakeProg{}
	a.register(live)
	a.register(dead)

	a.send(chatMsg{id: "u1", text: "hello"})
	waitFor(t, "initial fan-out", func() bool {
		return live.count() == 1 && dead.count() == 1
	})

	a.remove(dead)
	if n := registeredCount(a); n != 1 {
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
	a.register(kept)

	a.remove(&fakeProg{})
	if n := registeredCount(a); n != 1 {
		t.Fatalf("registry has %d programs after removing unknown, want 1", n)
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
			a.register(p)
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

func TestSendPreservesOrderForEveryRecipient(t *testing.T) {
	a := &app{}
	blocked := newGatedProg()
	fast := &fakeProg{}
	a.register(blocked)
	a.register(fast)

	first := chatMsg{id: "u1", text: "first"}
	second := chatMsg{id: "u2", text: "second"}
	third := chatMsg{id: "u3", text: "third"}
	a.send(first)
	select {
	case <-blocked.started:
	case <-time.After(time.Second):
		t.Fatal("timed out waiting for first recipient send to block")
	}
	a.send(second)
	a.send(third)
	waitFor(t, "fast recipient's broadcasts", func() bool { return fast.count() == 3 })
	close(blocked.release)
	waitFor(t, "blocked recipient's broadcasts", func() bool { return len(blocked.messages()) == 3 })

	want := []tea.Msg{first, second, third}
	if got := blocked.messages(); fmt.Sprint(got) != fmt.Sprint(want) {
		t.Fatalf("blocked recipient got %v, want %v", got, want)
	}
	if got := fast.messages(); fmt.Sprint(got) != fmt.Sprint(want) {
		t.Fatalf("fast recipient got %v, want %v", got, want)
	}
}

func TestStalledRecipientDoesNotSpawnGoroutinePerBroadcast(t *testing.T) {
	a := &app{}
	blocked := newBlockedProg()
	a.register(blocked)
	a.send(chatMsg{text: "first"})
	select {
	case <-blocked.started:
	case <-time.After(time.Second):
		t.Fatal("timed out waiting for stalled recipient")
	}

	baseline := runtime.NumGoroutine()
	for i := 0; i < 100; i++ {
		a.send(chatMsg{text: fmt.Sprint(i)})
	}
	if got := registeredCount(a); got != 0 {
		t.Fatalf("stalled recipient remains registered after queue overflow, got %d programs", got)
	}
	runtime.Gosched()
	if delta := runtime.NumGoroutine() - baseline; delta > 3 {
		t.Fatalf("100 broadcasts added %d goroutines for one stalled recipient, want at most 3", delta)
	}

	close(blocked.release)
}

func TestSelfSendFromUpdateIsDeliveredOnceInOrderWithoutBlocking(t *testing.T) {
	a := &app{}
	self := &channelProg{msgs: make(chan tea.Msg)}
	a.register(self)
	m := initialModel()
	m.app = a
	m.id = "self"

	for _, text := range []string{"first", "second"} {
		m.textarea.SetValue(text)
		updated := make(chan tea.Model, 1)
		go func() {
			model, _ := m.Update(tea.KeyMsg{Type: tea.KeyEnter})
			updated <- model
		}()
		select {
		case updatedModel := <-updated:
			m = updatedModel.(model)
		case <-time.After(time.Second):
			t.Fatal("Update blocked while sending a message to itself")
		}

		select {
		case msg := <-self.msgs:
			got, ok := msg.(chatMsg)
			if !ok || got.id != "self" || got.text != text {
				t.Fatalf("self received %#v, want chatMsg{%q, %q}", msg, "self", text)
			}
		case <-time.After(time.Second):
			t.Fatalf("timed out waiting for self-delivery of %q", text)
		}
	}

	// A final ordered message acts as a drain barrier: if either Update
	// broadcast was duplicated, the duplicate appears before this message.
	barrier := chatMsg{id: "self", text: "barrier"}
	a.send(barrier)
	select {
	case msg := <-self.msgs:
		if got, ok := msg.(chatMsg); !ok || got != barrier {
			t.Fatalf("self received %#v at drain barrier, want %#v", msg, barrier)
		}
	case <-time.After(time.Second):
		t.Fatal("timed out waiting for self-delivery drain barrier")
	}
	if got := self.sendCount(); got != 3 {
		t.Fatalf("self received %d Send calls for two updates and one barrier, want exactly 3", got)
	}
}
