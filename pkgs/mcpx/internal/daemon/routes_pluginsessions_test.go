package daemon

import (
	"encoding/json"
	"net/http"
	"net/http/httptest"
	"strings"
	"testing"
	"time"

	"github.com/developing-today/code/pkgs/mcpx/internal/events"
	"github.com/developing-today/code/pkgs/mcpx/internal/pluginsession"
)

// The plugin-session routes (#1873): open issues an id and records
// the manifest declarations, the session is visible in the list and
// in the /v1/status embedding while it lives, and closing it drops
// it from every surface at once.

func openPluginSession(t *testing.T, s *Server, body string) pluginsession.Session {
	t.Helper()
	rec := httptest.NewRecorder()
	req := httptest.NewRequest(http.MethodPost, "/v1/plugin-sessions", strings.NewReader(body))
	s.handlePluginSessionOpen(rec, req)
	if rec.Code != http.StatusCreated {
		t.Fatalf("open status = %d, want 201; body %s", rec.Code, rec.Body)
	}
	var out struct {
		Session pluginsession.Session `json:"session"`
	}
	if err := json.Unmarshal(rec.Body.Bytes(), &out); err != nil {
		t.Fatalf("open response does not decode: %v", err)
	}
	return out.Session
}

func TestPluginSessionOpenRequiresAPluginName(t *testing.T) {
	s := &Server{}
	rec := httptest.NewRecorder()
	req := httptest.NewRequest(http.MethodPost, "/v1/plugin-sessions", strings.NewReader(`{"capabilities":["x"]}`))
	s.handlePluginSessionOpen(rec, req)
	if rec.Code != http.StatusBadRequest {
		t.Fatalf("open without a plugin name: status = %d, want 400; body %s", rec.Code, rec.Body)
	}
	if got := s.pluginSessions.List(); len(got) != 0 {
		t.Fatalf("a refused open still created a session: %+v", got)
	}
}

func TestPluginSessionLifecycleAcrossSurfaces(t *testing.T) {
	s := &Server{}
	sess := openPluginSession(t, s, `{"plugin":"greeter","capabilities":["records.read"],"hooks":["onSuccess"]}`)
	if sess.ID == "" || sess.Plugin != "greeter" {
		t.Fatalf("open returned the wrong session: %+v", sess)
	}
	if len(sess.Capabilities) != 1 || sess.Capabilities[0] != "records.read" {
		t.Fatalf("declared capabilities not recorded: %+v", sess)
	}
	if len(sess.Hooks) != 1 || sess.Hooks[0] != "onSuccess" {
		t.Fatalf("declared hooks not recorded: %+v", sess)
	}

	// Visible in the list surface.
	rec := httptest.NewRecorder()
	s.handlePluginSessionList(rec, httptest.NewRequest(http.MethodGet, "/v1/plugin-sessions", nil))
	var listOut struct {
		Sessions []pluginsession.Session `json:"sessions"`
	}
	if err := json.Unmarshal(rec.Body.Bytes(), &listOut); err != nil {
		t.Fatalf("list response does not decode: %v", err)
	}
	if len(listOut.Sessions) != 1 || listOut.Sessions[0].ID != sess.ID {
		t.Fatalf("list does not show the live session: %+v", listOut)
	}

	// Visible in the /v1/status embedding.
	if got := s.pluginSessionStatus(); len(got) != 1 || got[0].ID != sess.ID {
		t.Fatalf("status embedding does not show the live session: %+v", got)
	}

	// Visible through the single-session read.
	rec = httptest.NewRecorder()
	req := httptest.NewRequest(http.MethodGet, "/v1/plugin-sessions/"+sess.ID, nil)
	req.SetPathValue("id", sess.ID)
	s.handlePluginSessionGet(rec, req)
	if rec.Code != http.StatusOK {
		t.Fatalf("get status = %d, want 200; body %s", rec.Code, rec.Body)
	}

	// Close drops it from every surface at once.
	rec = httptest.NewRecorder()
	req = httptest.NewRequest(http.MethodDelete, "/v1/plugin-sessions/"+sess.ID, nil)
	req.SetPathValue("id", sess.ID)
	s.handlePluginSessionClose(rec, req)
	if rec.Code != http.StatusOK {
		t.Fatalf("close status = %d, want 200; body %s", rec.Code, rec.Body)
	}
	var closeOut struct {
		Closed pluginsession.Session `json:"closed"`
	}
	if err := json.Unmarshal(rec.Body.Bytes(), &closeOut); err != nil {
		t.Fatalf("close response does not decode: %v", err)
	}
	if closeOut.Closed.ID != sess.ID || len(closeOut.Closed.Capabilities) != 1 {
		t.Fatalf("close did not return the dropped record: %+v", closeOut)
	}
	if got := s.pluginSessions.List(); len(got) != 0 {
		t.Fatalf("closed session still listed: %+v", got)
	}
	if got := s.pluginSessionStatus(); len(got) != 0 {
		t.Fatalf("closed session still in the status embedding: %+v", got)
	}

	// A second close, and a get, are 404 -- the session is gone, not
	// parked.
	for _, h := range []func(http.ResponseWriter, *http.Request){s.handlePluginSessionGet, s.handlePluginSessionClose} {
		rec = httptest.NewRecorder()
		req = httptest.NewRequest(http.MethodGet, "/v1/plugin-sessions/"+sess.ID, nil)
		req.SetPathValue("id", sess.ID)
		h(rec, req)
		if rec.Code != http.StatusNotFound {
			t.Fatalf("dead session answered %d, want 404", rec.Code)
		}
	}
}

