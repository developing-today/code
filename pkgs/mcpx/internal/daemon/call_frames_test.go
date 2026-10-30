package daemon

import (
	"bytes"
	"context"
	"encoding/json"
	"net/http"
	"net/http/httptest"
	"strings"
	"testing"
	"time"

	"github.com/developing-today/code/pkgs/mcpx/internal/config"
	"github.com/developing-today/code/pkgs/mcpx/internal/execsvc"
)

// collectFrames registers a sink for run on s and returns the frames it
// received, in arrival order.
func collectFrames(s *Server, run string) *[]execsvc.Frame {
	frames := &[]execsvc.Frame{}
	s.callRegistry().Register(run, func(f execsvc.Frame) {
		*frames = append(*frames, f)
	})
	return frames
}

func TestReportCallFrameMarksIsErrorResultsFailed(t *testing.T) {
	s := &Server{reg: &Registry{}}
	frames := collectFrames(s, "run-1")
	ctx := withRun(context.Background(), "run-1")

	s.reportCallFrame(ctx, "srv", "tool", map[string]any{}, time.Now(),
		json.RawMessage(`{"content":[{"type":"text","text":"no"}],"isError":true}`), nil)
	if len(*frames) != 1 {
		t.Fatalf("frames = %v, want 1", *frames)
	}
	c := (*frames)[0].Call
	if c == nil || c.OK || !strings.Contains(c.Error, "isError") {
		t.Fatalf("isError result must be a failed call frame, got %+v", c)
	}

	s.reportCallFrame(ctx, "srv", "tool", map[string]any{}, time.Now(),
		json.RawMessage(`{"content":[],"isError":false}`), nil)
	if len(*frames) != 2 || !(*frames)[1].Call.OK {
		t.Fatalf("a clean result must stay ok, got %+v", *frames)
	}
}

// Two servers can collide: server a's namespace is server b's name.
// Dispatch resolves the exact name first (Registry.Pool), so the call
// frame's destructiveHint must come from b's tool, not a's.
func TestReportCallFrameResolvesExactServerBeforeNamespace(t *testing.T) {
	reg := &Registry{testTools: []ToolInfo{
		{Server: "a", Namespace: "b", Tool: "x", Annotations: json.RawMessage(`{"readOnlyHint":true}`)},
		{Server: "b", Namespace: "c", Tool: "x", Annotations: json.RawMessage(`{"destructiveHint":true}`)},
	}}
	s := &Server{reg: reg}
	frames := collectFrames(s, "run-2")
	ctx := withRun(context.Background(), "run-2")

	s.reportCallFrame(ctx, "b", "x", map[string]any{}, time.Now(), nil, nil)
	if len(*frames) != 1 {
		t.Fatalf("frames = %v, want 1", *frames)
	}
	c := (*frames)[0].Call
	if c == nil || c.DestructiveHint == nil || !*c.DestructiveHint {
		t.Fatalf("call to server b must carry b's destructiveHint=true, got %+v", c)
	}

	// A caller naming a only by its namespace still gets a's annotations.
	reg2 := &Registry{testTools: []ToolInfo{
		{Server: "a", Namespace: "ns", Tool: "x", Annotations: json.RawMessage(`{"readOnlyHint":true}`)},
	}}
	s2 := &Server{reg: reg2}
	frames2 := collectFrames(s2, "run-3")
	s2.reportCallFrame(withRun(context.Background(), "run-3"), "ns", "x",
		map[string]any{}, time.Now(), nil, nil)
	if len(*frames2) != 1 {
		t.Fatalf("frames = %v, want 1", *frames2)
	}
	c2 := (*frames2)[0].Call
	if c2 == nil || c2.DestructiveHint == nil || *c2.DestructiveHint {
		t.Fatalf("call via namespace ns must carry a's destructiveHint=false, got %+v", c2)
	}
}

// The input-report path returns from handleCall before its reporting
// runs, so handleCallReporting reports the completed call itself.
func TestHandleCallReportingEmitsCallFrame(t *testing.T) {
	srv, err := NewServer(handoffOptions(handoffPaths(t),
		&config.Config{MCPServers: map[string]*config.Server{}}, "v"))
	if err != nil {
		t.Fatal(err)
	}
	frames := collectFrames(srv, "run-9")
	req := httptest.NewRequest(http.MethodPost, "/v1/call", nil)
	req.Header.Set("X-Mcpx-Run", "run-9")
	w := httptest.NewRecorder()

	srv.handleCallReporting(w, req, callReq{Server: "nope", Tool: "x"},
		config.CallContext{}, map[string]any{})

	if w.Code != http.StatusBadRequest {
		t.Fatalf("unknown server: status %d, want 400 (body %s)", w.Code, w.Body.String())
	}
	if len(*frames) != 1 {
		t.Fatalf("frames = %v, want the one completed call reported", *frames)
	}
	c := (*frames)[0].Call
	if c == nil || c.Server != "nope" || c.Tool != "x" || c.OK {
		t.Fatalf("call frame = %+v, want a failed nope.x frame", c)
	}
}

// Batch calls go through handleBatch, not the single-call handlers, so
// it must report each completed item itself: a script's batch(...)
// calls belong in its exec stream like any other call.
func TestHandleBatchEmitsCallFrames(t *testing.T) {
	srv, err := NewServer(handoffOptions(handoffPaths(t),
		&config.Config{MCPServers: map[string]*config.Server{}}, "v"))
	if err != nil {
		t.Fatal(err)
	}
	frames := collectFrames(srv, "run-10")
	body := []byte(`{"calls":[{"server":"nope","tool":"x","args":{}}]}`)
	req := httptest.NewRequest(http.MethodPost, "/v1/batch", bytes.NewReader(body))
	req.Header.Set("X-Mcpx-Run", "run-10")
	w := httptest.NewRecorder()

	srv.handleBatch(w, req)

	if w.Code != http.StatusOK {
		t.Fatalf("batch: status %d (body %s)", w.Code, w.Body.String())
	}
	if len(*frames) != 1 {
		t.Fatalf("frames = %v, want the one completed batch item reported", *frames)
	}
	c := (*frames)[0].Call
	if c == nil || c.Server != "nope" || c.Tool != "x" || c.OK {
		t.Fatalf("call frame = %+v, want a failed nope.x frame", c)
	}
}
