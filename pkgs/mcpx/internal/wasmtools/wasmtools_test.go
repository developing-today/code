package wasmtools

import (
	"context"
	_ "embed"
	"encoding/json"
	"strings"
	"testing"
)

//go:embed testdata/echo.wasm
var echoWasm []byte

//go:embed testdata/greedy.wasm
var greedyWasm []byte

func TestListAndCallWithGrantedLog(t *testing.T) {
	ctx := context.Background()
	src, err := Load(ctx, echoWasm, GrantLog)
	if err != nil {
		t.Fatalf("load echo with log grant: %v", err)
	}
	defer src.Close(ctx)

	tools, err := src.ListTools(ctx)
	if err != nil {
		t.Fatalf("list: %v", err)
	}
	if len(tools) != 1 || tools[0].Name != "echo" {
		t.Fatalf("tools = %+v", tools)
	}

	out, err := src.CallTool(ctx, "echo", map[string]any{"hello": "world"})
	if err != nil {
		t.Fatalf("call: %v", err)
	}
	var got struct {
		Tool      string          `json:"tool"`
		Arguments json.RawMessage `json:"arguments"`
	}
	if err := json.Unmarshal(out, &got); err != nil {
		t.Fatalf("parse result %s: %v", out, err)
	}
	if got.Tool != "echo" || !strings.Contains(string(got.Arguments), "world") {
		t.Fatalf("result = %s", out)
	}
	// The granted import worked: the guest logged the call via the host.
	if logs := src.Logs(); len(logs) != 1 || logs[0] != "echo" {
		t.Fatalf("logs = %v", logs)
	}
}

func TestSameModuleWithoutGrantIsDenied(t *testing.T) {
	ctx := context.Background()
	// echo.wasm imports mcpx.log; with no grant the loader must refuse it.
	if _, err := Load(ctx, echoWasm); err == nil {
		t.Fatal("echo.wasm loaded without the log grant; want instantiation failure")
	}
}

func TestUngrantedCapabilityImportIsDenied(t *testing.T) {
	ctx := context.Background()
	// greedy.wasm imports mcpx.read_file, which the host never offers —
	// not even when every grantable capability is granted.
	_, err := Load(ctx, greedyWasm, GrantLog)
	if err == nil {
		t.Fatal("greedy.wasm loaded; want instantiation failure for ungranted mcpx.read_file")
	}
	if !strings.Contains(err.Error(), "read_file") {
		t.Fatalf("error should name the missing import, got: %v", err)
	}
}

func TestAppendLogDropsOverlongLine(t *testing.T) {
	src := &Source{}
	src.appendLog([]byte("kept"))
	src.appendLog([]byte(strings.Repeat("x", maxLogLineSize+1)))

	if got := src.Logs(); len(got) != 1 || got[0] != "kept" {
		t.Fatalf("logs = %v, want [kept]", got)
	}
	if got := src.DroppedLogLines(); got != 1 {
		t.Fatalf("dropped lines = %d, want 1", got)
	}
}

func TestAppendLogEvictsOldestAtLineLimitIncludingEmptyLines(t *testing.T) {
	src := &Source{}
	src.appendLog([]byte("oldest"))
	for i := 1; i < maxLogLines; i++ {
		src.appendLog([]byte(""))
	}
	src.appendLog([]byte("newest"))

	got := src.Logs()
	if len(got) != maxLogLines {
		t.Fatalf("logs = %d lines, want %d", len(got), maxLogLines)
	}
	if got[0] != "" || got[len(got)-1] != "newest" {
		t.Fatalf("first/last retained lines = %q/%q, want oldest evicted and newest retained", got[0], got[len(got)-1])
	}
	if dropped := src.DroppedLogLines(); dropped != 1 {
		t.Fatalf("dropped lines = %d, want 1", dropped)
	}
}

func TestAppendLogEvictsOldestUntilByteLimitFits(t *testing.T) {
	src := &Source{}
	line := strings.Repeat("x", 1<<10)
	src.appendLog([]byte("oldest" + strings.Repeat("x", (1<<10)-len("oldest"))))
	for i := 1; i < maxLogLines; i++ {
		src.appendLog([]byte(line))
	}
	newest := strings.Repeat("y", maxLogLineSize)
	src.appendLog([]byte(newest))

	got := src.Logs()
	const evicted = maxLogLineSize / (1 << 10)
	if len(got) != maxLogLines-evicted+1 {
		t.Fatalf("logs = %d lines, want %d", len(got), maxLogLines-evicted+1)
	}
	if got[0] != line || got[len(got)-1] != newest {
		t.Fatalf("oldest/newest retained lines = %d/%d bytes, want %d/%d", len(got[0]), len(got[len(got)-1]), len(line), len(newest))
	}
	if dropped := src.DroppedLogLines(); dropped != evicted {
		t.Fatalf("dropped lines = %d, want %d", dropped, evicted)
	}
	if src.logBytes != maxLogBytes {
		t.Fatalf("retained bytes = %d, want %d", src.logBytes, maxLogBytes)
	}
}

func TestLogsIsDefensiveAndNonDraining(t *testing.T) {
	src := &Source{}
	src.appendLog([]byte("first"))
	src.appendLog([]byte("second"))

	first := src.Logs()
	first[0] = "mutated"
	second := src.Logs()
	if len(second) != 2 || second[0] != "first" || second[1] != "second" {
		t.Fatalf("second snapshot = %v, want [first second]", second)
	}
	if dropped := src.DroppedLogLines(); dropped != 0 {
		t.Fatalf("dropped lines = %d, want 0", dropped)
	}
}
