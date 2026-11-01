package daemon

import (
	"context"
	"io"
	"log"
	"net/http"
	"net/http/httptest"
	"testing"
	"time"

	"github.com/developing-today/code/pkgs/mcpx/internal/pool"
	"github.com/developing-today/code/pkgs/mcpx/internal/settings"
)

// #1864 item 4: a pool.callTimeout that arrived in the call-settings
// header is threaded onto the call context for the pool to clamp; the
// daemon's own value, from any other layer, is not a per-call request.
func TestRequestedCallTimeoutThreading(t *testing.T) {
	sch, err := settings.New(settings.Registry())
	if err != nil {
		t.Fatal(err)
	}
	s := &Server{
		set:    settings.NewSet(sch),
		logger: log.New(io.Discard, "", 0),
	}

	req := httptest.NewRequest(http.MethodPost, "/v1/call", nil)
	req.Header.Set(CallSettingsHeader, `{"pool.callTimeout":"5s"}`)
	ctx := withRequestedCallTimeout(withCallSettings(context.Background(), s.callSettings(req)))
	if d, ok := pool.RequestedCallTimeout(ctx); !ok || d != 5*time.Second {
		t.Fatalf("header request: got %v, %v; want 5s, true", d, ok)
	}

	plain := httptest.NewRequest(http.MethodPost, "/v1/call", nil)
	ctx = withRequestedCallTimeout(withCallSettings(context.Background(), s.callSettings(plain)))
	if _, ok := pool.RequestedCallTimeout(ctx); ok {
		t.Fatal("no header: a per-call request was threaded from nothing")
	}

	// An override that did not come through the header (a runtime change
	// to the daemon's own set) is the daemon's configuration, not a
	// caller's request.
	own := settings.NewSet(sch).WithOverrides(map[string]string{"pool.callTimeout": "5s"}, "test")
	ctx = withRequestedCallTimeout(withCallSettings(context.Background(), own))
	if _, ok := pool.RequestedCallTimeout(ctx); ok {
		t.Fatal("daemon-layer override was treated as a per-call request")
	}

	// No call settings on the context at all: untouched.
	if _, ok := pool.RequestedCallTimeout(withRequestedCallTimeout(context.Background())); ok {
		t.Fatal("bare context gained a per-call request")
	}
}

// The header value a script's calls ride on is exactly the caller's
// header-origin overrides -- never the daemon's own layers -- so a
// value can be forwarded to the generated client without leaking
// daemon configuration into a script's environment.
func TestCallSettingsHeaderValue(t *testing.T) {
	sch, err := settings.New(settings.Registry())
	if err != nil {
		t.Fatal(err)
	}
	s := &Server{
		set:    settings.NewSet(sch),
		logger: log.New(io.Discard, "", 0),
	}

	req := httptest.NewRequest(http.MethodPost, "/v1/exec", nil)
	req.Header.Set(CallSettingsHeader, `{"pool.callTimeout":"5s"}`)
	if got := callSettingsHeaderValue(s.callSettings(req)); got != `{"pool.callTimeout":"5s"}` {
		t.Fatalf("header request: got %q, want the header JSON back", got)
	}

	plain := httptest.NewRequest(http.MethodPost, "/v1/exec", nil)
	if got := callSettingsHeaderValue(s.callSettings(plain)); got != "" {
		t.Fatalf("no header: got %q, want empty", got)
	}

	own := settings.NewSet(sch).WithOverrides(map[string]string{"pool.callTimeout": "5s"}, "test")
	if got := callSettingsHeaderValue(own); got != "" {
		t.Fatalf("daemon-layer override leaked into the forwarded header: %q", got)
	}

	if got := callSettingsHeaderValue(nil); got != "" {
		t.Fatalf("nil set: got %q, want empty", got)
	}
	if got := callSettingsHeaderValue(callSettingsFrom(withCallSettings(context.Background(), s.callSettings(req)))); got != `{"pool.callTimeout":"5s"}` {
		t.Fatalf("context round trip: got %q", got)
	}
}
