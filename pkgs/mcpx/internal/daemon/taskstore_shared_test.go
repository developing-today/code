package daemon

import "testing"

// Issue #1555: TaskStore is how internal/cli shares the daemon's one
// store with the MCP surface, so it must return the same instance the
// /v1 handlers use — never a second store.
func TestTaskStoreAccessorReturnsTheOneStore(t *testing.T) {
	s := &Server{}
	if s.TaskStore() != s.taskStore() {
		t.Fatal("TaskStore() must return the daemon's one store instance")
	}
	if s.TaskStore() != s.TaskStore() {
		t.Fatal("TaskStore() must not create a second store")
	}
}
