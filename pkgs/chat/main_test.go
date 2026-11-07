package main

import (
	"context"
	"errors"
	"fmt"
	"io"
	"os"
	"sync"
	"testing"
	"time"

	tea "github.com/charmbracelet/bubbletea"
	"github.com/charmbracelet/ssh"
)

func TestRunServerTreatsShutdownAsCleanStop(t *testing.T) {
	signals := make(chan os.Signal, 1)
	signals <- os.Interrupt

	serveRelease := make(chan struct{})
	shutdownCalled := false
	err := runServer(
		func() error {
			<-serveRelease
			return ssh.ErrServerClosed
		},
		func(context.Context) error {
			shutdownCalled = true
			close(serveRelease)
			return nil
		},
		signals,
		time.Second,
	)

	if err != nil {
		t.Fatalf("runServer() error = %v, want nil", err)
	}
	if !shutdownCalled {
		t.Fatal("runServer() did not shut down after receiving a signal")
	}
}

func TestRunServerReturnsServeFailure(t *testing.T) {
	serveErr := errors.New("listen tcp 127.0.0.1:23234: bind: address already in use")
	shutdownCalled := false
	err := runServer(
		func() error { return serveErr },
		func(context.Context) error {
			shutdownCalled = true
			return nil
		},
		make(chan os.Signal),
		time.Second,
	)

	if !errors.Is(err, serveErr) {
		t.Fatalf("runServer() error = %v, want %v", err, serveErr)
	}
	if shutdownCalled {
		t.Fatal("runServer() called shutdown after a serve failure")
	}
}

// recordingModel captures every message a tea.Program receives.
type recordingModel struct {
	msgs chan tea.Msg
}

func (r *recordingModel) Init() tea.Cmd { return nil }

func (r *recordingModel) Update(msg tea.Msg) (tea.Model, tea.Cmd) {
	r.msgs <- msg
	return r, nil
}

func (r *recordingModel) View() string { return "" }

// recordingApp returns an app whose only program records what send()
// broadcasts, plus the channel to observe it on.
func recordingApp(t *testing.T) (*app, chan tea.Msg) {
	t.Helper()
	rec := &recordingModel{msgs: make(chan tea.Msg, 16)}
	// A pipe that never delivers input keeps the program alive for the
	// test's duration; the default stdin would EOF and kill it at once.
	pipeReader, pipeWriter := io.Pipe()
	prog := tea.NewProgram(rec, tea.WithoutRenderer(), tea.WithInput(pipeReader))
	go func() {
		_, _ = prog.Run()
	}()
	t.Cleanup(func() {
		prog.Quit()
		_ = pipeWriter.Close()
	})
	return &app{progs: []sender{prog}}, rec.msgs
}

func enterKey() tea.KeyMsg {
	return tea.KeyMsg{Type: tea.KeyEnter}
}

// drainChatMsgs collects broadcast chatMsgs for a short window.
func drainChatMsgs(msgs chan tea.Msg, window time.Duration) []chatMsg {
	var out []chatMsg
	deadline := time.After(window)
	for {
		select {
		case msg := <-msgs:
			if cm, ok := msg.(chatMsg); ok {
				out = append(out, cm)
			}
		case <-deadline:
			return out
		}
	}
}

// Regression test for #3273: pressing Enter on an empty or
// whitespace-only input must not broadcast anything.
func TestEnterOnEmptyInputBroadcastsNothing(t *testing.T) {
	for _, value := range []string{"", "   ", "\t "} {
		a, msgs := recordingApp(t)
		m := initialModel()
		m.app = a
		m.id = "tester"
		m.textarea.SetValue(value)
		before := m.textarea.Value() // the textarea sanitizes tabs on set

		updated, _ := m.Update(enterKey())
		got := updated.(model)

		if sent := drainChatMsgs(msgs, 200*time.Millisecond); len(sent) != 0 {
			t.Errorf("value %q: broadcast %v, want nothing", value, sent)
		}
		if got.textarea.Value() != before {
			t.Errorf("value %q: textarea changed to %q, want unchanged %q", value, got.textarea.Value(), before)
		}
		if len(got.messages) != 0 {
			t.Errorf("value %q: history gained %v, want none", value, got.messages)
		}
	}
}

// Control: real text — even with surrounding spaces — still broadcasts
// exactly that text, once, and clears the textarea.
func TestEnterOnRealTextBroadcastsVerbatimOnce(t *testing.T) {
	a, msgs := recordingApp(t)
	m := initialModel()
	m.app = a
	m.id = "tester"
	m.textarea.SetValue("  hello  ")

	updated, _ := m.Update(enterKey())
	got := updated.(model)

	sent := drainChatMsgs(msgs, 500*time.Millisecond)
	if len(sent) != 1 {
		t.Fatalf("broadcast %d messages, want exactly 1: %v", len(sent), sent)
	}
	if sent[0].text != "  hello  " || sent[0].id != "tester" {
		t.Errorf("broadcast = %+v, want id=tester text=%q", sent[0], "  hello  ")
	}
	if got.textarea.Value() != "" {
		t.Errorf("textarea = %q after send, want reset", got.textarea.Value())
	}
}

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
