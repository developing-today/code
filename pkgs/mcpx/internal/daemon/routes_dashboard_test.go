package daemon

import (
	"encoding/json"
	"net/http"
	"net/http/httptest"
	"os"
	"path/filepath"
	"strings"
	"testing"

	"github.com/developing-today/code/pkgs/mcpx/internal/logstore"
	"github.com/developing-today/code/pkgs/mcpx/internal/settings"
)

func TestHandleDashboard(t *testing.T) {
	s := &Server{version: "1.2.3"}
	req := httptest.NewRequest(http.MethodGet, "/dashboard", nil)
	w := httptest.NewRecorder()

	s.handleDashboard(w, req)
	if w.Code != http.StatusOK {
		t.Fatalf("expected 200, got %d", w.Code)
	}
	body := w.Body.String()
	if !strings.Contains(body, "mcpx Dashboard") {
		t.Errorf("expected dashboard title in body")
	}
	if !strings.Contains(body, "1.2.3") {
		t.Errorf("expected version 1.2.3 in dashboard badge")
	}
}

func TestHandleDashboardGating(t *testing.T) {
	set := func(overrides map[string]string) *settings.Set {
		t.Helper()
		sch, err := settings.New(settings.Registry())
		if err != nil {
			t.Fatal(err)
		}
		return settings.NewSet(sch).WithOverrides(overrides, "test")
	}
	cases := []struct {
		name      string
		overrides map[string]string
		want      int
	}{
		{"default bind keeps the page", nil, http.StatusOK},
		{"off answers 404", map[string]string{"daemon.ui": "false"}, http.StatusNotFound},
		{"widened bind hides the inherited default", map[string]string{"daemon.address": "0.0.0.0"}, http.StatusNotFound},
		{"widened bind serves when asked", map[string]string{"daemon.address": "0.0.0.0", "daemon.ui": "true"}, http.StatusOK},
	}
	for _, tc := range cases {
		t.Run(tc.name, func(t *testing.T) {
			s := &Server{version: "1.2.3", set: set(tc.overrides)}
			w := httptest.NewRecorder()
			s.handleDashboard(w, httptest.NewRequest(http.MethodGet, "/dashboard", nil))
			if w.Code != tc.want {
				t.Fatalf("expected %d, got %d", tc.want, w.Code)
			}
		})
	}
}

func TestHandleDashboardDir(t *testing.T) {
	dir := t.TempDir()
	if err := os.WriteFile(filepath.Join(dir, "index.html"), []byte("<p>external ui</p>"), 0o644); err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(filepath.Join(dir, "app.css"), []byte("body{color:red}"), 0o644); err != nil {
		t.Fatal(err)
	}
	sch, err := settings.New(settings.Registry())
	if err != nil {
		t.Fatal(err)
	}
	s := &Server{version: "1.2.3", set: settings.NewSet(sch).WithOverrides(map[string]string{
		"daemon.uiDir": dir,
	}, "test")}

	// The slashless page URL redirects to the trailing-slash form, so
	// the index's relative assets resolve under /dashboard/ where the
	// asset routes live.
	w := httptest.NewRecorder()
	s.handleDashboard(w, httptest.NewRequest(http.MethodGet, "/dashboard", nil))
	if w.Code != http.StatusMovedPermanently || w.Header().Get("Location") != "/dashboard/" {
		t.Fatalf("expected a redirect to /dashboard/, got %d (Location %q)", w.Code, w.Header().Get("Location"))
	}
	w = httptest.NewRecorder()
	s.handleDashboard(w, httptest.NewRequest(http.MethodGet, "/v1/dashboard", nil))
	if w.Code != http.StatusMovedPermanently || w.Header().Get("Location") != "/v1/dashboard/" {
		t.Fatalf("expected a redirect to /v1/dashboard/, got %d (Location %q)", w.Code, w.Header().Get("Location"))
	}

	// At the slash form the page itself replaces the embedded one.
	w = httptest.NewRecorder()
	s.handleDashboardAsset(w, httptest.NewRequest(http.MethodGet, "/dashboard/", nil))
	if w.Code != http.StatusOK || !strings.Contains(w.Body.String(), "external ui") {
		t.Fatalf("expected the override index.html, got %d: %q", w.Code, w.Body.String())
	}

	// Files beside it are served for relative links.
	w = httptest.NewRecorder()
	s.handleDashboardAsset(w, httptest.NewRequest(http.MethodGet, "/dashboard/app.css", nil))
	if w.Code != http.StatusOK || !strings.Contains(w.Body.String(), "color:red") {
		t.Fatalf("expected app.css, got %d: %q", w.Code, w.Body.String())
	}

	// The directory route is page-shaped without the prefix too.
	w = httptest.NewRecorder()
	s.handleDashboardAsset(w, httptest.NewRequest(http.MethodGet, "/v1/dashboard/", nil))
	if w.Code != http.StatusOK || !strings.Contains(w.Body.String(), "external ui") {
		t.Fatalf("expected index.html for the bare directory, got %d", w.Code)
	}

	// A traversal stays inside the directory.
	w = httptest.NewRecorder()
	s.handleDashboardAsset(w, httptest.NewRequest(http.MethodGet, "/dashboard/../../../etc/passwd", nil))
	if w.Code != http.StatusNotFound {
		t.Fatalf("expected 404 for traversal, got %d", w.Code)
	}
}

