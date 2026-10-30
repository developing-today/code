package mcpclient

import (
	"context"
	"encoding/json"
	"strings"
	"testing"
	"time"
)

// #2192 / design #2333: sampling and elicitation requests from an
// upstream server are bounded per server and (sampling) per session;
// over-limit questions fail with a named -32603 refusal and count
// against neither scope.

func rateLimitClient(server string, lim RequestLimits, shared *ServerRequestLimiter, calls *int) *Client {
	c := &Client{
		pending: map[int64]chan *rpcResponse{},
		onElicit: func(context.Context, string, json.RawMessage) (any, error) {
			*calls++
			return map[string]any{"action": "cancel"}, nil
		},
	}
	c.setupRequestLimits(Options{ServerName: server, RequestLimits: lim, ServerLimiter: shared})
	return c
}

var (
	samplingParams    = json.RawMessage(`{"messages":[{"role":"user","content":{"type":"text","text":"hi"}}],"maxTokens":10}`)
	elicitationParams = json.RawMessage(`{"mode":"form","message":"what?"}`)
)

func TestWindowCounter(t *testing.T) {
	w := newWindowCounter(2, time.Minute)
	now := time.Now()
	if !w.allow(now) || !w.allow(now) {
		t.Fatal("first two admissions refused")
	}
	if w.allow(now) {
		t.Fatal("third admission in the window allowed")
	}
	if !w.allow(now.Add(2 * time.Minute)) {
		t.Fatal("admission in the next window refused")
	}
	if !newWindowCounter(0, time.Minute).allow(now) {
		t.Fatal("zero limit should disable the counter")
	}
}

