package hooks_test

import (
	"strings"
	"testing"
	"time"

	"github.com/developing-today/code/pkgs/mcpx/internal/events"
	"github.com/developing-today/code/pkgs/mcpx/internal/hooks"
)

func TestParseEmptyDeclaresNothing(t *testing.T) {
	hs, err := hooks.Parse("", time.Second)
	if err != nil || len(hs) != 0 {
		t.Fatalf("empty: got %v, %v", hs, err)
	}
}

func TestParseCommandHookDefaults(t *testing.T) {
	hs, err := hooks.Parse(`[{"name":"page","on":"server.stopped","run":["true"]}]`, 3*time.Second)
	if err != nil {
		t.Fatal(err)
	}
	if len(hs) != 1 {
		t.Fatalf("want 1 hook, got %d", len(hs))
	}
	h := hs[0]
	if h.Executor != hooks.ExecRun || h.Phase != "observe" || h.FailureClosed {
		t.Fatalf("defaults wrong: %+v", h)
	}
	if h.Timeout != 3*time.Second {
		t.Fatalf("timeout should fall back to the daemon default, got %s", h.Timeout)
	}
}

func TestParseRejectsNonObservePhase(t *testing.T) {
	_, err := hooks.Parse(`[{"on":"call.finished","phase":"before","run":["true"]}]`, time.Second)
	if err == nil || !strings.Contains(err.Error(), "observe") {
		t.Fatalf("a blocking phase must be refused, got %v", err)
	}
}

func TestParseRejectsUnhookableKinds(t *testing.T) {
	for _, on := range []string{"call.started", "resource.watching", "hook.failed", "nope"} {
		_, err := hooks.Parse(`[{"on":"`+on+`","run":["true"]}]`, time.Second)
		if err == nil {
			t.Fatalf("on=%s should be refused", on)
		}
	}
}

func TestParseRejectsTwoActionsAndShellString(t *testing.T) {
	if _, err := hooks.Parse(`[{"on":"progress","run":["true"],"module":"x.py"}]`, time.Second); err == nil {
		t.Fatal("two actions should be refused")
	}
	if _, err := hooks.Parse(`[{"on":"progress","run":"echo hi"}]`, time.Second); err == nil {
		t.Fatal("a shell string for run should be refused: run is an argv array")
	}
}

func TestParseModuleKindsAndTimeoutOverride(t *testing.T) {
	hs, err := hooks.Parse(`[
		{"name":"py","on":["elicit"],"module":"h.py","timeoutMs":250,"failure":"closed"},
		{"name":"ts","on":"*","module":"h.ts"}
	]`, 2*time.Second)
	if err != nil {
		t.Fatal(err)
	}
	if hs[0].Executor != hooks.ExecModulePython || hs[0].Timeout != 250*time.Millisecond || !hs[0].FailureClosed {
		t.Fatalf("python hook wrong: %+v", hs[0])
	}
	if hs[1].Executor != hooks.ExecModuleJS {
		t.Fatalf("ts hook wrong: %+v", hs[1])
	}
}

func TestHookMatchesPrefixOnly(t *testing.T) {
	hs, err := hooks.Parse(`[{"on":"elicit","run":["true"],"when":{"server":"chrome"}}]`, time.Second)
	if err != nil {
		t.Fatal(err)
	}
	h := hs[0]
	if !h.Matches(events.Event{Kind: events.ElicitOpened, Server: "chrome"}) {
		t.Fatal("prefix elicit should match elicit.opened for the named server")
	}
	if h.Matches(events.Event{Kind: events.ElicitOpened, Server: "other"}) {
		t.Fatal("when.server should filter")
	}
	if h.Matches(events.Event{Kind: events.ServerStarted, Server: "chrome"}) {
		t.Fatal("elicit should not match server.started")
	}
}
