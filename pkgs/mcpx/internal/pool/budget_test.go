package pool

import (
	"context"
	"testing"
	"time"

	"github.com/dezren39/mcpx/internal/config"
)

// #1864 item 4: a caller's per-call pool.callTimeout may lower the
// budget for one call, never raise it above the configured ceiling,
// and never leak into a call that did not ask.
func TestCallTimeoutEffective(t *testing.T) {
	cases := []struct {
		name string
		cfg  time.Duration
		req  time.Duration // 0 = no per-call request
		want time.Duration
	}{
		{"absent keeps configured", 2 * time.Second, 0, 2 * time.Second},
		{"lower applies", 2 * time.Second, 500 * time.Millisecond, 500 * time.Millisecond},
		{"higher clamps to configured", 2 * time.Second, 30 * time.Second, 2 * time.Second},
		{"equal is configured", 2 * time.Second, 2 * time.Second, 2 * time.Second},
		{"no configured bound, request applies as asked", 0, 3 * time.Second, 3 * time.Second},
		{"no bound at all", 0, 0, 0},
	}
	for _, c := range cases {
		t.Run(c.name, func(t *testing.T) {
			p := &Pool{cfg: &config.Resolved{CallTimeout: c.cfg}}
			ctx := context.Background()
			if c.req > 0 {
				ctx = WithCallTimeout(ctx, c.req)
			}
			if got := p.callTimeout(ctx); got != c.want {
				t.Fatalf("callTimeout = %v, want %v", got, c.want)
			}
		})
	}
	t.Run("no leak into a plain context", func(t *testing.T) {
		p := &Pool{cfg: &config.Resolved{CallTimeout: 2 * time.Second}}
		asked := WithCallTimeout(context.Background(), 100*time.Millisecond)
		if got := p.callTimeout(asked); got != 100*time.Millisecond {
			t.Fatalf("asked call = %v, want 100ms", got)
		}
		if got := p.callTimeout(context.Background()); got != 2*time.Second {
			t.Fatalf("next call = %v, want the configured 2s", got)
		}
		if _, ok := RequestedCallTimeout(context.Background()); ok {
			t.Fatal("a plain context reports a per-call request")
		}
	})
}

// Conflict #6 (WP7): the call budget must not run while a question is
// pending, and must still run out when none is.
func TestBudget(t *testing.T) {
	const d = 50 * time.Millisecond
	t.Run("mcpx/timeouts/budget-expires-with-no-question", func(t *testing.T) {
		var q questions
		ctx, cancel := budget(context.Background(), d, &q)
		defer cancel()
		select {
		case <-ctx.Done():
			if context.Cause(ctx) != errCallTimeout {
				t.Fatalf("cause = %v", context.Cause(ctx))
			}
		case <-time.After(20 * d):
			t.Fatal("the budget never ran out")
		}
	})
	t.Run("mcpx/timeouts/budget-paused-while-a-question-is-pending", func(t *testing.T) {
		var q questions
		ctx, cancel := budget(context.Background(), d, &q)
		defer cancel()
		done := q.asking()
		select {
		case <-ctx.Done():
			t.Fatal("the budget ran out while a question was pending")
		case <-time.After(10 * d):
		}
		done()
		select {
		case <-ctx.Done():
		case <-time.After(20 * d):
			t.Fatal("the budget did not resume after the question closed")
		}
	})
}
