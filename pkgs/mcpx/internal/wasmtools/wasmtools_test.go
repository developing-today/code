package wasmtools_test

import (
	"context"
	_ "embed"
	"encoding/json"
	"strings"
	"testing"

	"github.com/developing-today/code/pkgs/mcpx/internal/wasmtools"
)

//go:embed testdata/echo.wasm
var echoWasm []byte

//go:embed testdata/greedy.wasm
var greedyWasm []byte

func TestListAndCallWithGrantedLog(t *testing.T) {
	ctx := context.Background()
	src, err := wasmtools.Load(ctx, echoWasm, wasmtools.GrantLog)
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
	if _, err := wasmtools.Load(ctx, echoWasm); err == nil {
		t.Fatal("echo.wasm loaded without the log grant; want instantiation failure")
	}
}

func TestUngrantedCapabilityImportIsDenied(t *testing.T) {
	ctx := context.Background()
	// greedy.wasm imports mcpx.read_file, which the host never offers —
	// not even when every grantable capability is granted.
	_, err := wasmtools.Load(ctx, greedyWasm, wasmtools.GrantLog)
	if err == nil {
		t.Fatal("greedy.wasm loaded; want instantiation failure for ungranted mcpx.read_file")
	}
	if !strings.Contains(err.Error(), "read_file") {
		t.Fatalf("error should name the missing import, got: %v", err)
	}
}
