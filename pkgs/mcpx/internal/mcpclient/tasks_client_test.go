package mcpclient_test

import (
	"context"
	"encoding/json"
	"errors"
	"strings"
	"sync"
	"testing"
	"time"

	"github.com/dezren39/mcpx/internal/mcpclient"
)

// Defining tests for #1556 (Phase 2 of #1554): CallTool drives a
// taskSupport-"required" tool through its era's task flow, and every
// other tool keeps the plain path. The fakes are scripted servers that
// record the exact request sequence, so the tests pin the wire shape,
// not just the outcome.

type taskFake struct {
	mu       sync.Mutex
	gets     int
	cancels  int
	plainReq bool // a tools/call for the required tool arrived without a task field
	extSeen  bool // a modern request declared the tasks extension
}

func (f *taskFake) methods(s *scriptedServer) []string {
	var out []string
	for _, r := range s.requests() {
		m, _ := r["method"].(string)
		out = append(out, m)
	}
	return out
}

func toolList() map[string]any {
	return map[string]any{"tools": []any{
		map[string]any{"name": "req", "execution": map[string]any{"taskSupport": "required"}},
		map[string]any{"name": "opt", "execution": map[string]any{"taskSupport": "optional"}},
	}}
}

// listFirst mirrors the pool's normal operation: the catalogue is
// listed on the pooled client (refreshSchemas) before it serves calls,
// which is what populates CallTool's taskSupport snapshot.
func listFirst(t *testing.T, c *mcpclient.Client) {
	t.Helper()
	ctx, cancel := context.WithTimeout(context.Background(), 5*time.Second)
	defer cancel()
	if _, err := c.ListTools(ctx); err != nil {
		t.Fatal(err)
	}
}

func dialLegacy(t *testing.T, s *scriptedServer) *mcpclient.Client {
	t.Helper()
	ctx, cancel := context.WithTimeout(context.Background(), 5*time.Second)
	defer cancel()
	c, err := mcpclient.NewWithOptions(ctx, s, mcpclient.Options{
		ClientName: "mcpx", ClientVersion: "t", Preference: mcpclient.ForceLegacy,
	})
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { c.Close() })
	return c
}

func TestLegacyRequiredToolRunsAsTask(t *testing.T) {
	f := &taskFake{}
	s := newScripted(func(method string, params map[string]any) any {
		switch method {
		case "initialize":
			return map[string]any{
				"protocolVersion": "2025-11-25",
				"serverInfo":      map[string]any{"name": "fake", "version": "1"},
				"capabilities":    map[string]any{},
			}
		case "tools/list":
			return toolList()
		case "tools/call":
			if _, ok := params["task"]; !ok {
				if params["name"] == "req" {
					f.mu.Lock()
					f.plainReq = true
					f.mu.Unlock()
				}
				return map[string]any{"content": []any{map[string]any{"type": "text", "text": "plain"}}}
			}
			return map[string]any{"task": map[string]any{
				"taskId": "t1", "status": "working", "ttl": 60000, "pollInterval": 1,
			}}
		case "tasks/get":
			f.mu.Lock()
			f.gets++
			n := f.gets
			f.mu.Unlock()
			status := "working"
			if n >= 2 {
				status = "completed"
			}
			return map[string]any{"taskId": "t1", "status": status, "ttl": 60000, "pollInterval": 1}
		case "tasks/result":
			return map[string]any{"content": []any{map[string]any{"type": "text", "text": "task-done"}}}
		}
		return nil
	})
	c := dialLegacy(t, s)
	listFirst(t, c)
	ctx, cancel := context.WithTimeout(context.Background(), 5*time.Second)
	defer cancel()

	raw, err := c.CallTool(ctx, "req", map[string]any{})
	if err != nil {
		t.Fatal(err)
	}
	if !strings.Contains(string(raw), "task-done") {
		t.Fatalf("result = %s, want the tasks/result payload", raw)
	}
	f.mu.Lock()
	plain := f.plainReq
	f.mu.Unlock()
	if plain {
		t.Fatal("the required tool was called without a task field")
	}
	seq := strings.Join(f.methods(s), ",")
	for _, want := range []string{"tools/call", "tasks/get", "tasks/result"} {
		if !strings.Contains(seq, want) {
			t.Fatalf("sequence %s missing %s", seq, want)
		}
	}

	// The optional tool keeps the plain path byte-for-byte: no task
	// field, no tasks/* traffic after its call.
	before := len(f.methods(s))
	raw, err = c.CallTool(ctx, "opt", map[string]any{})
	if err != nil {
		t.Fatal(err)
	}
	if !strings.Contains(string(raw), "plain") {
		t.Fatalf("optional result = %s", raw)
	}
	for _, m := range f.methods(s)[before:] {
		if strings.HasPrefix(m, "tasks/") {
			t.Fatalf("optional tool produced %s traffic", m)
		}
	}
}

