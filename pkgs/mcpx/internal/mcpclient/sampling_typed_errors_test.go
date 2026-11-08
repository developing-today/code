package mcpclient_test

import (
	"context"
	"encoding/json"
	"errors"
	"fmt"
	"strings"
	"testing"
	"time"

	"github.com/developing-today/code/pkgs/mcpx/internal/mcpclient"
)

// Typed sampling errors (issue #1835, Phase 1 of #1834): a handler error
// that carries its own JSON-RPC code reaches the upstream with that code
// through BOTH paths into Client.answer -- the legacy wire, where the
// server sends sampling/createMessage as a request of its own, and the
// modern input_required round, where the same answer() runs inside
// resolveInput. An untyped handler error keeps the -32603 wrap.

var typedSampleParams = map[string]any{
	"messages":  []any{map[string]any{"role": "user", "content": map[string]any{"type": "text", "text": "hi"}}},
	"maxTokens": 100,
}

func legacyTypedWire(t *testing.T, h mcpclient.ElicitHandler) *wire {
	t.Helper()
	w := newWire(t, "2025-11-25", func(m string, p map[string]any) (any, map[string]any) {
		if m == "initialize" {
			return initResult("2025-11-25", nil), nil
		}
		return nil, nil
	})
	dialWire(t, w, mcpclient.ForceLegacy, mcpclient.Options{OnServerRequest: h})
	return w
}

func pushSample(t *testing.T, w *wire, id string) map[string]any {
	t.Helper()
	w.push(map[string]any{"jsonrpc": "2.0", "id": id, "method": "sampling/createMessage", "params": typedSampleParams})
	var r map[string]any
	waitFor(t, "a reply to "+id, func() bool { r = reply(w, id); return r != nil })
	return r
}

func TestSamplingTypedErrorsLegacyWire(t *testing.T) {
	t.Run("wrapped user rejection reaches the upstream as -1 naming the action", func(t *testing.T) {
		w := legacyTypedWire(t, func(context.Context, string, json.RawMessage) (any, error) {
			return nil, fmt.Errorf("broker answer: %w",
				mcpclient.UserRejectedError("the sampling request was decline"))
		})
		r := pushSample(t, w, "u1")
		e, _ := r["error"].(map[string]any)
		if e == nil || e["code"] != float64(-1) {
			t.Fatalf("declined sampling not relayed as -1: %v", r)
		}
		if msg, _ := e["message"].(string); !strings.Contains(msg, "decline") {
			t.Fatalf("the -1 message does not name the action: %v", e)
		}
	})

	t.Run("tool shape error reaches the upstream as -32602", func(t *testing.T) {
		w := legacyTypedWire(t, func(context.Context, string, json.RawMessage) (any, error) {
			return nil, mcpclient.ToolShapeError("sampling result content mixes tool results with other content")
		})
		r := pushSample(t, w, "u2")
		e, _ := r["error"].(map[string]any)
		if e == nil || e["code"] != float64(-32602) {
			t.Fatalf("tool-shape sampling error not relayed as -32602: %v", r)
		}
	})

	t.Run("untyped handler failure stays -32603", func(t *testing.T) {
		w := legacyTypedWire(t, func(context.Context, string, json.RawMessage) (any, error) {
			return nil, errors.New("the answerer fell over")
		})
		r := pushSample(t, w, "u3")
		e, _ := r["error"].(map[string]any)
		if e == nil || e["code"] != float64(-32603) {
			t.Fatalf("untyped sampling failure not relayed as -32603: %v", r)
		}
	})
}

func TestSamplingTypedErrorsModernInputRequired(t *testing.T) {
	ask := func(method string, params map[string]any) any {
		switch method {
		case "server/discover":
			return modernDiscover(params)
		case "tools/call":
			if _, ok := params["inputResponses"]; ok {
				return map[string]any{"resultType": "complete", "content": []any{}}
			}
			return map[string]any{
				"resultType": "input_required",
				"inputRequests": map[string]any{
					"q": map[string]any{"method": "sampling/createMessage", "params": typedSampleParams},
				},
			}
		}
		return nil
	}
	call := func(t *testing.T, h mcpclient.ElicitHandler) error {
		t.Helper()
		c := dialModern(t, newScripted(ask), mcpclient.Options{OnServerRequest: h})
		ctx, cancel := context.WithTimeout(context.Background(), 5*time.Second)
		defer cancel()
		_, err := c.CallTool(ctx, "ask", map[string]any{})
		return err
	}

	t.Run("wrapped user rejection reaches the caller as -1 naming the action", func(t *testing.T) {
		err := call(t, func(context.Context, string, json.RawMessage) (any, error) {
			return nil, fmt.Errorf("broker answer: %w",
				mcpclient.UserRejectedError("the sampling request was cancel"))
		})
		if err == nil || !strings.Contains(err.Error(), "mcp error -1") || !strings.Contains(err.Error(), "cancel") {
			t.Fatalf("modern declined sampling not relayed as -1 naming the action: %v", err)
		}
	})

	t.Run("untyped handler failure stays -32603", func(t *testing.T) {
		err := call(t, func(context.Context, string, json.RawMessage) (any, error) {
			return nil, errors.New("the answerer fell over")
		})
		if err == nil || !strings.Contains(err.Error(), "mcp error -32603") {
			t.Fatalf("modern untyped sampling failure not relayed as -32603: %v", err)
		}
	})
}
