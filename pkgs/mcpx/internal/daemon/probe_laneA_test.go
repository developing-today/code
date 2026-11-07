package daemon

import (
	"net/http"
	"net/http/httptest"
	"testing"
)

// Probe (lane A, 2026-11-01): present-but-invalid numeric query params on
// GET /v1/log (limit) and GET /v1/stats (top) are silently discarded by
// `strconv.Atoi` with the error ignored — the handler falls back to the
// configured default and answers 200, indistinguishable from a valid
// request. The same handler (handleLogQuery) rejects an invalid `since`
// with 400, and #1679/#2223 established 400-naming-the-parameter as the
// house rule for this exact shape.
func TestProbeInvalidNumericParams(t *testing.T) {
	s := &Server{paths: Paths{State: t.TempDir()}, set: gapTestSettings(t)}

	cases := []struct {
		name    string
		handler func(http.ResponseWriter, *http.Request)
		target  string
	}{
		{"log limit valid control", s.handleLogQuery, "/v1/log?limit=5"},
		{"log limit banana", s.handleLogQuery, "/v1/log?limit=banana"},
		{"log limit fractional", s.handleLogQuery, "/v1/log?limit=2.7"},
		{"log limit scientific", s.handleLogQuery, "/v1/log?limit=1e3"},
		{"log since banana (contrast)", s.handleLogQuery, "/v1/log?since=banana"},
		{"stats top valid control", s.handleStats, "/v1/stats?top=5"},
		{"stats top banana", s.handleStats, "/v1/stats?top=banana"},
		{"stats top fractional", s.handleStats, "/v1/stats?top=2.7"},
	}
	for _, tc := range cases {
		rec := httptest.NewRecorder()
		req := httptest.NewRequest(http.MethodGet, tc.target, nil)
		tc.handler(rec, req)
		t.Logf("%s -> %d body=%.120q", tc.name, rec.Code, rec.Body.String())
	}
}