func TestLegacyTaskCancelPropagates(t *testing.T) {
	f := &taskFake{}
	s := newScripted(func(method string, params map[string]any) any {
		switch method {
		case "initialize":
			return map[string]any{
				"protocolVersion": "2025-11-25",
				"serverInfo":      map[string]any{"name": "fake", "version": "1"},
				"capabilities":    map[string]any{},
			}
		case "tools/list":
			return toolList()
		case "tools/call":
			return map[string]any{"task": map[string]any{
				"taskId": "t1", "status": "working", "ttl": 60000, "pollInterval": 1,
			}}
		case "tasks/get":
			return map[string]any{"taskId": "t1", "status": "working", "ttl": 60000, "pollInterval": 1}
		case "tasks/cancel":
			f.mu.Lock()
			f.cancels++
			f.mu.Unlock()
			return map[string]any{"taskId": "t1", "status": "cancelled", "ttl": 60000, "pollInterval": 1}
		}
		return nil
	})
	c := dialLegacy(t, s)
	listFirst(t, c)
	ctx, cancel := context.WithCancel(context.Background())
	time.AfterFunc(50*time.Millisecond, cancel)
	_, err := c.CallTool(ctx, "req", map[string]any{})
	if !errors.Is(err, context.Canceled) {
		t.Fatalf("err = %v, want context.Canceled", err)
	}
	deadline := time.Now().Add(2 * time.Second)
	for time.Now().Before(deadline) {
		f.mu.Lock()
		n := f.cancels
		f.mu.Unlock()
		if n > 0 {
			return
		}
		time.Sleep(10 * time.Millisecond)
	}
	t.Fatal("context cancel never sent tasks/cancel")
}

func TestLegacyTaskInputRequiredIsNamedError(t *testing.T) {
	s := newScripted(func(method string, params map[string]any) any {
		switch method {
		case "initialize":
			return map[string]any{
				"protocolVersion": "2025-11-25",
				"serverInfo":      map[string]any{"name": "fake", "version": "1"},
				"capabilities":    map[string]any{},
			}
		case "tools/list":
			return toolList()
		case "tools/call":
			return map[string]any{"task": map[string]any{
				"taskId": "t1", "status": "working", "ttl": 60000, "pollInterval": 1,
			}}
		case "tasks/get":
			return map[string]any{"taskId": "t1", "status": "input_required", "ttl": 60000, "pollInterval": 1}
		}
		return nil
	})
	c := dialLegacy(t, s)
	listFirst(t, c)
	ctx, cancel := context.WithTimeout(context.Background(), 5*time.Second)
	defer cancel()
	_, err := c.CallTool(ctx, "req", map[string]any{})
	var ire *mcpclient.TaskInputRequiredError
	if !errors.As(err, &ire) {
		t.Fatalf("err = %v, want a TaskInputRequiredError", err)
	}
	if ire.TaskID != "t1" || ire.Tool != "req" {
		t.Fatalf("named error = %+v", ire)
	}
}

