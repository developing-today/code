package daemon

import (
	"context"
	"io"
	"log"
	"net/http"
	"net/http/httptest"
	"testing"
	"time"

	"github.com/dezren39/mcpx/internal/pool"
	"github.com/dezren39/mcpx/internal/settings"
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
