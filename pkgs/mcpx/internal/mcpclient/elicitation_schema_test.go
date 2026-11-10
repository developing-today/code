package mcpclient_test

import (
	"context"
	"encoding/json"
	"strings"
	"testing"
	"time"

	"github.com/developing-today/code/pkgs/mcpx/internal/mcpclient"
)

// Phase 3 of #1481 (#1484): an accepted form answer is validated against
// the requestedSchema the server itself supplied, at the answer choke
// point, after defaults are applied.
//
// https://modelcontextprotocol.io/specification/2025-11-25/client/elicitation#form-mode-security
func TestElicitationAnswersValidatedAgainstRequestedSchema(t *testing.T) {
	schema := map[string]any{
		"type":     "object",
		"required": []any{"name"},
		"properties": map[string]any{
			"name": map[string]any{"type": "string", "default": "John Doe"},
			"age":  map[string]any{"type": "integer"},
		},
	}
	legacyAt := func(h mcpclient.ElicitHandler, warns chan mcpclient.Warning) (*wire, *mcpclient.Client) {
		w := newWire(t, "2025-11-25", func(m string, p map[string]any) (any, map[string]any) {
			if m == "initialize" {
				return initResult("2025-11-25", nil), nil
			}
			return nil, nil
		})
		c := dialWire(t, w, mcpclient.ForceLegacy, mcpclient.Options{OnServerRequest: h})
		if warns != nil {
			c.Subscribe(mcpclient.Notifications{OnWarning: func(wn mcpclient.Warning) { warns <- wn }})
		}
		return w, c
	}
	elicit := func(w *wire, id string, sch map[string]any) map[string]any {
		p := map[string]any{"message": "?", "mode": "form", "requestedSchema": sch}
		w.push(map[string]any{"jsonrpc": "2.0", "id": id, "method": "elicitation/create", "params": p})
		var r map[string]any
		waitFor(t, "a reply to "+id, func() bool { r = reply(w, id); return r != nil })
		return r
	}
	handler := func(out map[string]any) mcpclient.ElicitHandler {
		return func(context.Context, string, json.RawMessage) (any, error) { return out, nil }
	}

	t.Run("2025-11-25/elicitation/wrong-typed-answer-is-32602-not-relayed", func(t *testing.T) {
		warns := make(chan mcpclient.Warning, 4)
		w, _ := legacyAt(handler(map[string]any{"action": "accept", "content": map[string]any{"age": "forty-one"}}), warns)
		r := elicit(w, "v1", schema)
		e, _ := r["error"].(map[string]any)
		if e == nil || e["code"] != float64(-32602) {
			t.Fatalf("a wrong-typed answer must be -32602 to the server, got %v", r)
		}
		if msg, _ := e["message"].(string); !strings.Contains(msg, "$.age") {
			t.Errorf("the error must name the failing path, got %q", msg)
		}
		if r["result"] != nil {
			t.Errorf("the unvalidated answer must not be relayed: %v", r)
		}
		select {
		case wn := <-warns:
			if !strings.Contains(wn.Reason, "rejected") {
				t.Errorf("unexpected warning: %v", wn)
			}
		case <-time.After(2 * time.Second):
			t.Error("a Warning must be emitted for a rejected answer")
		}
	})

	t.Run("2025-11-25/elicitation/omitted-field-with-default-validates", func(t *testing.T) {
		// "name" is required by the schema; the answer omits it. Only
		// the defaults-before-validation order makes this valid.
		w, _ := legacyAt(handler(map[string]any{"action": "accept", "content": map[string]any{"age": 41}}), nil)
		r := elicit(w, "v2", schema)
		if r["error"] != nil {
			t.Fatalf("an answer made valid by its default must be relayed, got %v", r)
		}
		content, _ := r["result"].(map[string]any)["content"].(map[string]any)
		if content["name"] != "John Doe" || content["age"] != float64(41) {
			t.Errorf("relayed content = %v", content)
		}
	})

	t.Run("2025-11-25/elicitation/conforming-answer-relayed-decline-cancel-untouched", func(t *testing.T) {
		w, _ := legacyAt(handler(map[string]any{"action": "accept", "content": map[string]any{"name": "Ada", "age": 36}}), nil)
		r := elicit(w, "v3", schema)
		content, _ := r["result"].(map[string]any)["content"].(map[string]any)
		if r["error"] != nil || content["name"] != "Ada" || content["age"] != float64(36) {
			t.Errorf("a conforming answer must be relayed as today, got %v", r)
		}
		for i, action := range []string{"decline", "cancel"} {
			w, _ := legacyAt(handler(map[string]any{"action": action}), nil)
			r := elicit(w, "v4"+string(rune('a'+i)), schema)
			res, _ := r["result"].(map[string]any)
			if r["error"] != nil || res["action"] != action {
				t.Errorf("%s must pass through with no validation attempted, got %v", action, r)
			}
		}
	})

	t.Run("2025-11-25/elicitation/uncompilable-schema-relays-unvalidated-with-warning", func(t *testing.T) {
		// The schema is the server's own defect (a dialect schemaval
		// refuses): the user's answer must not be wedged on it.
		bad := map[string]any{
			"$schema": "https://example.invalid/draft-99/schema",
			"type":    "object",
			"properties": map[string]any{
				"age": map[string]any{"type": "integer"},
			},
		}
		warns := make(chan mcpclient.Warning, 4)
		w, _ := legacyAt(handler(map[string]any{"action": "accept", "content": map[string]any{"age": "forty-one"}}), warns)
		r := elicit(w, "v5", bad)
		if r["error"] != nil {
			t.Fatalf("an uncompilable requestedSchema must not error the answer, got %v", r)
		}
		content, _ := r["result"].(map[string]any)["content"].(map[string]any)
		if content["age"] != "forty-one" {
			t.Errorf("the answer must be relayed unvalidated, got %v", r)
		}
		select {
		case wn := <-warns:
			if !strings.Contains(wn.Reason, "unvalidated") {
				t.Errorf("unexpected warning: %v", wn)
			}
		case <-time.After(2 * time.Second):
			t.Error("a Warning must be emitted when validation is skipped")
		}
	})

	// The modern input-required flow inherits the check because
	// resolveInput routes through answer().
	t.Run("2026-07-28/elicitation/input-required-wrong-typed-answer-is-rejected", func(t *testing.T) {
		retried := false
		s := newScripted(func(method string, params map[string]any) any {
			switch method {
			case "server/discover":
				return modernDiscover(params)
			case "tools/call":
				if _, ok := params["inputResponses"]; ok {
					retried = true
					return map[string]any{"resultType": "complete", "content": []any{}}
				}
				return map[string]any{
					"resultType": "input_required",
					"inputRequests": map[string]any{
						"q": map[string]any{"method": "elicitation/create", "params": map[string]any{
							"mode": "form", "message": "?",
							"requestedSchema": map[string]any{"type": "object", "properties": map[string]any{
								"age": map[string]any{"type": "integer"},
							}},
						}},
					},
				}
			}
			return nil
		})
		c := dialModern(t, s, mcpclient.Options{
			OnServerRequest: handler(map[string]any{"action": "accept", "content": map[string]any{"age": "forty-one"}}),
		})
		ctx, cancel := context.WithTimeout(context.Background(), 5*time.Second)
		defer cancel()
		_, err := c.CallTool(ctx, "ask", map[string]any{})
		if err == nil || !strings.Contains(err.Error(), "requestedSchema") {
			t.Fatalf("the input-required round must surface the -32602, got %v", err)
		}
		if retried {
			t.Error("the unvalidated answer must never be retried upstream")
		}
	})
}
