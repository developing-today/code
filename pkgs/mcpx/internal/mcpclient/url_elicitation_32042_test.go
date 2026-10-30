package mcpclient_test

import (
	"context"
	"errors"
	"strings"
	"sync/atomic"
	"testing"

	"github.com/dezren39/mcpx/internal/mcpclient"
)

// #2079 (Phase 2 of #2077): a -32042 UrlElicitationRequired from a
// 2025-11-25 upstream names URL elicitations in its error data. The
// client hands them to the installed handler and, only when every one
// is accepted, retries the original call exactly once.

func urlElicitationData() map[string]any {
	return map[string]any{"elicitations": []any{
		map[string]any{"mode": "url", "url": "https://example.invalid/auth", "elicitationId": "e1", "message": "sign in"},
	}}
}

// dial32042 builds a legacy client at rev whose tools/call answers are
// served by call (n is the 1-based call count). handler may be nil.
func dial32042(t *testing.T, rev string, handler mcpclient.URLElicitationHandler, call func(n int32) (any, map[string]any)) *mcpclient.Client {
	t.Helper()
	var calls atomic.Int32
	w := newWire(t, rev, func(m string, p map[string]any) (any, map[string]any) {
		switch m {
		case "initialize":
			return initResult(rev, nil), nil
		case "tools/call":
			return call(calls.Add(1))
		}
		return nil, nil
	})
	return dialWire(t, w, mcpclient.ForceLegacy, mcpclient.Options{OnURLElicitationRequired: handler})
}

func TestURLElicitationRequiredRetry(t *testing.T) {
	t.Run("accepted-then-retry-succeeds-exactly-once", func(t *testing.T) {
		var got []mcpclient.URLElicitation
		c := dial32042(t, "2025-11-25", func(_ context.Context, els []mcpclient.URLElicitation) (bool, error) {
			got = els
			return true, nil
		}, func(n int32) (any, map[string]any) {
			if n == 1 {
				return nil, rpcErr(-32042, "url elicitation required", urlElicitationData())
			}
			return map[string]any{"content": []any{}}, nil
		})
		if _, err := c.CallTool(ctx5(), "x", nil); err != nil {
			t.Fatal(err)
		}
		if len(got) != 1 || got[0].ElicitationID != "e1" || got[0].URL != "https://example.invalid/auth" || got[0].Mode != "url" {
			t.Fatalf("handler saw %v", got)
		}
	})

	t.Run("second-32042-surfaces-no-third-call", func(t *testing.T) {
		var calls atomic.Int32
		c := dial32042(t, "2025-11-25", func(_ context.Context, _ []mcpclient.URLElicitation) (bool, error) {
			return true, nil
		}, func(n int32) (any, map[string]any) {
			calls.Store(n)
			return nil, rpcErr(-32042, "url elicitation required", urlElicitationData())
		})
		_, err := c.CallTool(ctx5(), "x", nil)
		if err == nil || !strings.Contains(err.Error(), "-32042") {
			t.Fatalf("want the second -32042 to surface, got %v", err)
		}
		if calls.Load() != 2 {
			t.Fatalf("tools/call ran %d times, want exactly 2 (original + one retry)", calls.Load())
		}
	})

	t.Run("declined-fails-without-retry", func(t *testing.T) {
		var calls atomic.Int32
		c := dial32042(t, "2025-11-25", func(_ context.Context, _ []mcpclient.URLElicitation) (bool, error) {
			return false, nil
		}, func(n int32) (any, map[string]any) {
			calls.Store(n)
			return nil, rpcErr(-32042, "url elicitation required", urlElicitationData())
		})
		_, err := c.CallTool(ctx5(), "x", nil)
		if err == nil || !strings.Contains(err.Error(), "declined or expired") {
			t.Fatalf("want a named decline error, got %v", err)
		}
		if calls.Load() != 1 {
			t.Fatalf("tools/call ran %d times, want 1 (no retry after decline)", calls.Load())
		}
	})

	t.Run("handler-error-surfaces-without-retry", func(t *testing.T) {
		c := dial32042(t, "2025-11-25", func(_ context.Context, _ []mcpclient.URLElicitation) (bool, error) {
			return false, errors.New("the URL elicitation e1 from srv was decline")
		}, func(n int32) (any, map[string]any) {
			return nil, rpcErr(-32042, "url elicitation required", urlElicitationData())
		})
		_, err := c.CallTool(ctx5(), "x", nil)
		if err == nil || !strings.Contains(err.Error(), "was decline") {
			t.Fatalf("want the handler error to surface, got %v", err)
		}
	})

	t.Run("no-handler-propagates-untouched", func(t *testing.T) {
		var calls atomic.Int32
		c := dial32042(t, "2025-11-25", nil, func(n int32) (any, map[string]any) {
			calls.Store(n)
			return nil, rpcErr(-32042, "url elicitation required", urlElicitationData())
		})
		_, err := c.CallTool(ctx5(), "x", nil)
		if err == nil || !strings.Contains(err.Error(), "-32042") {
			t.Fatalf("want -32042 to propagate, got %v", err)
		}
		if calls.Load() != 1 {
			t.Fatalf("tools/call ran %d times, want 1", calls.Load())
		}
	})
}

