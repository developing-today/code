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

func TestLoadRejectsGuestWithoutExportedMemory(t *testing.T) {
	_, err := Load(context.Background(), noMemoryGuest(false))
	if err == nil || !strings.Contains(err.Error(), "guest does not export memory") {
		t.Fatalf("Load error = %v, want an error naming missing memory", err)
	}
}

func TestLoadRejectsGuestWithoutExportedMemoryWhenStartCallsGrantedLog(t *testing.T) {
	_, err := Load(context.Background(), noMemoryGuest(true), GrantLog)
	if err == nil || !strings.Contains(err.Error(), "guest does not export memory") {
		t.Fatalf("Load error = %v, want an error naming missing memory", err)
	}
}

// noMemoryGuest is a tiny test fixture equivalent to a core-WASM module with
// the four required mcpx functions but no exported memory. In the second form,
// the guest imports mcpx.log and calls it from its start function, before Load
// can inspect the module. The exports all return zero; their behavior is
// irrelevant because Load must reject both modules first.
func noMemoryGuest(startCallsLog bool) []byte {
	module := []byte{0x00, 0x61, 0x73, 0x6d, 0x01, 0x00, 0x00, 0x00}
	appendSection := func(id byte, payload []byte) {
		module = append(module, id)
		module = appendWasmU32(module, uint32(len(payload)))
		module = append(module, payload...)
	}
	appendName := func(dst []byte, name string) []byte {
		dst = appendWasmU32(dst, uint32(len(name)))
		return append(dst, name...)
	}

	if startCallsLog {
		// (i32, i32) -> (), () -> i32, (i32, i32) -> i32, () -> ()
		appendSection(1, []byte{
			4,
			0x60, 2, 0x7f, 0x7f, 0,
			0x60, 0, 1, 0x7f,
			0x60, 2, 0x7f, 0x7f, 1, 0x7f,
			0x60, 0, 0,
		})
		importSection := []byte{1}
		importSection = appendName(importSection, "mcpx")
		importSection = appendName(importSection, "log")
		importSection = append(importSection, 0, 0) // function, type 0
		appendSection(2, importSection)
		appendSection(3, []byte{5, 1, 1, 1, 2, 3})
	} else {
		// () -> i32, (i32, i32) -> i32
		appendSection(1, []byte{
			2,
			0x60, 0, 1, 0x7f,
			0x60, 2, 0x7f, 0x7f, 1, 0x7f,
		})
		appendSection(3, []byte{4, 0, 0, 0, 1})
	}

	exports := []byte{4}
	for i, name := range []string{"mcpx_input_ptr", "mcpx_output_ptr", "mcpx_list_tools", "mcpx_call_tool"} {
		exports = appendName(exports, name)
		exports = append(exports, 0, byte(i)) // function export; import shifts indices below
		if startCallsLog {
			exports[len(exports)-1]++
		}
	}
	appendSection(7, exports)

	body := []byte{4, 0, 0x41, 0, 0x0b} // locals=0; i32.const 0; end
	code := []byte{4}
	if startCallsLog {
		code[0] = 5
	}
	for range 4 {
		code = append(code, body...)
	}
	if startCallsLog {
		startBody := []byte{8, 0, 0x41, 0, 0x41, 0, 0x10, 0, 0x0b} // locals=0; log(0, 0); end
		code = append(code, startBody...)
		appendSection(8, []byte{5}) // start function index, after imported log + four exports
	}
	appendSection(10, code)
	return module
}

func appendWasmU32(dst []byte, value uint32) []byte {
	for value >= 0x80 {
		dst = append(dst, byte(value)|0x80)
		value >>= 7
	}
	return append(dst, byte(value))
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