func TestPluginSessionListEmptyRendersEmptyList(t *testing.T) {
	s := &Server{}
	rec := httptest.NewRecorder()
	s.handlePluginSessionList(rec, httptest.NewRequest(http.MethodGet, "/v1/plugin-sessions", nil))
	if rec.Code != http.StatusOK {
		t.Fatalf("status = %d, want 200; body %s", rec.Code, rec.Body)
	}
	if !strings.Contains(rec.Body.String(), `"sessions":[]`) {
		t.Fatalf("empty list must render as [], body %s", rec.Body)
	}
}

// Slice 2 (#10046) at the route surface: opening a session with
// manifest hooks resolves them live -- the owned hook item is on
// the session the open returns, a matching bus event is delivered
// addressed to the session, and closing the session drops the
// subscription so a post-close event delivers nothing.
func TestPluginSessionHooksResolveLiveAtOpen(t *testing.T) {
	s := &Server{Events: events.New(64)}
	sess := openPluginSession(t, s, `{"plugin":"greeter","hooks":["elicit"]}`)
	if len(sess.Owned) != 1 || sess.Owned[0] != (pluginsession.OwnedItem{Kind: "hook", Name: "elicit"}) {
		t.Fatalf("open must return the session owning its live hook: %+v", sess)
	}
	sub, _ := s.Events.Subscribe(events.Filter{Kinds: []string{string(pluginsession.DeliveryKind)}, Session: sess.ID}, 0)
	defer sub.Close()
	s.Events.Publish(events.Event{Kind: events.ElicitOpened})
	select {
	case e := <-sub.C:
		if e.Session != sess.ID {
			t.Fatalf("delivery addressed to %q, want the owning session %q", e.Session, sess.ID)
		}
	case <-time.After(2 * time.Second):
		t.Fatal("matching event produced no delivery to the owning session")
	}
	rec := httptest.NewRecorder()
	req := httptest.NewRequest(http.MethodDelete, "/v1/plugin-sessions/"+sess.ID, nil)
	req.SetPathValue("id", sess.ID)
	s.handlePluginSessionClose(rec, req)
	if rec.Code != http.StatusOK {
		t.Fatalf("close status = %d, want 200; body %s", rec.Code, rec.Body)
	}
	s.Events.Publish(events.Event{Kind: events.ElicitOpened})
	select {
	case e := <-sub.C:
		t.Fatalf("post-close event delivered to a dropped session: %+v", e)
	case <-time.After(250 * time.Millisecond):
	}
}