func TestLegacyTaskFaultAndMalformed(t *testing.T) {
	newSrv := func(getStatus string, callResult any) *scriptedServer {
		return newScripted(func(method string, params map[string]any) any {
			switch method {
			case "initialize":
				return map[string]any{
					"protocolVersion": "2025-11-25",
					"serverInfo":      map[string]any{"name": "fake", "version": "1"},
					"capabilities":    map[string]any{},
				}
			case "tools/list":
				return toolList()
			case "tools/call":
				return callResult
			case "tasks/get":
				return map[string]any{"taskId": "t1", "status": getStatus, "ttl": 60000, "pollInterval": 1}
			case "tasks/result":
				return nil // the fake answers -32601, standing in for the task's fault
			}
			return nil
		})
	}
	ctx, cancel := context.WithTimeout(context.Background(), 5*time.Second)
	defer cancel()

	// Fault: tasks/result's own error surfaces.
	c := dialLegacy(t, newSrv("failed", map[string]any{"task": map[string]any{
		"taskId": "t1", "status": "working", "ttl": 60000, "pollInterval": 1,
	}}))
	listFirst(t, c)
	if _, err := c.CallTool(ctx, "req", map[string]any{}); err == nil {
		t.Fatal("a failed task returned no error")
	}

	// Malformed: a task-augmented answer with no {task} handle is a
	// call error, not a hang.
	c2 := dialLegacy(t, newSrv("working", map[string]any{"content": []any{}}))
	listFirst(t, c2)
	if _, err := c2.CallTool(ctx, "req", map[string]any{}); err == nil ||
		!strings.Contains(err.Error(), "malformed") {
		t.Fatalf("err = %v, want a malformed-response error", err)
	}

	// Unknown status: also malformed, never an unbounded poll.
	c3 := dialLegacy(t, newSrv("bogus", map[string]any{"task": map[string]any{
		"taskId": "t1", "status": "working", "ttl": 60000, "pollInterval": 1,
	}}))
	listFirst(t, c3)
	if _, err := c3.CallTool(ctx, "req", map[string]any{}); err == nil ||
		!strings.Contains(err.Error(), "unknown task status") {
		t.Fatalf("err = %v, want an unknown-status error", err)
	}
}

func TestModernRequiredToolRunsAsTask(t *testing.T) {
	f := &taskFake{}
	s := newScripted(func(method string, params map[string]any) any {
		if meta, ok := params["_meta"].(map[string]any); ok {
			if caps, ok := meta[mcpclient.MetaClientCapabilities].(map[string]any); ok {
				if ext, ok := caps["extensions"].(map[string]any); ok {
					if _, ok := ext["io.modelcontextprotocol/tasks"]; ok {
						f.mu.Lock()
						f.extSeen = true
						f.mu.Unlock()
					}
				}
			}
		}
		switch method {
		case "server/discover":
			return modernDiscover(params)
		case "tools/list":
			out := toolList()
			out["resultType"] = "complete"
			return out
		case "tools/call":
			if params["name"] == "opt" {
				return map[string]any{"resultType": "complete",
					"content": []any{map[string]any{"type": "text", "text": "plain"}}}
			}
			return map[string]any{"resultType": "task", "taskId": "m1",
				"status": "working", "pollIntervalMs": 1, "ttlMs": 60000}
		case "tasks/get":
			f.mu.Lock()
			f.gets++
			n := f.gets
			f.mu.Unlock()
			if n < 2 {
				return map[string]any{"taskId": "m1", "status": "working", "pollIntervalMs": 1, "ttlMs": 60000}
			}
			return map[string]any{"taskId": "m1", "status": "completed", "pollIntervalMs": 1, "ttlMs": 60000,
				"result": map[string]any{"content": []any{map[string]any{"type": "text", "text": "modern-done"}}}}
		}
		return nil
	})
	c := dialModern(t, s, mcpclient.Options{})
	listFirst(t, c)
	ctx, cancel := context.WithTimeout(context.Background(), 5*time.Second)
	defer cancel()

	raw, err := c.CallTool(ctx, "req", map[string]any{})
	if err != nil {
		t.Fatal(err)
	}
	if !strings.Contains(string(raw), "modern-done") {
		t.Fatalf("result = %s, want the DetailedTask's inlined result", raw)
	}
	f.mu.Lock()
	ext := f.extSeen
	f.mu.Unlock()
	if !ext {
		t.Fatal("no modern request declared extensions[io.modelcontextprotocol/tasks]")
	}

	// Optional stays plain: a complete result returned as-is, and no
	// tasks/get follows it.
	f.mu.Lock()
	getsBefore := f.gets
	f.mu.Unlock()
	raw, err = c.CallTool(ctx, "opt", map[string]any{})
	if err != nil {
		t.Fatal(err)
	}
	if !strings.Contains(string(raw), "plain") {
		t.Fatalf("optional result = %s", raw)
	}
	f.mu.Lock()
	getsAfter := f.gets
	f.mu.Unlock()
	if getsAfter != getsBefore {
		t.Fatal("optional tool produced tasks/get traffic")
	}
}

