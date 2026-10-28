package mcpclient_test

import (
	"context"
	"errors"
	"strings"
	"sync"
	"testing"
	"time"

	"github.com/dezren39/mcpx/internal/mcpclient"
)

// markedLost is the error frame lost() synthesises when a modern response
// stream breaks unanswered: codeStreamLost with the data marker.
func markedLost() map[string]any {
	return rpcErr(-32000, "the server closed the response stream before answering",
		map[string]any{"mcpxStreamLost": true})
}

type reissueStep struct {
	result any
	err    map[string]any
}

// reissueScript serves queued steps per key (method, or method:tool for
// tools/call); an exhausted queue answers method-not-found.
type reissueScript struct {
	mu    sync.Mutex
	steps map[string][]reissueStep
	hook  func(key string)
}

func (q *reissueScript) serve(method string, params map[string]any) (any, map[string]any) {
	key := method
	if method == "tools/call" {
		if name, _ := params["name"].(string); name != "" {
			key = method + ":" + name
		}
	}
	q.mu.Lock()
	steps := q.steps[key]
	var st reissueStep
	if len(steps) == 0 {
		st = reissueStep{err: rpcErr(-32601, "no scripted step for "+key, nil)}
	} else {
		st = steps[0]
		q.steps[key] = steps[1:]
	}
	hook := q.hook
	q.mu.Unlock()
	if hook != nil {
		hook(key)
	}
	return st.result, st.err
}

// reissueServer is a modern-only probe server whose non-handshake methods
// are answered by the script.
func reissueServer(q *reissueScript) *probeServer {
	return newProbeServer(func(method string, params map[string]any) (any, map[string]any, bool, time.Duration) {
		switch method {
		case "initialize":
			return nil, rpcErr(-32601, "initialize unsupported; supported: 2026-07-28", nil), false, 0
		case "server/discover":
			return discoverResult("2026-07-28"), nil, false, 0
		}
		r, e := q.serve(method, params)
		return r, e, false, 0
	})
}

func reissueDial(t *testing.T, s *probeServer) *mcpclient.Client {
	t.Helper()
	c, err := probeDial(t, s, mcpclient.Options{})
	if err != nil {
		t.Fatal(err)
	}
	if c.Era != mcpclient.EraModern {
		t.Fatalf("era = %q, want modern", c.Era)
	}
	return c
}

// requestIDs returns the ids of the frames sent for method (and tool, when
// set), in send order.
func requestIDs(s *probeServer, method, tool string) []float64 {
	s.mu.Lock()
	defer s.mu.Unlock()
	var ids []float64
	for _, f := range s.frames {
		if f["method"] != method {
			continue
		}
		if tool != "" {
			p, _ := f["params"].(map[string]any)
			if p["name"] != tool {
				continue
			}
		}
		id, _ := f["id"].(float64)
		ids = append(ids, id)
	}
	return ids
}

func TestModernReissueBrokenRequest(t *testing.T) {
	t.Run("2026-07-28", func(t *testing.T) {
		t.Run("transport", func(t *testing.T) {
			t.Run("client-reissues-broken-request-once", func(t *testing.T) {
				q := &reissueScript{steps: map[string][]reissueStep{
					"resources/read": {
						{err: markedLost()},
						{result: map[string]any{"contents": []any{}}},
					},
				}}
				s := reissueServer(q)
				c := reissueDial(t, s)
				ctx, cancel := context.WithTimeout(context.Background(), 5*time.Second)
				defer cancel()
				if _, err := c.ReadResource(ctx, "file:///x"); err != nil {
					t.Fatalf("a marked stream-lost must be re-issued and served: %v", err)
				}
				ids := requestIDs(s, "resources/read", "")
				if len(ids) != 2 {
					t.Fatalf("resources/read sent %d times, want exactly 2 (original + one re-issue)", len(ids))
				}
				if ids[0] == ids[1] {
					t.Fatalf("the re-issue must carry a fresh id, both were %v", ids[0])
				}
			})
		})
	})
}

func TestReissueSecondStreamLostSurfaces(t *testing.T) {
	q := &reissueScript{steps: map[string][]reissueStep{
		"resources/read": {{err: markedLost()}, {err: markedLost()}},
	}}
	s := reissueServer(q)
	c := reissueDial(t, s)
	ctx, cancel := context.WithTimeout(context.Background(), 5*time.Second)
	defer cancel()
	_, err := c.ReadResource(ctx, "file:///x")
	if err == nil || !strings.Contains(err.Error(), "the server closed the response stream") {
		t.Fatalf("a second stream-lost must surface, got %v", err)
	}
	if ids := requestIDs(s, "resources/read", ""); len(ids) != 2 {
		t.Fatalf("resources/read sent %d times, want 2 (one re-issue, no more)", len(ids))
	}
}

