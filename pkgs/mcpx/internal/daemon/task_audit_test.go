package daemon

import (
	"context"
	"encoding/json"
	"net/http/httptest"
	"os"
	"strings"
	"testing"

	"github.com/developing-today/code/pkgs/mcpx/internal/logging"
	"github.com/developing-today/code/pkgs/mcpx/internal/settings"
	"github.com/developing-today/code/pkgs/mcpx/internal/tasks"
)

func TestV1TaskRetrievalAuditIsDebugAndMetadataOnly(t *testing.T) {
	dir := t.TempDir()
	sink, err := logging.NewFileSink(logging.FileOptions{Dir: dir, Level: "debug"})
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { _ = sink.Close() })

	schema, err := settings.New(settings.Registry())
	if err != nil {
		t.Fatal(err)
	}
	server := &Server{sink: sink, set: settings.NewSet(schema)}
	store := server.taskStore()
	const payload = "task result content must not appear in retrieval logs"
	task, err := store.Start(tasks.Caller("api-session"), 60_000, func(context.Context) (any, *tasks.Fault) {
		return map[string]any{"result": payload}, nil
	})
	if err != nil {
		t.Fatal(err)
	}
	if _, fault, err := store.Result(context.Background(), task.TaskID); err != nil || fault != nil {
		t.Fatalf("Result() fault=%v err=%v", fault, err)
	}

	getReq := httptest.NewRequest("GET", "/v1/tasks/"+task.TaskID, nil)
	getReq.SetPathValue("id", task.TaskID)
	getResp := httptest.NewRecorder()
	server.handleTaskGet(getResp, getReq)
	if getResp.Code != 200 {
		t.Fatalf("tasks/get status = %d, body = %s", getResp.Code, getResp.Body)
	}

	resultReq := httptest.NewRequest("GET", "/v1/tasks/"+task.TaskID+"/result", nil)
	resultReq.SetPathValue("id", task.TaskID)
	resultResp := httptest.NewRecorder()
	server.handleTaskResult(resultResp, resultReq)
	if resultResp.Code != 200 || !strings.Contains(resultResp.Body.String(), payload) {
		t.Fatalf("tasks/result status = %d, body = %s", resultResp.Code, resultResp.Body)
	}

	raw, err := os.ReadFile(sink.Path())
	if err != nil {
		t.Fatal(err)
	}
	if strings.Contains(string(raw), payload) {
		t.Fatalf("task result leaked into audit log: %s", raw)
	}
	var retrievals []map[string]any
	for _, line := range strings.Split(strings.TrimSpace(string(raw)), "\n") {
		var record map[string]any
		if err := json.Unmarshal([]byte(line), &record); err != nil {
			t.Fatalf("invalid audit line %q: %v", line, err)
		}
		if record[logging.KeyEvent] == "task.retrieval" {
			retrievals = append(retrievals, record)
		}
	}
	if len(retrievals) != 2 {
		t.Fatalf("got %d retrieval records, want 2: %s", len(retrievals), raw)
	}
	for i, method := range []string{"tasks/get", "tasks/result"} {
		record := retrievals[i]
		if record["level"] != "debug" || record["surface"] != "v1" || record["method"] != method || record["task_id"] != task.TaskID {
			t.Errorf("%s record = %#v", method, record)
		}
	}
}
