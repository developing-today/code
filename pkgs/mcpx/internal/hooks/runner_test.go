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

func waitFor(t *testing.T, what string, cond func() bool) {
	t.Helper()
	deadline := time.Now().Add(5 * time.Second)
	for time.Now().Before(deadline) {
		if cond() {
			return
		}
		time.Sleep(10 * time.Millisecond)
	}
	t.Fatalf("timed out waiting for %s", what)
}

func TestDeclaredHookFiresOnMatchingKindsOnly(t *testing.T) {
	dir := t.TempDir()
	marker := filepath.Join(dir, "fired")
	hs, err := hooks.Parse(`[{"name":"rec","on":"server.stopped","run":["sh","-c","cat > `+marker+`"]}]`, 2*time.Second)
	if err != nil {
		t.Fatal(err)
	}
	bus := events.New(16)
	r := hooks.NewRunner(bus, hs, nil)
	r.Start()
	defer r.Stop()

	bus.Publish(events.Event{Kind: events.ServerStarted, Server: "x"})
	bus.Publish(events.Event{Kind: events.ServerStopped, Server: "chrome"})
	// Wait for the payload itself, not the marker file's existence:
	// `cat > marker` creates the file before it writes the payload, so
	// existence alone races the write and can read back an empty file
	// under load (the #2609 flake). Poll the real postcondition.
	waitFor(t, "hook payload to land", func() bool {
		body, err := os.ReadFile(marker)
		return err == nil &&
			strings.Contains(string(body), `"kind":"server.stopped"`)
	})
	body, err := os.ReadFile(marker)
	if err != nil {
		t.Fatal(err)
	}
	// The event arrives as JSON on stdin, with its kind and server.
	if !strings.Contains(string(body), `"kind":"server.stopped"`) ||
		!strings.Contains(string(body), `"server":"chrome"`) ||
		!strings.Contains(string(body), `"hook":"rec"`) {
		t.Fatalf("hook stdin payload wrong: %s", body)
	}
	// The stats counter is recorded asynchronously after the hook
	// process finishes; asserting it immediately races the recording
	// under load (the second #2609 flake, seen in the full-system
	// build). Poll the real postcondition.
	waitFor(t, "hook stats to record the firing", func() bool {
		st := r.Stats()
		return len(st) == 1 && st[0].Fired == 1 && st[0].Failed == 0
	})
}

func TestPythonModuleHookRuns(t *testing.T) {
	dir := t.TempDir()
	mod := filepath.Join(dir, "h.py")
	out := filepath.Join(dir, "out")
	src := "import sys, json\nd = json.load(sys.stdin)\nopen(r'" + out + "', 'w').write(d['kind'])\n"
	if err := os.WriteFile(mod, []byte(src), 0o644); err != nil {
		t.Fatal(err)
	}
	hs, err := hooks.Parse(`[{"name":"py","on":"progress","module":"`+mod+`"}]`, 5*time.Second)
	if err != nil {
		t.Fatal(err)
	}
	bus := events.New(16)
	r := hooks.NewRunner(bus, hs, nil)
	r.Start()
	defer r.Stop()
	bus.Publish(events.Event{Kind: events.Progress})
	waitFor(t, "python hook output", func() bool {
		b, err := os.ReadFile(out)
		return err == nil && string(b) == "progress"
	})
}

func TestTimeoutCutsHangingHookAndPublishProceeds(t *testing.T) {
	hs, err := hooks.Parse(`[{"name":"hang","on":"progress","run":["sleep","30"],"timeoutMs":100}]`, 2*time.Second)
	if err != nil {
		t.Fatal(err)
	}
	bus := events.New(16)
	r := hooks.NewRunner(bus, hs, nil)
	r.Start()
	defer r.Stop()
	start := time.Now()
	bus.Publish(events.Event{Kind: events.Progress})
	if elapsed := time.Since(start); elapsed > 2*time.Second {
		t.Fatalf("publishing must never wait on a hook; took %s", elapsed)
	}
	waitFor(t, "timeout to be recorded", func() bool {
		st := r.Stats()
		return len(st) == 1 && st[0].TimedOut == 1 && st[0].Failed == 1
	})
}

func TestFailingHookPublishesHookFailedAndIsCounted(t *testing.T) {
	hs, err := hooks.Parse(`[{"name":"bad","on":"progress","run":["false"]}]`, 2*time.Second)
	if err != nil {
		t.Fatal(err)
	}
	bus := events.New(16)
	sub, _ := bus.Subscribe(events.Filter{Kinds: []string{"hook.failed"}}, 0)
	defer sub.Close()
	r := hooks.NewRunner(bus, hs, nil)
	r.Start()
	defer r.Stop()
	bus.Publish(events.Event{Kind: events.Progress})
	select {
	case e := <-sub.C:
		if e.Kind != events.HookFailed || !strings.Contains(string(e.Data), `"hook":"bad"`) {
			t.Fatalf("hook.failed payload wrong: %s", e.Data)
		}
	case <-time.After(5 * time.Second):
		t.Fatal("no hook.failed event published")
	}
	waitFor(t, "failure counted", func() bool {
		st := r.Stats()
		return len(st) == 1 && st[0].Failed == 1
	})
}