func TestServerRequestRateLimits(t *testing.T) {
	ctx := context.Background()

	t.Run("sampling under-limit passes, burst refused by session", func(t *testing.T) {
		calls := 0
		c := rateLimitClient("rl-session-server", RequestLimits{
			SamplingPerServer: 100, SamplingPerSession: 2, ElicitationPerServer: 100,
		}, nil, &calls)
		for i := 0; i < 2; i++ {
			if _, rerr := c.answer(ctx, "sampling/createMessage", samplingParams); rerr != nil {
				t.Fatalf("request %d refused under the limit: %v", i, rerr)
			}
		}
		_, rerr := c.answer(ctx, "sampling/createMessage", samplingParams)
		if rerr == nil || rerr.Code != codeInternalError || !strings.Contains(rerr.Message, "proto.samplingSessionPerMinute") {
			t.Fatalf("third request: got %v, want a -32603 naming proto.samplingSessionPerMinute", rerr)
		}
		if calls != 2 {
			t.Fatalf("handler invoked %d times, want 2 (refused request must not reach it)", calls)
		}
	})

	t.Run("sampling per-server budget shared across sessions, isolated per server", func(t *testing.T) {
		lim := RequestLimits{SamplingPerServer: 2, SamplingPerSession: 100, ElicitationPerServer: 100}
		sharedA := NewServerRequestLimiter(lim)
		callsA, callsB := 0, 0
		a1 := rateLimitClient("rl-shared-a", lim, sharedA, &callsA)
		a2 := rateLimitClient("rl-shared-a", lim, sharedA, &callsA)
		for i := 0; i < 2; i++ {
			if _, rerr := a1.answer(ctx, "sampling/createMessage", samplingParams); rerr != nil {
				t.Fatalf("server A request %d refused: %v", i, rerr)
			}
		}
		// A second session to the same server inherits the spent budget.
		_, rerr := a2.answer(ctx, "sampling/createMessage", samplingParams)
		if rerr == nil || !strings.Contains(rerr.Message, "proto.samplingPerMinute") {
			t.Fatalf("second session to server A: got %v, want refusal naming proto.samplingPerMinute", rerr)
		}
		// Another server's limiter is untouched by A's burst.
		b := rateLimitClient("rl-shared-b", lim, NewServerRequestLimiter(lim), &callsB)
		if _, rerr := b.answer(ctx, "sampling/createMessage", samplingParams); rerr != nil {
			t.Fatalf("server B refused by server A's burst: %v", rerr)
		}
	})

	t.Run("a refused question counts against neither scope", func(t *testing.T) {
		// Session limit 1, server limit 2: client A's second question is
		// refused by its session. If that refusal consumed server
		// budget, client B (fresh session, same limiter) would find
		// only the refusal's leavings; instead B is admitted, and a
		// third client meets the server's real limit of 2.
		lim := RequestLimits{SamplingPerServer: 2, SamplingPerSession: 1, ElicitationPerServer: 100}
		shared := NewServerRequestLimiter(lim)
		calls := 0
		a := rateLimitClient("rl-refused", lim, shared, &calls)
		b := rateLimitClient("rl-refused", lim, shared, &calls)
		third := rateLimitClient("rl-refused", lim, shared, &calls)
		if _, rerr := a.answer(ctx, "sampling/createMessage", samplingParams); rerr != nil {
			t.Fatalf("A first: %v", rerr)
		}
		if _, rerr := a.answer(ctx, "sampling/createMessage", samplingParams); rerr == nil {
			t.Fatal("A second: admitted past its session limit")
		}
		if _, rerr := b.answer(ctx, "sampling/createMessage", samplingParams); rerr != nil {
			t.Fatalf("B first refused -- A's refusal consumed server budget: %v", rerr)
		}
		_, rerr := third.answer(ctx, "sampling/createMessage", samplingParams)
		if rerr == nil || !strings.Contains(rerr.Message, "proto.samplingPerMinute") {
			t.Fatalf("third client: got %v, want the server limit of 2 to bite", rerr)
		}
	})

	t.Run("elicitation refused past the per-server limit", func(t *testing.T) {
		lim := RequestLimits{SamplingPerServer: 100, SamplingPerSession: 100, ElicitationPerServer: 1}
		shared := NewServerRequestLimiter(lim)
		calls := 0
		c1 := rateLimitClient("rl-elicit", lim, shared, &calls)
		c2 := rateLimitClient("rl-elicit", lim, shared, &calls)
		if _, rerr := c1.answer(ctx, "elicitation/create", elicitationParams); rerr != nil {
			t.Fatalf("first elicitation refused: %v", rerr)
		}
		_, rerr := c2.answer(ctx, "elicitation/create", elicitationParams)
		if rerr == nil || rerr.Code != codeInternalError || !strings.Contains(rerr.Message, "proto.elicitationPerMinute") {
			t.Fatalf("second elicitation (other session, same server): got %v, want -32603 naming proto.elicitationPerMinute", rerr)
		}
		if calls != 1 {
			t.Fatalf("handler invoked %d times, want 1", calls)
		}
	})

	t.Run("modern input_required entries count individually", func(t *testing.T) {
		calls := 0
		c := rateLimitClient("rl-modern", RequestLimits{
			SamplingPerServer: 100, SamplingPerSession: 1, ElicitationPerServer: 100,
		}, nil, &calls)
		ir := inputRequired{ResultType: "input_required"}
		ir.InputRequests = map[string]struct {
			Method string          `json:"method"`
			Params json.RawMessage `json:"params"`
		}{
			"one": {Method: "sampling/createMessage", Params: samplingParams},
			"two": {Method: "sampling/createMessage", Params: samplingParams},
		}
		if _, err := c.resolveInput(ctx, nil, ir); err == nil {
			t.Fatal("two sampling entries under a session limit of 1: resolveInput succeeded")
		}
		if calls != 1 {
			t.Fatalf("handler invoked %d times, want exactly 1 (one entry admitted, one refused)", calls)
		}
	})

	t.Run("zero limits disable enforcement", func(t *testing.T) {
		calls := 0
		c := rateLimitClient("rl-disabled", RequestLimits{
			SamplingPerServer: -1, SamplingPerSession: -1, ElicitationPerServer: -1,
		}, nil, &calls)
		for i := 0; i < 5; i++ {
			if _, rerr := c.answer(ctx, "sampling/createMessage", samplingParams); rerr != nil {
				t.Fatalf("request %d refused with limits disabled: %v", i, rerr)
			}
		}
		if calls != 5 {
			t.Fatalf("handler invoked %d times, want 5", calls)
		}
	})
}