func TestHandleDashboardDirMissing(t *testing.T) {
	sch, err := settings.New(settings.Registry())
	if err != nil {
		t.Fatal(err)
	}
	s := &Server{set: settings.NewSet(sch).WithOverrides(map[string]string{
		"daemon.uiDir": filepath.Join(t.TempDir(), "gone"),
	}, "test")}
	w := httptest.NewRecorder()
	s.handleDashboard(w, httptest.NewRequest(http.MethodGet, "/dashboard", nil))
	if w.Code != http.StatusNotFound {
		t.Fatalf("expected 404 for a missing uiDir, got %d", w.Code)
	}
}

func TestHandleMetricsTokens(t *testing.T) {
	tmpDir := t.TempDir()
	ts, err := logstore.NewTokenStore(filepath.Join(tmpDir, "tokens.db"))
	if err != nil {
		t.Fatal(err)
	}
	defer ts.Close()

	_ = ts.RecordTurn(logstore.TokenRecord{
		ID:             "turn-1",
		BaselineTokens: 1000,
		InjectedTokens: 300,
	})

	s := &Server{tokenStore: ts}
	req := httptest.NewRequest(http.MethodGet, "/v1/metrics/tokens", nil)
	w := httptest.NewRecorder()

	s.handleMetricsTokens(w, req)
	if w.Code != http.StatusOK {
		t.Fatalf("expected 200, got %d", w.Code)
	}

	var sum logstore.TokenSummary
	if err := json.Unmarshal(w.Body.Bytes(), &sum); err != nil {
		t.Fatal(err)
	}
	if sum.TotalTurns != 1 {
		t.Errorf("expected 1 turn, got %d", sum.TotalTurns)
	}
	if sum.TotalTokensSaved != 700 {
		t.Errorf("expected 700 tokens saved, got %d", sum.TotalTokensSaved)
	}
}

func TestHandleMetricsTools(t *testing.T) {
	s := &Server{}
	req := httptest.NewRequest(http.MethodGet, "/v1/metrics/tools", nil)
	w := httptest.NewRecorder()

	s.handleMetricsTools(w, req)
	if w.Code != http.StatusOK {
		t.Fatalf("expected 200, got %d", w.Code)
	}

	var resp struct {
		Calls        []logstore.CallStat          `json:"calls"`
		Errors       []logstore.ErrorStat         `json:"errors"`
		Correlations []logstore.ToolCorrelation   `json:"correlations"`
		Timeline     []logstore.ToolTimelinePoint `json:"timeline"`
	}
	if err := json.Unmarshal(w.Body.Bytes(), &resp); err != nil {
		t.Fatalf("failed to decode response: %v", err)
	}
	if resp.Calls == nil || resp.Errors == nil || resp.Correlations == nil || resp.Timeline == nil {
		t.Errorf("expected non-nil slices in response")
	}
}

// The dashboard handlers read daemon.ui and daemon.uiDir per request, so
// both declarations must be hot: without Hot the settings API reports
// restartRequired and discards a runtime change the handlers would have
// honoured immediately.
func TestDashboardSettingsAreHot(t *testing.T) {
	for _, path := range []string{"daemon.ui", "daemon.uiDir"} {
		found := false
		for _, decl := range settings.Registry() {
			if decl.Path == path {
				found = true
				if !decl.Hot {
					t.Errorf("%s is not marked Hot; a runtime change would be discarded", path)
				}
			}
		}
		if !found {
			t.Errorf("no setting declared for %s", path)
		}
	}
}

// The embedded page must carry the TUI's six views and the elicitation
// panel (#1882): the settings slice alone left the page metrics-only, so
// this pins the wiring to the /v1 endpoints each view reads — a view
// renamed in the TUI or an endpoint dropped from the page fails here
// instead of silently shipping a page that cannot show it.
func TestHandleDashboardCarriesOperatorViews(t *testing.T) {
	s := &Server{version: "1.2.3"}
	w := httptest.NewRecorder()
	s.handleDashboard(w, httptest.NewRequest(http.MethodGet, "/dashboard", nil))
	if w.Code != http.StatusOK {
		t.Fatalf("expected 200, got %d", w.Code)
	}
	body := w.Body.String()
	for _, want := range []string{
		// The six TUI views, as tabs.
		`data-view="browse"`, `data-view="log"`, `data-view="stats"`,
		`data-view="servers"`, `data-view="sessions"`, `data-view="storage"`,
		// The /v1 endpoints those views read.
		"/v1/namespaces", "/v1/tools", "/v1/log?limit=50",
		"/v1/stats?dimension=", "/v1/stats?dimension=sessions",
		"/v1/servers",
		// Elicitation: list, answer, and the live event stream.
		"/v1/elicit", "answerElicit", "/v1/events",
	} {
		if !strings.Contains(body, want) {
			t.Errorf("embedded dashboard is missing %q", want)
		}
	}
}
