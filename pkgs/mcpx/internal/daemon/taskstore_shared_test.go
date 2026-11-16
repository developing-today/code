package daemon

import (
	"context"
	"testing"
	"time"

	"github.com/developing-today/code/pkgs/mcpx/internal/settings"
	"github.com/developing-today/code/pkgs/mcpx/internal/tasks"
)

// Issue #1555: TaskStore is how internal/cli shares the daemon's one
// store with the MCP surface, so it must return the same instance the
// /v1 handlers use — never a second store.
func TestTaskStoreAccessorReturnsTheOneStore(t *testing.T) {
	s := &Server{}
	if s.TaskStore() != s.taskStore() {
		t.Fatal("TaskStore() must return the daemon's one store instance")
	}
	one, two := s.TaskStore(), s.TaskStore()
	if one != two {
		t.Fatal("TaskStore() must not create a second store")
	}
}

func TestTaskStoreUsesHotRetentionSettings(t *testing.T) {
	sch, err := settings.New(settings.Registry())
	if err != nil {
		t.Fatal(err)
	}
	set := settings.NewSet(sch)
	s := &Server{set: set}
	store := s.TaskStore()

	if err := set.SetRuntime("tasks.ttl", "250ms"); err != nil {
		t.Fatal(err)
	}
	if err := set.SetRuntime("tasks.maxAge", "50ms"); err != nil {
		t.Fatal(err)
	}
	if s.TaskStore() != store {
		t.Fatal("a hot settings update replaced the shared task store")
	}
	task, err := store.Start(tasks.UnboundRequestor, 0, func(context.Context) (any, *tasks.Fault) { return "done", nil })
	if err != nil {
		t.Fatal(err)
	}
	if task.TTL != 250 {
		t.Fatalf("new task did not use hot tasks.ttl update: TTL=%dms", task.TTL)
	}
	if _, _, err := store.Result(context.Background(), task.TaskID); err != nil {
		t.Fatal(err)
	}
	terminal, ok := store.Get(task.TaskID)
	if !ok || terminal.ExpiresAt == nil || terminal.ExpiresAt.After(terminal.LastUpdatedAt.Add(50*time.Millisecond)) {
		t.Fatalf("new task did not use hot tasks.maxAge update: %+v", terminal)
	}
}
