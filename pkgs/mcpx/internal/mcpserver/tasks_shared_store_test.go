package mcpserver_test

import (
	"context"
	"path/filepath"
	"strings"
	"testing"
	"time"

	"github.com/developing-today/code/pkgs/mcpx/internal/mcpserver"
	"github.com/developing-today/code/pkgs/mcpx/internal/tasks"
)

// Issue #1555 (Phase 1 of #1554): when a store is injected, the MCP
// surface uses that store — the daemon's one durable store — instead
// of a private in-memory one. These tests pin the behaviours the
// merge must preserve: cross-surface visibility through the shared
// instance, MCP ownership/visibility unchanged, and restart
// durability for MCP-started tasks.

func sharedStoreServer(store *tasks.Store) *mcpserver.Server {
	u := &taskUpstream{fakeBackend: newBackend(), delay: 100 * time.Millisecond,
		tools: map[string][]mcpserver.Tool{"tasks": {supportTool("slow_compute", "optional")}}}
	s := taskServer(u, "tasks")
	s.TaskStore = store
	s.Timing.TaskAfter = 5 * time.Second
	s.Timing.TaskEager = 50 * time.Millisecond
	return s
}

func startSharedTask(t *testing.T, s *mcpserver.Server) string {
	t.Helper()
	r := resultOf(t, handle(t, s, "tools/call", callTool(extCaps, "slow_compute")))
	id, _ := r["taskId"].(string)
	if r["resultType"] != "task" || id == "" {
		t.Fatalf("want a CreateTaskResult, got %v", r)
	}
	return id
}

func TestInjectedStoreIsTheStoreTheMCPSurfaceUses(t *testing.T) {
	store := tasks.New()
	s := sharedStoreServer(store)

	id := startSharedTask(t, s)

	// Cross-surface, MCP -> /v1: the daemon's /v1 handlers read this
	// same instance, so the MCP-started task is in it under its id.
	if _, ok := store.Get(id); !ok {
		t.Fatalf("MCP-started task %s is not in the injected store", id)
	}
	if done := waitTask(t, s, id, terminal); done["status"] != "completed" {
		t.Fatalf("task through the injected store: %v", done)
	}

	// Cross-surface, /v1 -> MCP, with visibility preserved: a task
	// started directly in the store (the /v1 surface's way) has no
	// MCP owner, so no MCP connection may list it.
	other, _ := store.Start(tasks.Caller("v1"), 60000, func(context.Context) (any, *tasks.Fault) { return "v1", nil })
	// tasks/list is a legacy-era (2025) method: the 2026-07-28 tasks
	// extension removed it, and modern-started tasks are ownerless by
	// design, so the list check goes through the legacy handler with a
	// legacy-started task on a second server sharing this same store.
	// Legacy list requires an owner match: the listing connection sees
	// its own task and never the ownerless /v1 one.
	s2 := mcpserver.New(newBackend(), "mcpx", "test")
	s2.TaskStore = store
	s2.Handle(context.Background(), mcpserver.Request(1, "tools/call", map[string]any{
		"name": "mcpx_call", "arguments": map[string]any{"namespace": "alpha", "tool": "t"}, "task": map[string]any{},
	}))
	list := protoJSON(t, s2.Handle(context.Background(), mcpserver.Request(2, "tasks/list", nil)))
	if strings.Contains(list, other.TaskID) {
		t.Errorf("an ownerless /v1 task was listed by an MCP connection: %s", list)
	}
	if !strings.Contains(list, "tsk-") {
		t.Errorf("the connection's own legacy task is missing from its list: %s", list)
	}
}

func TestInjectedStoreKeepsMCPTasksAcrossReopen(t *testing.T) {
	path := filepath.Join(t.TempDir(), "tasks.db")
	store, err := tasks.Open(path)
	if err != nil {
		t.Fatal(err)
	}
	s := sharedStoreServer(store)

	id := startSharedTask(t, s)
	if done := waitTask(t, s, id, terminal); done["status"] != "completed" {
		t.Fatalf("task through the injected store: %v", done)
	}
	// A task still working when the store closes is the restart case.
	working, _ := store.Start(tasks.Caller("v1"), 60000, func(context.Context) (any, *tasks.Fault) {
		time.Sleep(2 * time.Second)
		return "late", nil
	})
	if err := store.Close(); err != nil {
		t.Fatal(err)
	}

	reopened, err := tasks.Open(path)
	if err != nil {
		t.Fatal(err)
	}
	defer reopened.Close()
	got, ok := reopened.Get(id)
	if !ok || got.Status != tasks.Completed {
		t.Errorf("terminal MCP task after reopen: %+v, ok=%v", got, ok)
	}
	w, ok := reopened.Get(working.TaskID)
	if !ok || w.Status != tasks.Failed ||
		w.StatusMessage != "daemon restarted before task completion" {
		t.Errorf("working task after reopen: %+v, ok=%v", w, ok)
	}
}
