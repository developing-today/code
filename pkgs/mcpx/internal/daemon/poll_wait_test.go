package daemon

import (
	"context"
	"io"
	"log"
	"net/http"
	"net/http/httptest"
	"strings"
	"testing"
	"time"

	"github.com/developing-today/code/pkgs/mcpx/internal/settings"
	"github.com/developing-today/code/pkgs/mcpx/internal/tasks"
)

func TestLongPollWaitBounds(t *testing.T) {
	sch, err := settings.New(settings.Registry())
	if err != nil {
		t.Fatal(err)
	}
	set := settings.NewSet(sch)
	for _, key := range []string{"tasks.resultWait", "proto.askPoll"} {
		if err := set.SetRuntime(key, "80ms"); err != nil {
			t.Fatal(err)
		}
	}
	s := &Server{set: set, logger: log.New(io.Discard, "", 0), reg: &Registry{asks: newAskTable()}}
	release := make(chan struct{})
	task, err := s.taskStore().Start(tasks.UnboundRequestor, 0, func(context.Context) (any, *tasks.Fault) { <-release; return nil, nil })
	if err != nil {
		t.Fatal(err)
	}
	defer close(release)
	s.reg.asks.begin(task.TaskID, "server", "key", "session")

	for _, endpoint := range []struct {
		name    string
		handler http.HandlerFunc
		status  int
	}{
		{"task", s.handleTaskResult, http.StatusRequestTimeout},
		{"ask", s.handleAskPoll, http.StatusOK},
	} {
		for _, tc := range []struct {
			name, value      string
			minimum, maximum time.Duration
			bad              bool
		}{
			{"small", "20", 15 * time.Millisecond, 500 * time.Millisecond, false},
			{"clamped", "5000", 60 * time.Millisecond, 500 * time.Millisecond, false},
			{"default", "nonsense", 60 * time.Millisecond, 500 * time.Millisecond, false},
			{"negative", "-1", 0, 0, true},
			{"overflow", "10000000000000", 0, 0, true},
			{"parse-overflow", "999999999999999999999999", 0, 0, true},
		} {
			t.Run(endpoint.name+"/"+tc.name, func(t *testing.T) {
				req := httptest.NewRequest(http.MethodGet, "/?waitMs="+tc.value, nil)
				req.SetPathValue("id", task.TaskID)
				w := httptest.NewRecorder()
				start := time.Now()
				endpoint.handler(w, req)
				elapsed := time.Since(start)
				if tc.bad {
					if w.Code != http.StatusBadRequest || !strings.Contains(w.Body.String(), "waitMs") {
						t.Fatalf("status %d body %s", w.Code, w.Body.String())
					}
				} else if w.Code != endpoint.status || elapsed < tc.minimum || elapsed > tc.maximum {
					t.Fatalf("status %d elapsed %v; want %d and %v..%v", w.Code, elapsed, endpoint.status, tc.minimum, tc.maximum)
				}
			})
		}
	}
}
