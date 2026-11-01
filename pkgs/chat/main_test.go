package main

import (
	"io"
	"testing"
	"time"

	tea "github.com/charmbracelet/bubbletea"
)

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
	return &app{progs: []*tea.Program{prog}}, rec.msgs
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