func TestModernTaskFaultInputRequiredMalformed(t *testing.T) {
	newSrv := func(getResult map[string]any, callResult map[string]any) *scriptedServer {
		return newScripted(func(method string, params map[string]any) any {
			switch method {
			case "server/discover":
				return modernDiscover(params)
			case "tools/list":
				out := toolList()
				out["resultType"] = "complete"
				return out
			case "tools/call":
				return callResult
			case "tasks/get":
				return getResult
			}
			return nil
		})
	}
	handle := map[string]any{"resultType": "task", "taskId": "m1",
		"status": "working", "pollIntervalMs": 1, "ttlMs": 60000}
	ctx, cancel := context.WithTimeout(context.Background(), 5*time.Second)
	defer cancel()

	// Fault: the DetailedTask's inlined error surfaces in the client's
	// existing rpcError shape.
	c := dialModern(t, newSrv(map[string]any{
		"taskId": "m1", "status": "failed",
		"error": map[string]any{"code": -32603, "message": "boom"},
	}, handle), mcpclient.Options{})
	listFirst(t, c)
	if _, err := c.CallTool(ctx, "req", map[string]any{}); err == nil ||
		!strings.Contains(err.Error(), "boom") {
		t.Fatalf("err = %v, want the inlined fault", err)
	}

	// input_required: the distinct named error.
	c2 := dialModern(t, newSrv(map[string]any{
		"taskId": "m1", "status": "input_required",
	}, handle), mcpclient.Options{})
	listFirst(t, c2)
	_, err := c2.CallTool(ctx, "req", map[string]any{})
	var ire *mcpclient.TaskInputRequiredError
	if !errors.As(err, &ire) {
		t.Fatalf("err = %v, want a TaskInputRequiredError", err)
	}

	// Malformed: a task handle with no taskId is a call error.
	c3 := dialModern(t, newSrv(nil, map[string]any{
		"resultType": "task", "status": "working",
	}), mcpclient.Options{})
	listFirst(t, c3)
	if _, err := c3.CallTool(ctx, "req", map[string]any{}); err == nil ||
		!strings.Contains(err.Error(), "malformed") {
		t.Fatalf("err = %v, want a malformed-response error", err)
	}
}

// TestModernOptionalToolTaskAnswerIsFinished: advertising the tasks
// extension lets a server answer an OPTIONAL tool with a task past its
// eager window; the handle must be polled to the real result, not
// returned to the caller as if it were one (#1556 Codex review).
func TestModernOptionalToolTaskAnswerIsFinished(t *testing.T) {
	s := newScripted(func(method string, params map[string]any) any {
		switch method {
		case "server/discover":
			return modernDiscover(params)
		case "tools/list":
			out := toolList()
			out["resultType"] = "complete"
			return out
		case "tools/call":
			return map[string]any{"resultType": "task", "taskId": "m9",
				"status": "working", "pollIntervalMs": 1, "ttlMs": 60000}
		case "tasks/get":
			return map[string]any{"taskId": "m9", "status": "completed", "pollIntervalMs": 1, "ttlMs": 60000,
				"result": map[string]any{"content": []any{map[string]any{"type": "text", "text": "opt-done"}}}}
		}
		return nil
	})
	c := dialModern(t, s, mcpclient.Options{})
	listFirst(t, c)
	ctx, cancel := context.WithTimeout(context.Background(), 5*time.Second)
	defer cancel()
	raw, err := c.CallTool(ctx, "opt", map[string]any{})
	if err != nil {
		t.Fatal(err)
	}
	if !strings.Contains(string(raw), "opt-done") {
		t.Fatalf("result = %s, want the optional task's inlined result", raw)
	}
}

