package mcpserver_test

import (
	"context"
	"encoding/json"
	"log/slog"
	"strings"
	"testing"

	"github.com/developing-today/code/pkgs/mcpx/internal/logging"
	"github.com/developing-today/code/pkgs/mcpx/internal/mcpserver"
	"github.com/developing-today/code/pkgs/mcpx/internal/tasks"
)

func TestTaskRetrievalAuditIsDebugAndMetadataOnly(t *testing.T) {
	store := tasks.New()
	records := make(chan logging.Record, 8)
	store.Audit = func(record logging.Record) { records <- record }
	server := mcpserver.New(newBackend(), "mcpx", "test")
	server.TaskStore = store

	const payload = "task result content must not appear in retrieval logs"
	task, err := store.Start(tasks.UnboundRequestor, 60_000, func(context.Context) (any, *tasks.Fault) {
		return map[string]any{"result": payload}, nil
	})
	if err != nil {
		t.Fatal(err)
	}
	if _, fault, err := store.Result(context.Background(), task.TaskID); err != nil || fault != nil {
		t.Fatalf("Result() fault=%v err=%v", fault, err)
	}
	<-records // task.created
	<-records // task.terminal

	if errCode(handle(t, server, "tasks/get", taskParams(map[string]any{"taskId": task.TaskID}))) != 0 {
		t.Fatal("tasks/get failed")
	}
	result := handle(t, server, "tasks/result", map[string]any{"taskId": task.TaskID})
	if errCode(result) != 0 || !strings.Contains(protoJSON(t, result), payload) {
		t.Fatalf("tasks/result did not return the task result: %v", result)
	}

	for _, method := range []string{"tasks/get", "tasks/result"} {
		record := <-records
		if record.Level != slog.LevelDebug || record.Attrs[logging.KeyEvent] != "task.retrieval" {
			t.Errorf("%s audit record = %+v; want debug task.retrieval", method, record)
		}
		if record.Attrs["surface"] != "mcp" || record.Attrs["method"] != method || record.Attrs["task_id"] != task.TaskID {
			t.Errorf("%s retrieval metadata = %#v", method, record.Attrs)
		}
		encoded, err := json.Marshal(record.Attrs)
		if err != nil {
			t.Fatal(err)
		}
		if strings.Contains(string(encoded), payload) {
			t.Errorf("%s retrieval audit contains task result: %s", method, encoded)
		}
	}
}
