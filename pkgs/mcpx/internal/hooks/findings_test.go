package hooks_test

import (
	"os"
	"path/filepath"
	"strings"
	"testing"
	"time"

	"github.com/dezren39/mcpx/internal/events"
	"github.com/dezren39/mcpx/internal/hooks"
)

// Codex P1 on PR #2541: a hook that spawns a descendant
// (sh -c 'sleep 2 & wait') must be cut at its timeout, not held open
// by the descendant's inherited stdout/stderr pipes.
func TestDescendantHookCutAtTimeout(t *testing.T) {
	hs, err := hooks.Parse(`[{"name":"spawn","on":"progress","run":["sh","-c","sleep 2 & wait"],"timeoutMs":100}]`, 2*time.Second)
	if err != nil {
		t.Fatal(err)
	}
	bus := events.New(16)
	r := hooks.NewRunner(bus, hs, nil)
	r.Start()
	defer r.Stop()
	start := time.Now()
	bus.Publish(events.Event{Kind: events.Progress})
	waitFor(t, "descendant hook timeout to be recorded", func() bool {
		st := r.Stats()
		return len(st) == 1 && st[0].TimedOut == 1 && st[0].Failed == 1
	})
	if elapsed := time.Since(start); elapsed > 1500*time.Millisecond {
		t.Fatalf("descendant kept the hook alive past its timeout: %s", elapsed.Round(time.Millisecond))
	}
}

// Codex P2 on PR #2541: events lost while the hook's worker is busy
// must be counted in Stats.Dropped (the gap marker rides on this).
func TestSlowHookDropsAreCounted(t *testing.T) {
	hs, err := hooks.Parse(`[{"name":"slow","on":"progress","run":["sleep","1"],"timeoutMs":5000}]`, 10*time.Second)
	if err != nil {
		t.Fatal(err)
	}
	bus := events.New(256)
	r := hooks.NewRunner(bus, hs, nil)
	r.Start()
	defer r.Stop()
	bus.Publish(events.Event{Kind: events.Progress})
	time.Sleep(200 * time.Millisecond) // let the worker pick up the first event and block in sleep
	for i := 0; i < 40; i++ {
		bus.Publish(events.Event{Kind: events.Progress})
	}
	waitFor(t, "dropped events to be counted", func() bool {
		st := r.Stats()
		return len(st) == 1 && st[0].Dropped > 0
	})
}

// Codex P2 on PR #2541: hookReady (via hooks.Ready, shared with
// doctor) must not report a module executor ready when the file
// exists but its interpreter is missing from PATH.
func TestReadyReportsMissingInterpreter(t *testing.T) {
	dir := t.TempDir()
	origPath := os.Getenv("PATH")
	py := filepath.Join(dir, "h.py")
	if err := os.WriteFile(py, []byte("x=1\n"), 0o644); err != nil {
		t.Fatal(err)
	}
	js := filepath.Join(dir, "h.mjs")
	if err := os.WriteFile(js, []byte("1\n"), 0o644); err != nil {
		t.Fatal(err)
	}
	t.Setenv("PATH", t.TempDir()) // no interpreters here

	hs, err := hooks.Parse(`[
		{"name":"py","on":"progress","module":"`+py+`"},
		{"name":"js","on":"progress","module":"`+js+`"},
		{"name":"missing","on":"progress","module":"`+filepath.Join(dir, "nope.py")+`"},
		{"name":"bin","on":"progress","run":["definitely-not-a-real-binary-xyz"]}
	]`, 2*time.Second)
	if err != nil {
		t.Fatal(err)
	}
	for _, h := range hs {
		if got := hooks.Ready(h); !strings.HasPrefix(got, "no: ") {
			t.Fatalf("hook %s executor %s should be unready with an empty PATH, got %q", h.Name, h.Executor, got)
		}
	}

	// With the real PATH restored, a plain binary on PATH is ready.
	t.Setenv("PATH", origPath)
	hs, err = hooks.Parse(`[{"name":"ok","on":"progress","run":["sh","-c","true"]}]`, 2*time.Second)
	if err != nil {
		t.Fatal(err)
	}
	if got := hooks.Ready(hs[0]); got != "yes" {
		t.Fatalf("run hook on PATH should be ready, got %q", got)
	}
}
