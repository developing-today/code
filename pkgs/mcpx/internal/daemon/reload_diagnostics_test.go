package daemon

import (
	"fmt"
	"strings"
	"sync"
	"testing"
	"time"

	"github.com/developing-today/code/pkgs/mcpx/internal/config"
	"github.com/developing-today/code/pkgs/mcpx/internal/testsupport"
)

// TestReloadEmitsStartFinishAndDrainDiagnostics is the measurement harness
// for #16312: the e2e `servers add` hang had no daemon-side evidence in any
// log, so the reload path went quiet at exactly the moment it needed to be
// loud. Reload must emit start/finish timing records plus per-orphan drain
// progress through the registry log function, so the next hang on the
// add/reload path is greppable in the daemon log.
func TestReloadEmitsStartFinishAndDrainDiagnostics(t *testing.T) {
	fake := testsupport.FakeMCPBinary(t)
	dir := t.TempDir()

	var mu sync.Mutex
	var lines []string
	logf := func(format string, args ...any) {
		mu.Lock()
		defer mu.Unlock()
		lines = append(lines, fmt.Sprintf(format, args...))
	}
	snapshot := func() []string {
		mu.Lock()
		defer mu.Unlock()
		return append([]string(nil), lines...)
	}
	joined := func() string { return strings.Join(snapshot(), "\n") }

	initial := &config.Config{MCPServers: map[string]*config.Server{
		"keep": {Name: "keep", Command: fake},
		"gone": {Name: "gone", Command: fake, Args: []string{"--gone"}},
	}}
	r, err := NewRegistry(initial, Paths{State: dir, Cache: dir}, logf)
	if err != nil {
		t.Fatal(err)
	}
	defer r.Close()

	next := &config.Config{MCPServers: map[string]*config.Server{
		"keep": {Name: "keep", Command: fake},
		"new":  {Name: "new", Command: fake, Args: []string{"--new"}},
	}}
	added, removed, err := r.Reload(next)
	if err != nil {
		t.Fatalf("Reload: %v", err)
	}
	if len(added) != 1 || added[0] != "new" {
		t.Fatalf("added = %v, want [new]", added)
	}
	if len(removed) != 1 || removed[0] != "gone" {
		t.Fatalf("removed = %v, want [gone]", removed)
	}

	// The synchronous swap records its own boundaries: without these, a
	// hang inside Reload is indistinguishable from a hang before it.
	if got := joined(); !strings.Contains(got, "reload start:") {
		t.Errorf("no reload-start diagnostic in log:\n%s", got)
	}
	if got := joined(); !strings.Contains(got, "reload done:") ||
		!strings.Contains(got, "new") || !strings.Contains(got, "gone") {
		t.Errorf("no reload-done diagnostic naming added/removed in log:\n%s", got)
	}

	// Orphan drain and session-record discard run on a background goroutine;
	// poll rather than sleep, with a deadline that fails loudly instead of
	// hanging the suite the way the e2e hang hung CI.
	deadline := time.Now().Add(10 * time.Second)
	for {
		got := joined()
		waiting := strings.Contains(got, `orphaned pool "gone"`)
		discarded := strings.Contains(got, "session records discarded")
		if waiting && discarded {
			break
		}
		if time.Now().After(deadline) {
			t.Fatalf("orphan-drain diagnostics missing after 10s (waiting=%v discarded=%v):\n%s", waiting, discarded, got)
		}
		time.Sleep(20 * time.Millisecond)
	}
}
