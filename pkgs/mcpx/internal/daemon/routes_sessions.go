package daemon

import (
	"net/http"

	"github.com/dezren39/mcpx/internal/pool"
)

// routesSessions registers the session inventory (#1931 slice 1).
//
// The inventory is a read of what the pools already know: every live
// instance, as pool.InstanceStatus — the same snapshot `mcpx status`
// embeds per server, flattened into one list for a caller that wants the
// sessions without the rest of the status document. Nothing here kills,
// detaches or resumes; those are later slices of #1931.
func (s *Server) routesSessions(mux *http.ServeMux) {
	mux.HandleFunc("GET /v1/sessions", s.handleSessions)
}

func (s *Server) handleSessions(w http.ResponseWriter, _ *http.Request) {
	writeJSON(w, http.StatusOK, map[string]any{
		"sessions": sessionInventory(s.reg.Status()),
	})
}

// sessionInventory flattens every pool's instances into one list. The
// result is never nil: an empty inventory is an empty list, not null and
// not an error — a caller listing sessions on a fresh daemon has a valid
// answer, it is just short.
func sessionInventory(statuses []pool.Status) []pool.InstanceStatus {
	out := []pool.InstanceStatus{}
	for _, st := range statuses {
		out = append(out, st.Instances...)
	}
	return out
}
