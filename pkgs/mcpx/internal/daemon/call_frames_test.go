package daemon

import (
	"bytes"
	"context"
	"encoding/json"
	"net/http"
	"net/http/httptest"
	"os"
	"path/filepath"
	"strings"
	"testing"
	"time"

	"github.com/developing-today/code/pkgs/mcpx/internal/config"
	"github.com/developing-today/code/pkgs/mcpx/internal/execsvc"
	"github.com/developing-today/code/pkgs/mcpx/internal/logging"
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

func TestReportCallFrameWritesPayloadFreeLedgerWithoutRun(t *testing.T) {
	dir := t.TempDir()
	ledger, err := logging.NewFileSink(logging.FileOptions{Dir: dir})
	if err != nil {
		t.Fatal(err)
	}
	defer func() { _ = ledger.Close() }()
	s := &Server{callLedger: ledger}
	start := time.Now().Add(-25 * time.Millisecond)
	s.reportCallFrame(context.Background(), "srv", "tool", "session-7", map[string]any{"secret": "do-not-write"}, start,
		json.RawMessage(`{"isError":true,"content":[{"text":"private error"}]}`), nil)
	s.reportCallFrame(context.Background(), "srv", "ok", "", map[string]any{}, time.Now(), json.RawMessage(`{"ok":true}`), nil)

	data, err := os.ReadFile(filepath.Join(dir, "mcpx-"+time.Now().Format("2006-01-02")+".jsonl"))
	if err != nil {
		t.Fatal(err)
	}
	lines := strings.Split(strings.TrimSpace(string(data)), "\n")
	if len(lines) != 2 {
		t.Fatalf("ledger has %d records, want exactly 2: %s", len(lines), data)
	}
	var failed, succeeded map[string]any
	if err := json.Unmarshal([]byte(lines[0]), &failed); err != nil {
		t.Fatal(err)
	}
	if err := json.Unmarshal([]byte(lines[1]), &succeeded); err != nil {
		t.Fatal(err)
	}
	if failed["server"] != "srv" || failed["tool"] != "tool" || failed["outcome"] != "error" || failed["msg"] != "tool_call" || failed["caller_session"] != "session-7" {
		t.Fatalf("error ledger row = %v", failed)
	}
	if _, ok := failed["duration_ms"]; !ok || failed["ts"] == nil {
		t.Fatalf("error ledger row lacks timestamp or duration: %v", failed)
	}
	if succeeded["server"] != "srv" || succeeded["tool"] != "ok" || succeeded["outcome"] != "success" {
		t.Fatalf("success ledger row = %v", succeeded)
	}
	if strings.Contains(string(data), "secret") || strings.Contains(string(data), "private error") {
		t.Fatalf("ledger leaked call payload: %s", data)
	}
}

func TestPruneCallLedgerKeepsEightCalendarDays(t *testing.T) {
	dir := t.TempDir()
	now := time.Date(2026, time.November, 18, 12, 0, 0, 0, time.UTC)
	for _, name := range []string{
		"mcpx-2026-11-10.jsonl",
		"mcpx-2026-11-11.jsonl",
		"mcpx-2026-11-11-120000.jsonl",
		"other-2026-11-01.jsonl",
	} {
		if err := os.WriteFile(filepath.Join(dir, name), []byte("{}\n"), 0o600); err != nil {
			t.Fatal(err)
		}
	}

	pruneCallLedger(dir, now)
	if _, err := os.Stat(filepath.Join(dir, "mcpx-2026-11-10.jsonl")); !os.IsNotExist(err) {
		t.Fatalf("expired daily ledger file was not removed: %v", err)
	}
	for _, name := range []string{"mcpx-2026-11-11.jsonl", "mcpx-2026-11-11-120000.jsonl", "other-2026-11-01.jsonl"} {
		if _, err := os.Stat(filepath.Join(dir, name)); err != nil {
			t.Fatalf("kept ledger/unrelated file %s missing: %v", name, err)
		}
	}
}

func TestReportCallFrameMarksIsErrorResultsFailed(t *testing.T) {
	s := &Server{reg: &Registry{}}
	frames := collectFrames(s, "run-1")
	ctx := withRun(context.Background(), "run-1")

	s.reportCallFrame(ctx, "srv", "tool", "", map[string]any{}, time.Now(),
		json.RawMessage(`{"content":[{"type":"text","text":"no"}],"isError":true}`), nil)
	if len(*frames) != 1 {
		t.Fatalf("frames = %v, want 1", *frames)
	}
	c := (*frames)[0].Call
	if c == nil || c.OK || !strings.Contains(c.Error, "isError") {
		t.Fatalf("isError result must be a failed call frame, got %+v", c)
	}

	s.reportCallFrame(ctx, "srv", "tool", "", map[string]any{}, time.Now(),
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

	s.reportCallFrame(ctx, "b", "x", "", map[string]any{}, time.Now(), nil, nil)
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
	s2.reportCallFrame(withRun(context.Background(), "run-3"), "ns", "x", "",
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
