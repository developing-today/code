package daemon

import (
	"net/http"
	"os"
	"path"
	"path/filepath"
	"strings"

	"github.com/dezren39/mcpx/internal/dashboard"
	"github.com/dezren39/mcpx/internal/logstore"
)

func (s *Server) handleDashboard(w http.ResponseWriter, r *http.Request) {
	if !s.dashboardEnabled() {
		http.NotFound(w, r)
		return
	}
	if dir := s.dashboardDir(); dir != "" {
		// An external page's relative assets resolve against the URL it
		// was loaded from: at /dashboard, app.css means /app.css, which
		// is not a route. Send the browser to the trailing-slash form
		// first, where the asset routes live. Only when the index is
		// really there -- a missing directory keeps its plain 404.
		if !strings.HasSuffix(r.URL.Path, "/") {
			if _, err := os.Stat(filepath.Join(dir, "index.html")); err == nil {
				target := r.URL.Path + "/"
				if r.URL.RawQuery != "" {
					target += "?" + r.URL.RawQuery
				}
				http.Redirect(w, r, target, http.StatusMovedPermanently)
				return
			}
		}
		s.serveDashboardFile(w, r, dir, "index.html")
		return
	}
	w.Header().Set("Content-Type", "text/html; charset=utf-8")
	_, _ = w.Write([]byte(dashboard.HTML(s.version)))
}

// handleDashboardAsset serves a file next to a daemon.uiDir index.html. It
// exists because a page loaded from /dashboard resolves its relative links
// against the route root, so the files beside index.html need a route of
// their own; without an override there are none and every subpath is a 404.
func (s *Server) handleDashboardAsset(w http.ResponseWriter, r *http.Request) {
	if !s.dashboardEnabled() {
		http.NotFound(w, r)
		return
	}
	dir := s.dashboardDir()
	if dir == "" {
		http.NotFound(w, r)
		return
	}
	rel := strings.TrimPrefix(r.URL.Path, "/v1")
	rel = strings.TrimPrefix(rel, "/dashboard")
	s.serveDashboardFile(w, r, dir, strings.TrimPrefix(rel, "/"))
}

// dashboardEnabled reports whether /dashboard answers at all. The embedded
// page is on by default for the loopback bind it was written for; a bind
// widened past loopback is where the page needs an explicit yes. Read per
// request so a reload picks up a change without a restart.
func (s *Server) dashboardEnabled() bool {
	if s.set == nil {
		return true
	}
	if !s.set.Bool("daemon.ui") {
		return false
	}
	return s.set.Given("daemon.ui") || s.loopbackHost(s.set.String("daemon.address"))
}

// dashboardDir resolves daemon.uiDir, or "" for the embedded page.
func (s *Server) dashboardDir() string {
	if s.set == nil {
		return ""
	}
	dir := strings.TrimSpace(s.set.String("daemon.uiDir"))
	if dir == "" {
		return ""
	}
	if dir == "~" || strings.HasPrefix(dir, "~/") {
		if home, err := os.UserHomeDir(); err == nil {
			dir = filepath.Join(home, strings.TrimPrefix(dir, "~"))
		}
	}
	return dir
}

// serveDashboardFile serves one regular file from the override directory.
// Only files are served: a directory or a missing path is a 404, so turning
// the override on does not also publish a listing of whatever is in there.
func (s *Server) serveDashboardFile(w http.ResponseWriter, r *http.Request, dir, rel string) {
	if rel == "" || strings.HasSuffix(rel, "/") {
		rel += "index.html"
	}
	clean := path.Clean("/" + rel)
	if clean == "/" {
		clean = "/index.html"
	}
	full := filepath.Join(dir, filepath.FromSlash(clean))
	fi, err := os.Stat(full)
	if err != nil || fi.IsDir() {
		http.NotFound(w, r)
		return
	}
	http.ServeFile(w, r, full)
}

func (s *Server) handleMetricsTokens(w http.ResponseWriter, r *http.Request) {
	if s.tokenStore == nil {
		writeJSON(w, 200, logstore.TokenSummary{})
		return
	}
	sum, err := s.tokenStore.Summary()
	if err != nil {
		writeErr(w, 500, err)
		return
	}
	writeJSON(w, 200, sum)
}

func (s *Server) handleMetricsTools(w http.ResponseWriter, r *http.Request) {
	if s.set == nil {
		writeJSON(w, 200, map[string]any{
			"calls":        []any{},
			"errors":       []any{},
			"correlations": []any{},
			"timeline":     []any{},
		})
		return
	}
	st, err := s.openStore()
	if err != nil {
		writeJSON(w, 200, map[string]any{
			"calls":        []any{},
			"errors":       []any{},
			"correlations": []any{},
			"timeline":     []any{},
		})
		return
	}
	defer st.Close()

	calls, _ := st.Calls(logstore.Query{})
	errors, _ := st.Errors(logstore.Query{})
	correlations, _ := st.Correlations(logstore.Query{}, 20)
	timeline, _ := st.Timeline(logstore.Query{}, 24)

	if calls == nil {
		calls = []logstore.CallStat{}
	}
	if errors == nil {
		errors = []logstore.ErrorStat{}
	}
	if correlations == nil {
		correlations = []logstore.ToolCorrelation{}
	}
	if timeline == nil {
		timeline = []logstore.ToolTimelinePoint{}
	}

	writeJSON(w, 200, map[string]any{
		"calls":        calls,
		"errors":       errors,
		"correlations": correlations,
		"timeline":     timeline,
	})
}
