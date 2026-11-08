package mcpserver

import (
	"context"
	"log/slog"
	"testing"

	"github.com/developing-today/code/pkgs/mcpx/internal/logging"
	"github.com/developing-today/code/pkgs/mcpx/internal/tasks"
)

func TestStartTaskAuditsMCPTaskCreationAndTerminal(t *testing.T) {
	store := tasks.New()
	records := make(chan logging.Record, 4)
	store.Audit = func(record logging.Record) { records <- record }
	server := New(nil, "mcpx", "test")
	server.TaskStore = store

	const payload = "private tool output"
	task, rpcErr := server.startTask(context.Background(), "connection-7", 60_000, func(context.Context) (any, *rpcError) {
		return map[string]any{"result": payload}, nil
	})
	if rpcErr != nil {
		t.Fatal(rpcErr)
	}
	if _, fault, err := store.Result(context.Background(), task.TaskID); err != nil || fault != nil {
		t.Fatalf("Result() fault=%v err=%v", fault, err)
	}
	created, terminal := <-records, <-records
	for _, record := range []logging.Record{created, terminal} {
		if record.Level != slog.LevelInfo || record.Attrs["task_id"] != task.TaskID {
			t.Errorf("MCP lifecycle record = %+v", record)
		}
		if record.Attrs["result"] == payload {
			t.Errorf("MCP lifecycle record leaked task output: %#v", record.Attrs)
		}
	}
	if created.Attrs[logging.KeyEvent] != "task.created" || created.Attrs["requestor_key"] != "connection-7" {
		t.Errorf("MCP creation record = %#v", created.Attrs)
	}
	if terminal.Attrs[logging.KeyEvent] != "task.terminal" || terminal.Attrs["status"] != tasks.Completed {
		t.Errorf("MCP terminal record = %#v", terminal.Attrs)
	}
}

func TestPrivateTaskStoreAuditsViaServerSeam(t *testing.T) {
	records := make(chan logging.Record, 4)
	server := New(nil, "mcpx", "test")
	server.TaskAudit = func(record logging.Record) { records <- record }

	task, rpcErr := server.startTask(context.Background(), "connection-9", 60_000, func(context.Context) (any, *rpcError) {
		return map[string]any{"result": "ok"}, nil
	})
	if rpcErr != nil {
		t.Fatal(rpcErr)
	}
	store := server.tasks()
	if _, fault, err := store.Result(context.Background(), task.TaskID); err != nil || fault != nil {
		t.Fatalf("Result() fault=%v err=%v", fault, err)
	}
	created, terminal := <-records, <-records
	if created.Attrs[logging.KeyEvent] != "task.created" || terminal.Attrs[logging.KeyEvent] != "task.terminal" {
		t.Errorf("private-store audit events = %q, %q; want task.created, task.terminal",
			created.Attrs[logging.KeyEvent], terminal.Attrs[logging.KeyEvent])
	}
}
