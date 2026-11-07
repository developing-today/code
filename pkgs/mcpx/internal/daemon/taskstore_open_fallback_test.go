package daemon

import (
	"bytes"
	"context"
	"log"
	"os"
	"path/filepath"
	"strings"
	"testing"

	"github.com/developing-today/code/pkgs/mcpx/internal/tasks"
)

func TestTaskStoreOpenFailureLogsAndKeepsFallbackUsable(t *testing.T) {
	stateFile := filepath.Join(t.TempDir(), "state")
	if err := os.WriteFile(stateFile, []byte("not a directory"), 0o600); err != nil {
		t.Fatal(err)
	}

	var output bytes.Buffer
	s := &Server{
		paths:  Paths{State: stateFile},
		logger: log.New(&output, "", 0),
	}
	store := s.TaskStore()
	if store == nil {
		t.Fatal("taskStore returned nil after open failure")
	}
	if s.TaskStore() != store {
		t.Fatal("taskStore did not keep the fallback instance")
	}

	line := "task store: could not open " + filepath.Join(stateFile, "tasks.db")
	if got := strings.Count(output.String(), line); got != 1 {
		t.Fatalf("open failure log count = %d, want 1; output: %q", got, output.String())
	}
	if !strings.Contains(output.String(), line+": ") {
		t.Fatalf("open error is missing from log: %q", output.String())
	}
	if !strings.Contains(output.String(), "using in-memory fallback") {
		t.Fatalf("fallback is not identified in log: %q", output.String())
	}

	task, err := store.Start(tasks.UnboundRequestor, 60_000, func(ctx context.Context) (any, *tasks.Fault) {
		<-ctx.Done()
		return nil, nil
	})
	if err != nil {
		t.Fatalf("in-memory fallback could not start a task: %v", err)
	}
	if task.TaskID == "" {
		t.Fatal("in-memory fallback returned a task without an id")
	}
	store.Cancel(task.TaskID)
}
