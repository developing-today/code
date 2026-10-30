package execsvc_test

import (
	"context"
	"errors"
	"os"
	"path/filepath"
	"strings"
	"sync"
	"testing"
	"time"

	"github.com/dezren39/mcpx/internal/execsvc"
	"github.com/dezren39/mcpx/internal/runner"
)

func TestCallRegistryReportsOnlyToTheRegisteredRun(t *testing.T) {
	reg := execsvc.NewCallRegistry()
	if reg.ReportCall("nobody", "srv", "tool", nil, time.Millisecond, nil, nil) {
		t.Fatal("a call for a run with no stream must not be delivered")
	}
	var got []execsvc.Frame
	unregister := reg.Register("run-1", func(f execsvc.Frame) { got = append(got, f) })
	destructive := true
	if !reg.ReportCall("run-1", "srv", "tool", map[string]any{"x": 1}, 12*time.Millisecond, nil, &destructive) {
		t.Fatal("the registered run should receive its call")
	}
	if len(got) != 1 || got[0].Type != execsvc.FrameCall || got[0].RunID != "run-1" {
		t.Fatalf("frames = %+v", got)
	}
	c := got[0].Call
	if c == nil || c.Server != "srv" || c.Tool != "tool" || !c.OK || c.Error != "" ||
		c.DurationMs != 12 || c.DestructiveHint == nil || !*c.DestructiveHint {
		t.Fatalf("Call = %+v", c)
	}
	if want := execsvc.ArgsHash(map[string]any{"x": 1}); c.ArgsHash != want {
		t.Fatalf("ArgsHash = %q, want %q", c.ArgsHash, want)
	}
	if !reg.ReportCall("run-1", "srv", "tool", nil, time.Millisecond, errors.New("boom"), nil) {
		t.Fatal("a failed call should still be reported")
	}
	if len(got) != 2 || got[1].Call.OK || got[1].Call.Error != "boom" {
		t.Fatalf("failure frame = %+v", got[1])
	}
	unregister()
	if reg.ReportCall("run-1", "srv", "tool", nil, time.Millisecond, nil, nil) {
		t.Fatal("an unregistered run must stop receiving calls")
	}
}

// Unregistering must drain a report that is already being delivered:
// the frame lands, unregister returns only after it does, and later
// reports for the run are dropped. Without the drain, a call finishing
// as its run ends could be written after the run's terminal frames —
// or after the stream's handler has returned.
func TestCallRegistryUnregisterDrainsInFlightReports(t *testing.T) {
	reg := execsvc.NewCallRegistry()
	entered := make(chan struct{})
	release := make(chan struct{})
	var once sync.Once
	unregister := reg.Register("run-1", func(f execsvc.Frame) {
		once.Do(func() { close(entered) })
		<-release
	})
	reported := make(chan bool, 1)
	go func() {
		reported <- reg.ReportCall("run-1", "srv", "tool", nil, time.Millisecond, nil, nil)
	}()
	<-entered
	unregistered := make(chan struct{})
	go func() {
		unregister()
		close(unregistered)
	}()
	select {
	case <-unregistered:
		t.Fatal("unregister returned while a report was still in flight")
	case <-time.After(100 * time.Millisecond):
	}
	close(release)
	if ok := <-reported; !ok {
		t.Fatal("the in-flight report should be delivered, not dropped")
	}
	<-unregistered
	if reg.ReportCall("run-1", "srv", "tool", nil, time.Millisecond, nil, nil) {
		t.Fatal("a report after unregister must be dropped")
	}
	// Unregister is idempotent: RunWith defers it after calling it
	// explicitly before the terminal frames.
	unregister()
}

// documentedGroups reads the frame-order block in docs/exec.md and
// returns its groups in order: one entry per line of the block, each
// the set of frame kinds that line allows.
func documentedGroups(t *testing.T) [][]string {
	t.Helper()
	b, err := os.ReadFile(filepath.Join("..", "..", "docs", "exec.md"))
	if err != nil {
		t.Fatal(err)
	}
	var groups [][]string
	inBlock := false
	for _, line := range strings.Split(string(b), "\n") {
		if strings.HasPrefix(line, "start{runId}") {
			inBlock = true
		}
		if !inBlock {
			continue
		}
		if strings.HasPrefix(line, "----") {
			break
		}
		var kinds []string
		for _, tok := range strings.FieldsFunc(line, func(r rune) bool {
			return r == ' ' || r == '|' || r == '{'
		}) {
			switch tok {
			case "start", "log", "emit", "stdout", "call", "artifact", "result", "error", "end":
				kinds = append(kinds, tok)
			}
		}
		if len(kinds) > 0 {
			groups = append(groups, kinds)
		}
		if strings.HasPrefix(line, "end{") {
			break
		}
	}
	return groups
}

func TestCallFramesMatchTheDocumentedOrder(t *testing.T) {
	groups := documentedGroups(t)
	if len(groups) != 5 {
		t.Fatalf("documented groups = %v, want 5 (start, interleaved, artifact, result|error, end)", groups)
	}
	// The call kind must be documented in the interleaved group: that is
	// the promise this issue adds to the doc, ahead of the code.
	interleaved := groups[1]
	found := false
	for _, k := range interleaved {
		if k == execsvc.FrameCall {
			found = true
		}
	}
	if !found {
		t.Fatalf("docs/exec.md does not place call in the interleaved group: %v", interleaved)
	}

	svc := service(t, store(t))
	svc.Calls = execsvc.NewCallRegistry()

	var mu sync.Mutex
	var frames []execsvc.Frame
	reported := false
	sink := func(f execsvc.Frame) error {
		mu.Lock()
		frames = append(frames, f)
		first := f.Type == execsvc.FrameStart && !reported
		if first {
			reported = true
		}
		runID := f.RunID
		mu.Unlock()
		if first {
			go func() {
				time.Sleep(100 * time.Millisecond)
				destructive := true
				svc.Calls.ReportCall(runID, "srv", "tool",
					map[string]any{"x": 1}, 12*time.Millisecond, nil, &destructive)
			}()
		}
		return nil
	}
	res, err := svc.RunWith(context.Background(), runner.Options{
		Source:       "const until = Date.now() + 600; while (Date.now() < until) {}\nconsole.log('done');\n",
		ClientSource: minimalClient,
		Permissions:  "all",
	}, execsvc.Options{}, sink)
	if err != nil {
		t.Fatalf("run: %v (%+v)", err, res)
	}

	// Every emitted frame must belong to a documented group no earlier
	// than the previous frame's group: the emitted order refines the
	// documented one, never contradicts it.
	groupOf := func(kind string) int {
		for i, g := range groups {
			for _, k := range g {
				if k == kind {
					return i
				}
			}
		}
		return -1
	}
	mu.Lock()
	defer mu.Unlock()
	last := 0
	sawCall := false
	for _, f := range frames {
		g := groupOf(f.Type)
		if g < 0 {
			t.Fatalf("emitted frame kind %q is not in the documented order %v", f.Type, groups)
		}
		if g < last {
			t.Fatalf("emitted order contradicts the doc: %q in group %d after group %d (%v)", f.Type, g, last, frames)
		}
		last = g
		if f.Type == execsvc.FrameCall {
			sawCall = true
			if f.Call == nil || f.Call.Server != "srv" || f.Call.Tool != "tool" || !f.Call.OK {
				t.Fatalf("call frame = %+v", f)
			}
		}
	}
	if !sawCall {
		t.Fatalf("no call frame was emitted: %v", frames)
	}
	if frames[0].Type != execsvc.FrameStart {
		t.Fatalf("first frame = %q, want start", frames[0].Type)
	}
}
