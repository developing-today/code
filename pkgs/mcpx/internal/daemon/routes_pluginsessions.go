package daemon

import (
	"encoding/json"
	"fmt"
	"net/http"

	"github.com/developing-today/code/pkgs/mcpx/internal/defaults"
	"github.com/developing-today/code/pkgs/mcpx/internal/pluginsession"
)

// routesPluginSessions registers the plugin-session surface (#1873,
// Approach C): a plugin connecting over /v1 opens a session, the
// daemon issues its id, and everything the session declares or owns
// is keyed to that id until the session ends.
//
// The registry itself (internal/pluginsession) is the single
// teardown point: this file's DELETE is the clean end, and any
// future crash detection ends a session through the same Close, so
// no path can leave a dead plugin's registrations behind.
func (s *Server) routesPluginSessions(mux *http.ServeMux) {
	s.route(mux, "POST /v1/plugin-sessions", s.handlePluginSessionOpen)
	s.route(mux, "GET /v1/plugin-sessions", s.handlePluginSessionList)
	s.route(mux, "GET /v1/plugin-sessions/{id}", s.handlePluginSessionGet)
	s.route(mux, "DELETE /v1/plugin-sessions/{id}", s.handlePluginSessionClose)
}

// pluginSessionOpenRequest is what a connecting plugin sends: its
// manifest's name, and the capabilities and hook subscriptions the
// manifest declares. Capabilities are recorded and reportable and
// nothing more (decision 0003 -- no enforcement here). Hook
// declarations are recorded the same way, and at open they are also
// resolved into live bus subscribers owned by the session
// (pluginsession.Resolver, #10046): decision 0001's rule stands --
// hooks come from the manifest, there is no registration route.
type pluginSessionOpenRequest struct {
	Plugin       string   `json:"plugin"`
	Capabilities []string `json:"capabilities"`
	Hooks        []string `json:"hooks"`
}

func (s *Server) handlePluginSessionOpen(w http.ResponseWriter, r *http.Request) {
	var req pluginSessionOpenRequest
	if err := json.NewDecoder(http.MaxBytesReader(w, r.Body,
		s.bodyBytes("http.bodyLimit", defaults.HTTPBodyLimit))).Decode(&req); err != nil {
		writeErr(w, http.StatusBadRequest, fmt.Errorf("plugin session: bad request body: %w", err))
		return
	}
	if req.Plugin == "" {
		writeErr(w, http.StatusBadRequest, fmt.Errorf("plugin session: plugin name is required"))
		return
	}
	sess := s.hookResolver().Open(req.Plugin, req.Capabilities, req.Hooks)
	writeJSON(w, http.StatusCreated, map[string]any{"session": sess})
}

func (s *Server) handlePluginSessionList(w http.ResponseWriter, _ *http.Request) {
	writeJSON(w, http.StatusOK, map[string]any{
		"sessions": s.pluginSessions.List(),
	})
}

func (s *Server) handlePluginSessionGet(w http.ResponseWriter, r *http.Request) {
	id := r.PathValue("id")
	sess, ok := s.pluginSessions.Get(id)
	if !ok {
		writeErr(w, http.StatusNotFound, fmt.Errorf("plugin session %s not found", id))
		return
	}
	writeJSON(w, http.StatusOK, map[string]any{"session": sess})
}

// handlePluginSessionClose ends a session: the registry drops the
// session and everything it owns in one atomic step, and the answer
// is the dropped record, so the caller can see exactly what ended.
func (s *Server) handlePluginSessionClose(w http.ResponseWriter, r *http.Request) {
	id := r.PathValue("id")
	dropped, ok := s.hookResolver().Close(id)
	if !ok {
		writeErr(w, http.StatusNotFound, fmt.Errorf("plugin session %s not found", id))
		return
	}
	writeJSON(w, http.StatusOK, map[string]any{"closed": dropped})
}

// hookResolver is the server's manifest-hook resolver, built on
// first use against this server's registry and event bus. A server
// with no bus (tests construct them bare) resolves nothing: the
// declarations stay recorded, exactly as the foundation left them.
func (s *Server) hookResolver() *pluginsession.Resolver {
	s.pluginHooksOnce.Do(func() {
		s.pluginHooks = pluginsession.NewResolver(&s.pluginSessions, s.Events)
	})
	return s.pluginHooks
}

// pluginSessionStatus is the /v1/status embedding: the same list
// the plugin-sessions routes serve, so `mcpx status` answers "what
// is extending this daemon" with the live plugin sessions included.
func (s *Server) pluginSessionStatus() []pluginsession.Session {
	return s.pluginSessions.List()
}
