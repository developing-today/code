package daemon

import (
	"net/http"

	"github.com/developing-today/code/pkgs/mcpx/internal/api"
)

// The authorisation decision report over the daemon API (#1972 step 2):
// the operator's read of the permissive funnel's record. GET returns the
// counts by operation (with the permission tier a future policy would
// require) and by principal over the recorded window; DELETE clears the
// record so the next read covers only what happened after the clear.
// Both are declared operations, so the funnel records them like any
// other request — a report read shows up in the following report under
// authz_decisions, and the clear's own decision is wiped by the clear it
// performs. Neither changes any behaviour: the stage stays permissive,
// nothing is refused, and no credential value exists anywhere in the
// record to expose.

func (s *Server) handleAuthzDecisions(w http.ResponseWriter, _ *http.Request) {
	writeJSON(w, 200, api.ReportDecisions())
}

func (s *Server) handleAuthzDecisionsClear(w http.ResponseWriter, _ *http.Request) {
	api.ClearDecisions()
	writeJSON(w, 200, api.ReportDecisions())
}
