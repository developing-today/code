// Package wasmtools is the #1970 spike: a WebAssembly module hosted by
// wazero as an mcpx tool source (Approach B — a core-WASM .wasm module
// as a tool source).
//
// Capability model: the host grants imports, the guest can reach
// nothing else. This package instantiates NO WASI at all, so a guest
// gets no filesystem, no network, no clock and no environment by
// default; the only imports that can exist are the ones named in the
// grant set passed to Load (today: "log"). A guest that imports
// anything ungranted fails to instantiate — deny-by-default is a
// property of the loader, not a check inside the guest.
//
// Guest ABI (core WASM, shaped so a component-model ABI can replace it
// underneath later, per #1970 Approach C):
//
//	exports: memory
//	         mcpx_input_ptr() i32   — host writes [name][argsJSON] here
//	         mcpx_output_ptr() i32  — guest writes result JSON here
//	         mcpx_list_tools() i32  — returns byte length of the JSON
//	                                  tool list ([mcpclient.Tool])
//	         mcpx_call_tool(nameLen, argsLen i32) i32
//	                                — returns byte length of the JSON
//	                                  result for that call
//	imports: "mcpx"."log"(ptr, len) — only when "log" is granted
//
// The method shapes deliberately mirror mcpclient.Client
// (ListTools(ctx) ([]Tool, error); CallTool(ctx, name, args)
// (json.RawMessage, error), internal/mcpclient/client.go) so a WASM
// source can sit behind the same surface as the stdio/HTTP transports
// once the tool-source interface tracked by #1885 (child #83) lands.
// Today there is no such registration point — upstreams are concrete
// transports in internal/mcpclient and internal/adapter — so this
// package is a standalone spike, not a fifth composition path.
package wasmtools

import (
	"context"
	"encoding/json"
	"fmt"

	"github.com/tetratelabs/wazero"
	"github.com/tetratelabs/wazero/api"

	"github.com/dezren39/mcpx/internal/mcpclient"
)

// GrantLog is the host capability that lets a guest write one log line
// per call through the host (collected on Source.Logs).
const GrantLog = "log"

const (
	inputPtrFn  = "mcpx_input_ptr"
	outputPtrFn = "mcpx_output_ptr"
	listFn      = "mcpx_list_tools"
	callFn      = "mcpx_call_tool"
	maxMsg      = 1 << 20
)

// Source is one instantiated WASM tool source.
type Source struct {
	rt   wazero.Runtime
	mod  api.Module
	logs []string
}

// Load instantiates src (a core-WASM binary) granting exactly the
// named host capabilities. Any import the guest declares beyond the
// grant set — WASI included — makes Load fail.
func Load(ctx context.Context, src []byte, grants ...string) (*Source, error) {
	s := &Source{}
	rt := wazero.NewRuntime(ctx)
	s.rt = rt

	granted := map[string]bool{}
	for _, g := range grants {
		granted[g] = true
	}
	host := rt.NewHostModuleBuilder("mcpx")
	if granted[GrantLog] {
		host.NewFunctionBuilder().WithFunc(func(_ context.Context, m api.Module, ptr, length uint32) {
			b, ok := m.Memory().Read(ptr, length)
			if ok {
				s.logs = append(s.logs, string(b))
			}
		}).Export("log")
	}
	if _, err := host.Instantiate(ctx); err != nil {
		rt.Close(ctx)
		return nil, fmt.Errorf("wasmtools: instantiate host: %w", err)
	}

	mod, err := rt.Instantiate(ctx, src)
	if err != nil {
		rt.Close(ctx)
		return nil, fmt.Errorf("wasmtools: instantiate guest (ungranted import?): %w", err)
	}
	s.mod = mod
	for _, fn := range []string{inputPtrFn, outputPtrFn, listFn, callFn} {
		if mod.ExportedFunction(fn) == nil {
			rt.Close(ctx)
			return nil, fmt.Errorf("wasmtools: guest does not export %s", fn)
		}
	}
	return s, nil
}

// Close releases the runtime and every module in it.
func (s *Source) Close(ctx context.Context) error { return s.rt.Close(ctx) }

// Logs returns the lines the guest wrote through the granted "log"
// capability, in order.
func (s *Source) Logs() []string { return append([]string(nil), s.logs...) }

func (s *Source) readOutput(ctx context.Context, length uint64) ([]byte, error) {
	if length > maxMsg {
		return nil, fmt.Errorf("wasmtools: guest returned %d bytes, over the %d cap", length, maxMsg)
	}
	res, err := s.mod.ExportedFunction(outputPtrFn).Call(ctx)
	if err != nil {
		return nil, err
	}
	b, ok := s.mod.Memory().Read(uint32(res[0]), uint32(length))
	if !ok {
		return nil, fmt.Errorf("wasmtools: guest output out of memory bounds")
	}
	return append([]byte(nil), b...), nil
}

// ListTools returns the guest's tool list.
func (s *Source) ListTools(ctx context.Context) ([]mcpclient.Tool, error) {
	res, err := s.mod.ExportedFunction(listFn).Call(ctx)
	if err != nil {
		return nil, fmt.Errorf("wasmtools: list_tools: %w", err)
	}
	raw, err := s.readOutput(ctx, res[0])
	if err != nil {
		return nil, err
	}
	var tools []mcpclient.Tool
	if err := json.Unmarshal(raw, &tools); err != nil {
		return nil, fmt.Errorf("wasmtools: parse tool list: %w", err)
	}
	return tools, nil
}

// CallTool invokes one tool on the guest and returns its raw JSON result.
func (s *Source) CallTool(ctx context.Context, name string, args any) (json.RawMessage, error) {
	argsJSON, err := json.Marshal(args)
	if err != nil {
		return nil, err
	}
	res, err := s.mod.ExportedFunction(inputPtrFn).Call(ctx)
	if err != nil {
		return nil, err
	}
	ptr := uint32(res[0])
	mem := s.mod.Memory()
	if !mem.Write(ptr, []byte(name)) || !mem.Write(ptr+uint32(len(name)), argsJSON) {
		return nil, fmt.Errorf("wasmtools: call input out of memory bounds")
	}
	out, err := s.mod.ExportedFunction(callFn).Call(ctx, uint64(uint32(len(name))), uint64(uint32(len(argsJSON))))
	if err != nil {
		return nil, fmt.Errorf("wasmtools: call_tool: %w", err)
	}
	return s.readOutput(ctx, out[0])
}
