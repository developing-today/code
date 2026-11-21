package daemon

import (
	"encoding/json"
	"net/http/httptest"
	"strings"
	"testing"
	"time"

	"github.com/developing-today/code/pkgs/mcpx/internal/execsvc"
)

func TestCallHistoryHandlersDistinguishMissingEmptyAndCollected(t *testing.T) {
	s := &Server{calls: execsvc.NewCallRegistry()}
	request := func(method, runID string) *httptest.ResponseRecorder {
		req := httptest.NewRequest(method, "/v1/calls/"+runID, nil)
		req.SetPathValue("run", runID)
		rec := httptest.NewRecorder()
		if method == "POST" {
			s.handleCallHistoryBegin(rec, req)
		} else {
			s.handleCallHistorySnapshot(rec, req)
		}
		return rec
	}
	if got := request("GET", "missing"); got.Code != 404 {
		t.Fatalf("missing snapshot status = %d, want 404", got.Code)
	}
	if got := request("POST", "empty"); got.Code != 200 {
		t.Fatalf("begin status = %d, want 200", got.Code)
	}
	var empty struct {
		Available bool               `json:"available"`
		Calls     []execsvc.CallInfo `json:"calls"`
	}
	if err := json.Unmarshal(request("GET", "empty").Body.Bytes(), &empty); err != nil {
		t.Fatal(err)
	}
	if !empty.Available || empty.Calls == nil || len(empty.Calls) != 0 {
		t.Fatalf("empty history response = %+v, want available empty array", empty)
	}

	request("POST", "collected")
	args := map[string]any{"secret": "not-retained"}
	s.callRegistry().ReportCall("collected", "srv", "tool", args, time.Millisecond, nil, nil)
	rec := request("GET", "collected")
	if rec.Code != 200 {
		t.Fatalf("collected snapshot status = %d, body %s", rec.Code, rec.Body)
	}
	if strings.Contains(rec.Body.String(), "not-retained") || !strings.Contains(rec.Body.String(), execsvc.ArgsHash(args)) {
		t.Fatalf("snapshot must return only hashed arguments: %s", rec.Body)
	}
}
