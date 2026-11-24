package daemon

import (
	"net/http"
	"net/http/httptest"
	"path/filepath"
	"strings"
	"testing"
	"time"

	"github.com/developing-today/code/pkgs/mcpx/internal/api"
	"github.com/developing-today/code/pkgs/mcpx/internal/logstore"
)

// The #13577 shared-export boundary at the HTTP layer: synthetic markers
// standing in for denied or otherwise sensitive retained content are stored
// only in the user-owned feedback SQLite store, and the shared decision
// audit export (GET /v1/authz/decisions) must never contain them — while the
// user-owned history read still returns them (preservation, not deletion).
func TestDecisionAuditExportExcludesRetainedFeedbackMarkers(t *testing.T) {
	api.ClearDecisions()

	const deniedMarker = "SYN-13577-DENIED-9f2c4a71"
	const sensitiveMarker = "SYN-13577-SENSITIVE-7b3e1c05"

	fs, err := logstore.NewFeedbackStore(filepath.Join(t.TempDir(), "feedback.db"))
	if err != nil {
		t.Fatalf("NewFeedbackStore: %v", err)
	}
	t.Cleanup(func() { _ = fs.Close() })

	traceID := "trc-13577-boundary"
	if err := fs.RecordInteraction(logstore.InteractionRecord{
		TraceID:   traceID,
		Timestamp: time.Now(),
		Source:    "proxy",
		Input:     "user query carrying " + sensitiveMarker,
		Output:    "capped upstream response carrying " + deniedMarker,
		ToolsUsed: []string{"search"},
	}); err != nil {
		t.Fatalf("RecordInteraction: %v", err)
	}
	// A denied-shaped rating (score 0) with sensitive notes: must stay in
	// user history, must never reach the shared audit export.
	if err := fs.ProvideFeedback(logstore.FeedbackParams{
		TraceID: traceID,
		Score:   0.0,
		Target:  "retrieval",
		Notes:   "reviewer note carrying " + deniedMarker + " and " + sensitiveMarker,
	}); err != nil {
		t.Fatalf("ProvideFeedback: %v", err)
	}

	s := &Server{reg: &Registry{feedbackStore: fs}}
	mux := http.NewServeMux()
	s.route(mux, "GET /v1/authz/decisions", s.handleAuthzDecisions)
	s.route(mux, "GET /v1/interactions", s.handleInteractionsList)
	handler := s.authorize(mux)

	do := func(method, path string) *httptest.ResponseRecorder {
		rec := httptest.NewRecorder()
		handler.ServeHTTP(rec, httptest.NewRequest(method, path, nil))
		return rec
	}

	// Shared audit export: must not echo either marker, nor any feedback
	// payload field name.
	rec := do(http.MethodGet, "/v1/authz/decisions")
	if rec.Code != http.StatusOK {
		t.Fatalf("GET /v1/authz/decisions = %d, want 200", rec.Code)
	}
	exportBody := rec.Body.String()
	for _, marker := range []string{deniedMarker, sensitiveMarker} {
		if strings.Contains(exportBody, marker) {
			t.Errorf("shared audit export contains retained marker %q: %s", marker, exportBody)
		}
	}
	for _, field := range []string{`"input"`, `"output"`, `"toolsUsed"`, `"feedbackNotes"`, `"traceId"`} {
		if strings.Contains(exportBody, field) {
			t.Errorf("shared audit export carries feedback field %s: %s", field, exportBody)
		}
	}

	// User-owned history: the same markers must still be there. The fix
	// separates the stores; it does not delete user history or ratings.
	hrec := do(http.MethodGet, "/v1/interactions?q="+deniedMarker)
	if hrec.Code != http.StatusOK {
		t.Fatalf("GET /v1/interactions = %d, want 200", hrec.Code)
	}
	if !strings.Contains(hrec.Body.String(), deniedMarker) {
		t.Errorf("user history lost retained marker %q after boundary change: %s", deniedMarker, hrec.Body.String())
	}
}
