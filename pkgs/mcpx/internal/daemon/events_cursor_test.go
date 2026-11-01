package daemon

import (
	"context"
	"net/http"
	"net/http/httptest"
	"strings"
	"testing"
	"time"

	"github.com/developing-today/code/pkgs/mcpx/internal/defaults"
	"github.com/developing-today/code/pkgs/mcpx/internal/events"
)

// #1679: handleEvents discarded the strconv.ParseUint error for its
// resume cursors, so a malformed since / Last-Event-ID was silently
// coerced -- a syntax error to 0 (replay everything retained) and an
// out-of-range value to MaxUint64 (replay nothing, no gap). A
// present-but-unparseable cursor is now a 400 naming the parameter,
// answered before any stream headers; the absent, explicit-zero, and
// valid behaviours are unchanged.

func TestEventsRejectsAMalformedSince(t *testing.T) {
	s := &Server{Events: events.New(defaults.EventHistory), set: gapTestSettings(t)}
	for _, raw := range []string{"banana", "-1", "99999999999999999999999999", ""} {
		rec := httptest.NewRecorder()
		req := httptest.NewRequest(http.MethodGet, "/v1/events?since="+raw, nil)
		s.handleEvents(rec, req)
		if rec.Code != http.StatusBadRequest {
			t.Errorf("since=%q: got status %d, want 400", raw, rec.Code)
		}
		if !strings.Contains(rec.Body.String(), "since") {
			t.Errorf("since=%q: the 400 should name the parameter, got %q", raw, rec.Body.String())
		}
	}
}

func TestEventsRejectsAMalformedLastEventID(t *testing.T) {
	s := &Server{Events: events.New(defaults.EventHistory), set: gapTestSettings(t)}
	for _, raw := range []string{"banana", "99999999999999999999999999"} {
		rec := httptest.NewRecorder()
		req := httptest.NewRequest(http.MethodGet, "/v1/events", nil)
		req.Header.Set("Last-Event-ID", raw)
		s.handleEvents(rec, req)
		if rec.Code != http.StatusBadRequest {
			t.Errorf("Last-Event-ID=%q: got status %d, want 400", raw, rec.Code)
		}
		if !strings.Contains(rec.Body.String(), "Last-Event-ID") {
			t.Errorf("Last-Event-ID=%q: the 400 should name the parameter, got %q", raw, rec.Body.String())
		}
	}
}

// serveEvents runs the streaming handler in the background; the
// returned finish function cancels the request and waits for the
// handler to return, so reading the recorder afterwards is safe. The
// replay (or its absence) is decided synchronously at subscribe time,
// before the handler's first flush, so a short settle is enough.
func serveEvents(t *testing.T, s *Server, req *http.Request) func() *httptest.ResponseRecorder {
	t.Helper()
	ctx, cancel := context.WithCancel(req.Context())
	req = req.WithContext(ctx)
	rec := httptest.NewRecorder()
	done := make(chan struct{})
	go func() {
		defer close(done)
		s.handleEvents(rec, req)
	}()
	time.Sleep(150 * time.Millisecond)
	return func() *httptest.ResponseRecorder {
		cancel()
		<-done
		return rec
	}
}

func TestEventsCursorBehavioursUnchanged(t *testing.T) {
	newBus := func() *events.Bus {
		bus := events.New(defaults.EventHistory)
		bus.Publish(events.Event{Kind: events.CallFinished})
		bus.Publish(events.Event{Kind: events.CallFinished})
		return bus
	}

	// Absent cursor: live only -- the two retained events are not
	// replayed.
	s := &Server{Events: newBus(), set: gapTestSettings(t)}
	rec := serveEvents(t, s, httptest.NewRequest(http.MethodGet, "/v1/events", nil))()
	if rec.Code != http.StatusOK {
		t.Errorf("absent cursor: got status %d, want 200", rec.Code)
	}
	if strings.Contains(rec.Body.String(), "id: 1\n") {
		t.Errorf("absent cursor should be live only, but the retained history was replayed:\n%s", rec.Body.String())
	}

	// Explicit zero: replay everything retained.
	s = &Server{Events: newBus(), set: gapTestSettings(t)}
	rec = serveEvents(t, s, httptest.NewRequest(http.MethodGet, "/v1/events?since=0", nil))()
	if rec.Code != http.StatusOK {
		t.Errorf("since=0: got status %d, want 200", rec.Code)
	}
	if !strings.Contains(rec.Body.String(), "id: 1\n") || !strings.Contains(rec.Body.String(), "id: 2\n") {
		t.Errorf("since=0 should replay both retained events:\n%s", rec.Body.String())
	}

	// A valid cursor past both events: streams, replays nothing.
	s = &Server{Events: newBus(), set: gapTestSettings(t)}
	rec = serveEvents(t, s, httptest.NewRequest(http.MethodGet, "/v1/events?since=2", nil))()
	if rec.Code != http.StatusOK {
		t.Errorf("since=2: got status %d, want 200", rec.Code)
	}
	if strings.Contains(rec.Body.String(), "id: 1\n") || strings.Contains(rec.Body.String(), "id: 2\n") {
		t.Errorf("since=2 should replay nothing:\n%s", rec.Body.String())
	}

	// A valid Last-Event-ID resumes after it.
	s = &Server{Events: newBus(), set: gapTestSettings(t)}
	req := httptest.NewRequest(http.MethodGet, "/v1/events", nil)
	req.Header.Set("Last-Event-ID", "1")
	rec = serveEvents(t, s, req)()
	if rec.Code != http.StatusOK {
		t.Errorf("Last-Event-ID=1: got status %d, want 200", rec.Code)
	}
	if !strings.Contains(rec.Body.String(), "id: 2\n") || strings.Contains(rec.Body.String(), "id: 1\n") {
		t.Errorf("Last-Event-ID=1 should replay only event 2:\n%s", rec.Body.String())
	}
}