// TestSeedToolExecPopulatesRequired: a serving client that never listed
// (the pool listed on a different instance) still recognises a required
// tool once the pool seeds its snapshot from the cached catalogue.
func TestSeedToolExecPopulatesRequired(t *testing.T) {
	f := &taskFake{}
	s := newScripted(func(method string, params map[string]any) any {
		switch method {
		case "initialize":
			return map[string]any{
				"protocolVersion": "2025-11-25",
				"serverInfo":      map[string]any{"name": "fake", "version": "1"},
				"capabilities":    map[string]any{},
			}
		case "tools/call":
			if _, ok := params["task"]; !ok {
				f.mu.Lock()
				f.plainReq = true
				f.mu.Unlock()
				return map[string]any{"content": []any{map[string]any{"type": "text", "text": "plain"}}}
			}
			return map[string]any{"task": map[string]any{
				"taskId": "t1", "status": "completed", "ttl": 60000, "pollInterval": 1,
			}}
		case "tasks/result":
			return map[string]any{"content": []any{map[string]any{"type": "text", "text": "seeded-done"}}}
		}
		return nil
	})
	c := dialLegacy(t, s)
	// No listFirst: seed instead, as Pool.Call does from its cache.
	c.SeedToolExec([]mcpclient.Tool{
		{Name: "req", Execution: json.RawMessage(`{"taskSupport":"required"}`)},
	})
	ctx, cancel := context.WithTimeout(context.Background(), 5*time.Second)
	defer cancel()
	raw, err := c.CallTool(ctx, "req", map[string]any{})
	if err != nil {
		t.Fatal(err)
	}
	if !strings.Contains(string(raw), "seeded-done") {
		t.Fatalf("result = %s, want the tasks/result payload", raw)
	}
	f.mu.Lock()
	defer f.mu.Unlock()
	if f.plainReq {
		t.Fatal("the seeded required tool was called without a task field")
	}
}

// TestLegacyTaskRelayRidesTheCall: a downstream relay's _meta reaches
// the task-augmented tools/call, as it does on the plain path.
func TestLegacyTaskRelayRidesTheCall(t *testing.T) {
	var sawMeta bool
	s := newScripted(func(method string, params map[string]any) any {
		switch method {
		case "initialize":
			return map[string]any{
				"protocolVersion": "2025-11-25",
				"serverInfo":      map[string]any{"name": "fake", "version": "1"},
				"capabilities":    map[string]any{},
			}
		case "tools/list":
			return toolList()
		case "tools/call":
			if meta, ok := params["_meta"].(map[string]any); ok {
				if _, ok := meta["traceparent"]; ok {
					sawMeta = true
				}
			}
			return map[string]any{"task": map[string]any{
				"taskId": "t1", "status": "completed", "ttl": 60000, "pollInterval": 1,
			}}
		case "tasks/result":
			return map[string]any{"content": []any{map[string]any{"type": "text", "text": "relayed"}}}
		}
		return nil
	})
	c := dialLegacy(t, s)
	listFirst(t, c)
	ctx, cancel := context.WithTimeout(context.Background(), 5*time.Second)
	defer cancel()
	ctx = mcpclient.WithRelay(ctx, &mcpclient.Relay{
		Meta: map[string]json.RawMessage{"traceparent": json.RawMessage(`"00-abc"`)},
	})
	if _, err := c.CallTool(ctx, "req", map[string]any{}); err != nil {
		t.Fatal(err)
	}
	if !sawMeta {
		t.Fatal("the task-augmented call carried no relay _meta")
	}
}

// TestLegacyTaskTTLBoundsTheWait: a poll interval far beyond the task's
// TTL must not overshoot the deadline by a whole interval -- the wait
// ends at the TTL with the TTL error.
func TestLegacyTaskTTLBoundsTheWait(t *testing.T) {
	s := newScripted(func(method string, params map[string]any) any {
		switch method {
		case "initialize":
			return map[string]any{
				"protocolVersion": "2025-11-25",
				"serverInfo":      map[string]any{"name": "fake", "version": "1"},
				"capabilities":    map[string]any{},
			}
		case "tools/list":
			return toolList()
		case "tools/call":
			return map[string]any{"task": map[string]any{
				"taskId": "t1", "status": "working", "ttl": 40, "pollInterval": 60000,
			}}
		case "tasks/get":
			return map[string]any{"taskId": "t1", "status": "working", "ttl": 40, "pollInterval": 60000}
		case "tasks/cancel":
			return map[string]any{"taskId": "t1", "status": "cancelled", "ttl": 40, "pollInterval": 60000}
		}
		return nil
	})
	c := dialLegacy(t, s)
	listFirst(t, c)
	ctx, cancel := context.WithTimeout(context.Background(), 5*time.Second)
	defer cancel()
	start := time.Now()
	_, err := c.CallTool(ctx, "req", map[string]any{})
	if err == nil || !strings.Contains(err.Error(), "did not finish within its TTL") {
		t.Fatalf("err = %v, want the TTL error", err)
	}
	if elapsed := time.Since(start); elapsed > 2*time.Second {
		t.Fatalf("TTL wait took %v, want it bounded near the 40ms TTL", elapsed)
	}
}
