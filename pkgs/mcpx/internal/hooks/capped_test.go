package hooks

import (
	"strings"
	"testing"
)

// Codex P2 on PR #2541: hook output must be retained bounded in
// daemon memory -- excess bytes are drained and discarded, not kept.
func TestCappedWriterRetainsBoundedHead(t *testing.T) {
	w := &cappedWriter{max: 16}
	data := strings.Repeat("x", 1000)
	n, err := w.Write([]byte(data))
	if err != nil || n != len(data) {
		t.Fatalf("Write must drain everything: got %d, %v", n, err)
	}
	if got := w.String(); got != strings.Repeat("x", 16) {
		t.Fatalf("capped writer kept %d bytes, want 16", len(got))
	}
	// Later writes after the cap is full are still drained, not retained.
	if _, err := w.Write([]byte("more")); err != nil {
		t.Fatal(err)
	}
	if got := w.String(); len(got) != 16 {
		t.Fatalf("capped writer grew past the cap: %d", len(got))
	}
}

func TestLimitedHookOutputUsesDaemonBound(t *testing.T) {
	w := limitedHookOutput()
	if w.max <= 0 {
		t.Fatalf("hook output bound must be positive, got %d", w.max)
	}
}
