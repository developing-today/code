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

// Codex P2 on this PR: a background descendant started by a hook
// that has already exited (sh -c '(sleep 1; printf completed) &')
// must be allowed its configured budget. The shell exits immediately,
// but the descendant holds the pipes until the second completes; a
// WaitDelay that fires before the budget closes the pipes and records
// a bogus failure.
func TestEarlyParentExitKeepsBudget(t *testing.T) {
	hs, err := hooks.Parse(`[{"name":"early","on":"progress","run":["sh","-c","(sleep 1; printf completed) &"],"timeoutMs":5000}]`, 15*time.Second)
	if err != nil {
		t.Fatal(err)
	}
	dir := t.TempDir()
	out := filepath.Join(dir, "out")
	bus := events.New(16)
	r := hooks.NewRunner(bus, hs, nil)
	r.Start()
	defer r.Stop()
	bus.Publish(events.Event{Kind: events.ServerLog, Server: "early"})
	bus.Publish(events.Event{Kind: events.ServerLog, Server: "early"})
	bus.Publish(events.Event{Kind: events.Progress})
	waitFor(t, "early-parent hook to succeed within budget", func() bool {
		st := r.Stats()
		return len(st) == 1 && st[0].Fired == 1 && st[0].Failed == 0
	})
	_ = out
}

// Codex P1/P2: mixed filters must not inherit drops from a bus
// overflow the hook never saw. The progress hook and the
// server-log hook share nothing; not one event on one kind may
// charge a drop on the hook subscribed to the other.
func TestMixedFiltersDoNotShareDrops(t *testing.T) {
	hs, err := hooks.Parse(`[
		{"name":"progress","on":["progress"],"run":["sleep","5"],"timeoutMs":30000},
		{"name":"log","on":["server.log"],"run":["true"]}
	]`, 35*time.Second)
	if err != nil {
		t.Fatal(err)
	}
	bus := events.New(16)
	r := hooks.NewRunner(bus, hs, nil)
	r.Start()
	defer r.Stop()
	// Block the progress worker so its subscription overflows: the
	// first event is picked up and sleeps, the rest pile up behind it.
	bus.Publish(events.Event{Kind: events.Progress, Server: "p"})
	time.Sleep(200 * time.Millisecond)
	for i := 0; i < 300; i++ {
		bus.Publish(events.Event{Kind: events.Progress, Server: "p"})
	}
	waitFor(t, "progress drops to be counted", func() bool {
		st := r.Stats()
		var p, l uint64
		for _, s := range st {
			switch s.Name {
			case "progress":
				p = s.Dropped
			case "log":
				l = s.Dropped
			}
		}
		if p == 0 {
			return false
		}
		if l != 0 {
			t.Fatalf("server.log hook charged %d drops for progress events it never subscribed to", l)
		}
		if p < 43 || p > 47 {
			t.Fatalf("progress drops should be 43..47 (301 events, 256 buffer), got %d", p)
		}
		return true
	})
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
	for i := 0; i < 300; i++ {
		bus.Publish(events.Event{Kind: events.Progress})
	}
	waitFor(t, "dropped events to be counted", func() bool {
		st := r.Stats()
		if len(st) != 1 || st[0].Dropped == 0 {
			return false
		}
		if got := r.Stats()[0].Dropped; got < 44 || got > 46 {
			t.Fatalf("dropped events should be 44..46 (301 events, 256 buffer), got %d", got)
		}
		return true
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
