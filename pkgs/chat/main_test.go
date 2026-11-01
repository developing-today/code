package main

import (
	"io"
	"strings"
	"testing"
	"time"

	tea "github.com/charmbracelet/bubbletea"
)

// Regression test for #3273: pressing Enter on an empty or whitespace-only
// input must not broadcast a blank message into every session's history.
func TestEnterOnEmptyOrWhitespaceInputBroadcastsNothing(t *testing.T) {
	m := initialModel()
	m.id = "tester"
	a := &app{}
	m.app = a
	p := tea.NewProgram(m, tea.WithInput(nil), tea.WithOutput(io.Discard))
	a.progs = []*tea.Program{p}

	done := make(chan tea.Model, 1)
	go func() {
		fm, _ := p.Run()
		done <- fm
	}()
	// Let the program start before sending keys; Program.Send blocks
	// until the event loop consumes each message, keeping order.
	time.Sleep(200 * time.Millisecond)

	enter := tea.KeyMsg{Type: tea.KeyEnter}
	typeText := func(s string) {
		p.Send(tea.KeyMsg{Type: tea.KeyRunes, Runes: []rune(s)})
	}

	p.Send(enter)     // empty input: must broadcast nothing
	typeText("   ")   // whitespace only
	p.Send(enter)     // whitespace-only input: must broadcast nothing
	typeText("hello") // real content (value is now "   hello")
	p.Send(enter)     // must broadcast exactly that text, once
	p.Send(tea.KeyMsg{Type: tea.KeyCtrlC})

	var fm tea.Model
	select {
	case fm = <-done:
	case <-time.After(10 * time.Second):
		t.Fatal("program did not quit after Ctrl-C")
	}
	final, ok := fm.(model)
	if !ok {
		t.Fatalf("final model has type %T, want model", fm)
	}
	if len(final.messages) != 1 {
		t.Fatalf("history has %d messages %q, want exactly 1 (the real text); empty/whitespace Enter broadcast a blank message", len(final.messages), final.messages)
	}
	if !strings.Contains(final.messages[0], "hello") {
		t.Fatalf("history message %q does not contain the typed text", final.messages[0])
	}
	if !strings.Contains(final.messages[0], "   hello") {
		t.Fatalf("history message %q does not preserve the exact typed text %q", final.messages[0], "   hello")
	}
}
