package main

import (
	"fmt"
	"strings"
	"testing"
)

func TestChatHistoryCap(t *testing.T) {
	m := initialModel()

	// Initial model starts with 0 messages
	if len(m.messages) != 0 {
		t.Fatalf("expected initial messages length 0, got %d", len(m.messages))
	}

	totalMessages := maxHistory + 50
	for i := 1; i <= totalMessages; i++ {
		text := fmt.Sprintf("message %d", i)
		updatedModel, _ := m.Update(chatMsg{
			id:   "alice",
			text: text,
		})
		m = updatedModel.(model)
	}

	// Verify messages slice is capped at maxHistory
	if len(m.messages) != maxHistory {
		t.Errorf("expected len(m.messages) == %d, got %d", maxHistory, len(m.messages))
	}

	// Oldest retained message should be totalMessages - maxHistory + 1
	oldestExpected := fmt.Sprintf("message %d", totalMessages-maxHistory+1)
	if !strings.Contains(m.messages[0], oldestExpected) {
		t.Errorf("expected oldest message to contain %q, got %q", oldestExpected, m.messages[0])
	}

	// Newest message should be totalMessages
	newestExpected := fmt.Sprintf("message %d", totalMessages)
	if !strings.Contains(m.messages[len(m.messages)-1], newestExpected) {
		t.Errorf("expected newest message to contain %q, got %q", newestExpected, m.messages[len(m.messages)-1])
	}

	// Verify the newest message is visible in viewport view
	viewportContent := m.viewport.View()
	if !strings.Contains(viewportContent, newestExpected) {
		t.Errorf("expected viewport to show newest message %q, got viewport view:\n%s", newestExpected, viewportContent)
	}

	// Verify earlier dropped message is not in retained messages
	droppedMsg := "message 1"
	if strings.Contains(m.messages[0], droppedMsg) {
		t.Errorf("expected dropped message %q to not be in m.messages[0], got %q", droppedMsg, m.messages[0])
	}
}

func BenchmarkChatMsgUpdate(b *testing.B) {
	m := initialModel()
	// Pre-fill history to maxHistory
	for i := 0; i < maxHistory; i++ {
		updatedModel, _ := m.Update(chatMsg{
			id:   "benchuser",
			text: fmt.Sprintf("setup message %d", i),
		})
		m = updatedModel.(model)
	}

	b.ResetTimer()
	b.ReportAllocs()

	msg := chatMsg{
		id:   "benchuser",
		text: "streaming bench message content",
	}

	for i := 0; i < b.N; i++ {
		updatedModel, _ := m.Update(msg)
		m = updatedModel.(model)
	}
}
