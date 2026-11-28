package daemon

import (
	"encoding/json"
	"fmt"
	"net/http"
	"net/http/httptest"
	"path/filepath"
	"strings"
	"testing"
	"time"

	"github.com/developing-today/code/pkgs/mcpx/internal/logstore"
)

// setupInteractionsListServer builds a Server backed by a real feedback
// store seeded with three interactions scored 0.2 / 0.5 / 0.9 (#2223).
func setupInteractionsListServer(t *testing.T) *Server {
	t.Helper()
	fs, err := logstore.NewFeedbackStore(filepath.Join(t.TempDir(), "feedback.db"))
	if err != nil {
		t.Fatalf("NewFeedbackStore: %v", err)
	}
	t.Cleanup(func() { _ = fs.Close() })
	for i, score := range []float64{0.2, 0.5, 0.9} {
		traceID := fmt.Sprintf("trc-2223-%d", i)
		rec := logstore.InteractionRecord{
			TraceID:   traceID,
			Timestamp: time.Now(),
			Source:    "proxy",
			Input:     "find bad responses",
			Output:    "an answer",
		}
		if err := fs.RecordInteraction(rec); err != nil {
			t.Fatalf("RecordInteraction: %v", err)
		}
		if err := fs.ProvideFeedback(logstore.FeedbackParams{TraceID: traceID, Score: score}); err != nil {
			t.Fatalf("ProvideFeedback: %v", err)
		}
	}
	return &Server{reg: &Registry{feedbackStore: fs}}
}

func getInteractions(t *testing.T, s *Server, query string) *httptest.ResponseRecorder {
	t.Helper()
	req := httptest.NewRequest(http.MethodGet, "/v1/interactions?"+query, nil)
	rec := httptest.NewRecorder()
	s.handleInteractionsList(rec, req)
	return rec
}

// TestInteractionsListRejectsInvalidScoreBounds pins #2223: a non-numeric,
// NaN, infinite, or out-of-range minScore/maxScore must be a 400 naming the
// parameter — never a silently unfiltered (or silently empty) 200.
func TestInteractionsListRejectsInvalidScoreBounds(t *testing.T) {
	s := setupInteractionsListServer(t)
	for _, param := range []string{"minScore", "maxScore"} {
		for _, bad := range []string{"abc", "NaN", "Inf", "+Inf", "-Inf", "1e999"} {
			rec := getInteractions(t, s, param+"="+bad)
			if rec.Code != http.StatusBadRequest {
				t.Errorf("%s=%s: status = %d, want 400 (body %s)", param, bad, rec.Code, rec.Body.String())
				continue
			}
			if !strings.Contains(rec.Body.String(), param) {
				t.Errorf("%s=%s: 400 body %q does not name the parameter", param, bad, rec.Body.String())
			}
		}
	}
}

// TestInteractionsListRejectsInvalidLimitOffset pins the same 400 rule for
// this handler's limit/offset, which had the same silent-drop shape.
func TestInteractionsListRejectsInvalidLimitOffset(t *testing.T) {
	s := setupInteractionsListServer(t)
	for _, query := range []string{"limit=abc", "offset=1.5"} {
		rec := getInteractions(t, s, query)
		if rec.Code != http.StatusBadRequest {
			t.Errorf("%s: status = %d, want 400 (body %s)", query, rec.Code, rec.Body.String())
		}
	}
}

// TestInteractionsListFiniteScoreBoundsUnchanged is the control: valid
// finite bounds behave exactly as before the 400 rule existed.
func TestInteractionsListFiniteScoreBoundsUnchanged(t *testing.T) {
	s := setupInteractionsListServer(t)
	cases := []struct {
		query string
		want  int
	}{
		{"", 3},
		{"minScore=0.4", 2},
		{"maxScore=0.4", 1},
		{"minScore=0.4&maxScore=0.6", 1},
		{"limit=2", 2},
		{"limit=2&offset=2", 1},
	}
	for _, tc := range cases {
		rec := getInteractions(t, s, tc.query)
		if rec.Code != http.StatusOK {
			t.Errorf("%q: status = %d, want 200 (body %s)", tc.query, rec.Code, rec.Body.String())
			continue
		}
		var list []logstore.InteractionRecord
		if err := json.NewDecoder(rec.Body).Decode(&list); err != nil {
			t.Fatalf("%q: decode list: %v", tc.query, err)
		}
		if len(list) != tc.want {
			t.Errorf("%q: got %d rows, want %d", tc.query, len(list), tc.want)
		}
	}
}