func TestUnmarkedStreamLostErrorsAreNotReissued(t *testing.T) {
	cases := map[string]map[string]any{
		// The receive loop's own -32000, as fail() synthesises it.
		"connection-closed": rpcErr(-32000, "connection closed: boom", nil),
		// A server-sent -32000 with the lost message but no marker: no
		// message matching anywhere.
		"server-sent-same-message": rpcErr(-32000, "the server closed the response stream before answering", nil),
		// A server-sent -32000 whose data lacks the marker.
		"server-sent-other-data": rpcErr(-32000, "server error", map[string]any{"other": true}),
	}
	for name, frame := range cases {
		t.Run(name, func(t *testing.T) {
			q := &reissueScript{steps: map[string][]reissueStep{
				"resources/read": {{err: frame}, {result: map[string]any{"contents": []any{}}}},
			}}
			s := reissueServer(q)
			c := reissueDial(t, s)
			ctx, cancel := context.WithTimeout(context.Background(), 5*time.Second)
			defer cancel()
			if _, err := c.ReadResource(ctx, "file:///x"); err == nil {
				t.Fatal("an unmarked -32000 must surface, not be re-issued")
			}
			if ids := requestIDs(s, "resources/read", ""); len(ids) != 1 {
				t.Fatalf("resources/read sent %d times, want 1 (no re-issue)", len(ids))
			}
		})
	}
}

func TestReissueSkipsCancelledCaller(t *testing.T) {
	ctx, cancel := context.WithCancel(context.Background())
	defer cancel()
	q := &reissueScript{
		steps: map[string][]reissueStep{
			"resources/read": {{err: markedLost()}, {result: map[string]any{"contents": []any{}}}},
		},
		hook: func(key string) {
			if key == "resources/read" {
				cancel()
			}
		},
	}
	s := reissueServer(q)
	c := reissueDial(t, s)
	_, err := c.ReadResource(ctx, "file:///x")
	if !errors.Is(err, context.Canceled) {
		t.Fatalf("a cancelled caller must see its cancellation, got %v", err)
	}
	if ids := requestIDs(s, "resources/read", ""); len(ids) != 1 {
		t.Fatalf("resources/read sent %d times, want 1 (no re-issue after cancel)", len(ids))
	}
}

func TestReissueLegacyEraUntouched(t *testing.T) {
	q := &reissueScript{steps: map[string][]reissueStep{
		"resources/read": {{err: markedLost()}, {result: map[string]any{"contents": []any{}}}},
	}}
	s := newProbeServer(func(method string, params map[string]any) (any, map[string]any, bool, time.Duration) {
		switch method {
		case "initialize":
			return initOK, nil, false, 0
		case "server/discover":
			return nil, rpcErr(-32601, "no server/discover", nil), false, 0
		}
		r, e := q.serve(method, params)
		return r, e, false, 0
	})
	c, err := probeDial(t, s, mcpclient.Options{Preference: mcpclient.ForceLegacy})
	if err != nil {
		t.Fatal(err)
	}
	ctx, cancel := context.WithTimeout(context.Background(), 5*time.Second)
	defer cancel()
	if _, err := c.ReadResource(ctx, "file:///x"); err == nil {
		t.Fatal("a legacy stream-lost must surface (resume is the legacy mechanism)")
	}
	if ids := requestIDs(s, "resources/read", ""); len(ids) != 1 {
		t.Fatalf("resources/read sent %d times, want 1 (no re-issue in the legacy era)", len(ids))
	}
}

func TestCallToolReissueAnnotationGate(t *testing.T) {
	tool := func(name string, annotations map[string]any) map[string]any {
		m := map[string]any{"name": name, "inputSchema": map[string]any{"type": "object"}}
		if annotations != nil {
			m["annotations"] = annotations
		}
		return m
	}
	ok := map[string]any{"content": []any{}}
	q := &reissueScript{steps: map[string][]reissueStep{
		"tools/list": {{result: map[string]any{"tools": []any{
			tool("safe", map[string]any{"readOnlyHint": true}),
			tool("idem", map[string]any{"idempotentHint": true}),
			tool("destructive", map[string]any{"destructiveHint": true}),
			tool("plain", nil),
		}}}},
		"tools/call:safe":        {{err: markedLost()}, {result: ok}},
		"tools/call:idem":        {{err: markedLost()}, {result: ok}},
		"tools/call:destructive": {{err: markedLost()}, {result: ok}},
		"tools/call:plain":       {{err: markedLost()}, {result: ok}},
	}}
	s := reissueServer(q)
	c := reissueDial(t, s)
	ctx, cancel := context.WithTimeout(context.Background(), 5*time.Second)
	defer cancel()

	// The host flow: tools are listed before they are called, which is
	// what retains the annotations the gate reads.
	tools, err := c.ListTools(ctx)
	if err != nil || len(tools) != 4 {
		t.Fatalf("ListTools = %d tools, %v; want 4 tools, no error", len(tools), err)
	}

	for _, name := range []string{"safe", "idem"} {
		if _, err := c.CallTool(ctx, name, map[string]any{}); err != nil {
			t.Fatalf("%s: an annotated-safe tool must be re-issued and served: %v", name, err)
		}
		ids := requestIDs(s, "tools/call", name)
		if len(ids) != 2 || ids[0] == ids[1] {
			t.Fatalf("%s: tools/call sent with ids %v, want 2 sends with distinct ids", name, ids)
		}
	}
	for _, name := range []string{"destructive", "plain"} {
		if _, err := c.CallTool(ctx, name, map[string]any{}); err == nil {
			t.Fatalf("%s: an unannotated/non-idempotent tool must not be re-issued", name)
		}
		if ids := requestIDs(s, "tools/call", name); len(ids) != 1 {
			t.Fatalf("%s: tools/call sent %d times, want 1 (gate closed)", name, len(ids))
		}
	}
}
