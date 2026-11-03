package daemon

import (
	"encoding/json"
	"net/http"
	"net/http/httptest"
	"testing"

	"github.com/developing-today/code/pkgs/mcpx/internal/api"
)

// The decision report over the daemon API (#1972 step 2): traffic
// through the funnel appears in the report with counts by operation
// and principal. The report routes are declared operations, so the
// funnel records them too — a read shows up in the next report under
// authz_decisions — and the DELETE clear empties the window (its own
// decision is recorded before the handler runs, then wiped by the
// clear it performs).
func TestAuthzDecisionReportSurface(t *testing.T) {
	api.ClearDecisions()
	s := &Server{}
	mux := http.NewServeMux()
	s.route(mux, "GET /v1/health", func(w http.ResponseWriter, _ *http.Request) {
		w.WriteHeader(http.StatusNoContent)
	})
	s.route(mux, "GET /v1/authz/decisions", s.handleAuthzDecisions)
	s.route(mux, "DELETE /v1/authz/decisions", s.handleAuthzDecisionsClear)
	handler := s.authorize(mux)

	do := func(method, path string) *httptest.ResponseRecorder {
		rec := httptest.NewRecorder()
		handler.ServeHTTP(rec, httptest.NewRequest(method, path, nil))
		return rec
	}
	report := func() api.DecisionReport {
		t.Helper()
		rec := do(http.MethodGet, "/v1/authz/decisions")
		if rec.Code != http.StatusOK {
			t.Fatalf("GET /v1/authz/decisions = %d, want 200", rec.Code)
		}
		var got api.DecisionReport
		if err := json.Unmarshal(rec.Body.Bytes(), &got); err != nil {
			t.Fatalf("report body does not decode: %v (%s)", err, rec.Body.String())
		}
		return got
	}
	opCount := func(r api.DecisionReport, op string) int {
		for _, row := range r.ByOperation {
			if row.Op == op {
				return row.Count
			}
		}
		return 0
	}

	if rec := do(http.MethodGet, "/v1/health"); rec.Code != http.StatusNoContent {
		t.Fatalf("GET /v1/health = %d, want 204", rec.Code)
	}
	// This read records its own decision before the handler runs, so
	// the report it returns holds the health request and itself.
	got := report()
	if got.Total != 2 || got.Allowed != 2 {
		t.Fatalf("report after one health request = total %d allowed %d, want 2/2 (health + this read)", got.Total, got.Allowed)
	}
	if n := opCount(got, "health"); n != 1 {
		t.Errorf("report health count = %d, want 1", n)
	}
	if n := opCount(got, "authz_decisions"); n != 1 {
		t.Errorf("report authz_decisions count = %d, want 1 (the read itself)", n)
	}
	if len(got.ByPrincipal) == 0 {
		t.Error("report by_principal is empty after recorded requests")
	}
	if again := report(); again.Total != 3 {
		t.Errorf("report total after a second read = %d, want 3 (each read records itself)", again.Total)
	}

	rec := do(http.MethodDelete, "/v1/authz/decisions")
	if rec.Code != http.StatusOK {
		t.Fatalf("DELETE /v1/authz/decisions = %d, want 200", rec.Code)
	}
	var cleared api.DecisionReport
	if err := json.Unmarshal(rec.Body.Bytes(), &cleared); err != nil {
		t.Fatalf("clear body does not decode: %v", err)
	}
	if cleared.Total != 0 {
		t.Errorf("clear response total = %d, want 0", cleared.Total)
	}
	// After the clear, a read sees only its own decision.
	if after := report(); after.Total != 1 || opCount(after, "authz_decisions") != 1 {
		t.Errorf("report after clear = total %d (%+v), want only the reading decision itself", after.Total, after.ByOperation)
	}
}