func TestURLElicitationRequiredMalformed(t *testing.T) {
	cases := map[string]any{
		"unparseable-data": "junk",
		"empty-list":       map[string]any{"elicitations": []any{}},
		"form-mode-entry":  map[string]any{"elicitations": []any{map[string]any{"mode": "form", "elicitationId": "e1"}}},
		"missing-id-entry": map[string]any{"elicitations": []any{map[string]any{"mode": "url", "url": "https://example.invalid/a"}}},
		"no-elicitations":  map[string]any{"other": 1},
		"data-absent":      nil,
	}
	for name, data := range cases {
		t.Run(name, func(t *testing.T) {
			called := false
			var calls atomic.Int32
			c := dial32042(t, "2025-11-25", func(_ context.Context, _ []mcpclient.URLElicitation) (bool, error) {
				called = true
				return true, nil
			}, func(n int32) (any, map[string]any) {
				calls.Store(n)
				return nil, rpcErr(-32042, "url elicitation required", data)
			})
			_, err := c.CallTool(ctx5(), "x", nil)
			if err == nil || !strings.Contains(err.Error(), "-32042") {
				t.Fatalf("want the original -32042 unmodified, got %v", err)
			}
			if called {
				t.Fatal("the handler must not run for malformed data")
			}
			if calls.Load() != 1 {
				t.Fatalf("tools/call ran %d times, want 1", calls.Load())
			}
		})
	}
}

func TestURLElicitationRequiredEraGate(t *testing.T) {
	t.Run("2025-06-18-propagates", func(t *testing.T) {
		called := false
		c := dial32042(t, "2025-06-18", func(_ context.Context, _ []mcpclient.URLElicitation) (bool, error) {
			called = true
			return true, nil
		}, func(n int32) (any, map[string]any) {
			return nil, rpcErr(-32042, "url elicitation required", urlElicitationData())
		})
		_, err := c.CallTool(ctx5(), "x", nil)
		if err == nil || !strings.Contains(err.Error(), "-32042") {
			t.Fatalf("want -32042 to propagate at 2025-06-18, got %v", err)
		}
		if called {
			t.Fatal("the handler must not run outside 2025-11-25")
		}
	})

	t.Run("2026-propagates", func(t *testing.T) {
		called := false
		w := newWire(t, "2026-07-28", func(m string, p map[string]any) (any, map[string]any) {
			switch m {
			case "server/discover":
				return discoverWith(map[string]any{"tools": map[string]any{}}), nil
			case "tools/call":
				return nil, rpcErr(-32042, "url elicitation required", urlElicitationData())
			}
			return nil, nil
		})
		c := dialWire(t, w, mcpclient.ForceModern, mcpclient.Options{
			OnURLElicitationRequired: func(_ context.Context, _ []mcpclient.URLElicitation) (bool, error) {
				called = true
				return true, nil
			},
		})
		_, err := c.CallTool(ctx5(), "x", nil)
		if err == nil || !strings.Contains(err.Error(), "-32042") {
			t.Fatalf("want -32042 to propagate at 2026, got %v", err)
		}
		if called {
			t.Fatal("the handler must not run at 2026")
		}
	})
}
