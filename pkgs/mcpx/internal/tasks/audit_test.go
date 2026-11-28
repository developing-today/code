package tasks_test

import (
	"context"
	"encoding/json"
	"log/slog"
	"strings"
	"testing"
	"time"

	"github.com/developing-today/code/pkgs/mcpx/internal/logging"
	"github.com/developing-today/code/pkgs/mcpx/internal/tasks"
)

func TestTaskAuditLogsCreationAndTerminalMetadataOnly(t *testing.T) {
	store := tasks.New()
	store.MaxTTL = 500 * time.Millisecond
	records := make(chan logging.Record, 8)
	store.Audit = func(record logging.Record) { records <- record }

	const payload = "do not write this task result to the audit log"
	task, err := store.Start(tasks.Caller("session-17"), 5000, func(context.Context) (any, *tasks.Fault) {
		return map[string]any{"result": payload}, nil
	})
	if err != nil {
		t.Fatal(err)
	}
	if _, fault, err := store.Result(context.Background(), task.TaskID); err != nil || fault != nil {
		t.Fatalf("Result() fault=%v err=%v", fault, err)
	}

	created, terminal := <-records, <-records
	if created.Level != slog.LevelInfo || created.Attrs[logging.KeyEvent] != "task.created" {
		t.Errorf("creation record = %+v; want info task.created", created)
	}
	if created.Attrs["task_id"] != task.TaskID || created.Attrs["requestor_key"] != "session-17" {
		t.Errorf("creation metadata = %#v", created.Attrs)
	}
	if created.Attrs["effective_ttl_ms"] != int64(500) || created.Attrs["ttl_clamped"] != true {
		t.Errorf("creation TTL metadata = %#v", created.Attrs)
	}
	if terminal.Level != slog.LevelInfo || terminal.Attrs[logging.KeyEvent] != "task.terminal" {
		t.Errorf("terminal record = %+v; want info task.terminal", terminal)
	}
	if terminal.Attrs["task_id"] != task.TaskID || terminal.Attrs["status"] != tasks.Completed {
		t.Errorf("terminal metadata = %#v", terminal.Attrs)
	}
	if _, ok := terminal.Attrs["duration_ms"]; !ok {
		t.Errorf("terminal record has no duration: %#v", terminal.Attrs)
	}
	assertAuditDoesNotContain(t, payload, created, terminal)
}

func TestTaskAuditLogsLimitRefusalsWithRequestorAndLimit(t *testing.T) {
	store := tasks.New()
	store.PerMinute = 1
	records := make(chan logging.Record, 8)
	store.Audit = func(record logging.Record) { records <- record }
	requestor := tasks.Caller("session-23")
	if !store.Admit(requestor) || store.Admit(requestor) {
		t.Fatal("expected the second per-minute admission to be refused")
	}
	rateRefusal := <-records
	if rateRefusal.Level != slog.LevelInfo || rateRefusal.Attrs[logging.KeyEvent] != "task.limit_refused" {
		t.Errorf("rate-limit record = %+v", rateRefusal)
	}
	if rateRefusal.Attrs["limit"] != "per_minute" || rateRefusal.Attrs["limit_value"] != 1 || rateRefusal.Attrs["requestor_key"] != "session-23" {
		t.Errorf("rate-limit metadata = %#v", rateRefusal.Attrs)
	}

	store.MaxPerRequestor = 1
	blocked := make(chan struct{})
	task, err := store.Start(requestor, 5000, func(ctx context.Context) (any, *tasks.Fault) {
		<-blocked
		return nil, nil
	})
	if err != nil {
		t.Fatal(err)
	}
	<-records // task.created
	if _, err := store.Start(requestor, 5000, func(context.Context) (any, *tasks.Fault) { return nil, nil }); err == nil {
		t.Fatal("expected per-requestor task cap refusal")
	}
	capRefusal := <-records
	if capRefusal.Level != slog.LevelInfo || capRefusal.Attrs[logging.KeyEvent] != "task.limit_refused" {
		t.Errorf("cap-limit record = %+v", capRefusal)
	}
	if capRefusal.Attrs["limit"] != "max_per_requestor" || capRefusal.Attrs["limit_value"] != 1 || capRefusal.Attrs["requestor_key"] != "session-23" {
		t.Errorf("cap-limit metadata = %#v", capRefusal.Attrs)
	}
	close(blocked)
	if _, ok := store.CancelImmediate(task.TaskID, "test cleanup"); !ok {
		t.Errorf("cleanup task %s disappeared", task.TaskID)
	}
}

func assertAuditDoesNotContain(t *testing.T, secret string, records ...logging.Record) {
	t.Helper()
	for _, record := range records {
		encoded, err := json.Marshal(record.Attrs)
		if err != nil {
			t.Fatal(err)
		}
		if strings.Contains(string(encoded), secret) {
			t.Errorf("audit metadata contains task content %q: %s", secret, encoded)
		}
	}
}
