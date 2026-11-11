package mcpclient

import (
	"context"
	"encoding/json"
	"errors"
	"net/http/httptrace"
)

type toolCallDispatchKey struct{}

// WithToolCallDispatchHook installs a callback invoked only after a tools/call
// request has been completely written to its transport.
func WithToolCallDispatchHook(ctx context.Context, hook func()) context.Context {
	if hook == nil {
		return ctx
	}
	return context.WithValue(ctx, toolCallDispatchKey{}, hook)
}

// NoteToolCallDispatched fires the dispatch hook installed by
// WithToolCallDispatchHook, for a call that is being dispatched by a
// path with no transport frame to observe -- a script-backed tool view
// (#1933 slice 2) executes in-process, so the moment its program
// starts is its dispatch. Callers invoke it exactly when execution
// begins, never during validation or rendering, so a hook observer
// can distinguish "never ran" from "ran with an unknown outcome".
func NoteToolCallDispatched(ctx context.Context) {
	if hook, ok := ctx.Value(toolCallDispatchKey{}).(func()); ok {
		hook()
	}
}

func isToolCallFrame(msg []byte) bool {
	var frame struct {
		Method string `json:"method"`
	}
	return json.Unmarshal(msg, &frame) == nil && frame.Method == "tools/call"
}

func markToolCallDispatched(ctx context.Context, msg []byte) {
	if !isToolCallFrame(msg) {
		return
	}
	if hook, ok := ctx.Value(toolCallDispatchKey{}).(func()); ok {
		hook()
	}
}

func httpDispatchContext(ctx context.Context, msg []byte) context.Context {
	if !isToolCallFrame(msg) {
		return ctx
	}
	trace := &httptrace.ClientTrace{
		WroteRequest: func(info httptrace.WroteRequestInfo) {
			if info.Err == nil {
				markToolCallDispatched(ctx, msg)
			}
		},
	}
	return httptrace.WithClientTrace(ctx, trace)
}

// IsConclusiveUpstreamError reports whether err is a JSON-RPC error the
// upstream answered with, as opposed to a lost response. A conclusive
// rejection (for example -32602 for invalid parameters) means the outcome
// is known -- the write did not take effect -- so it must never be recorded
// as an unknown outcome. A stream-lost error (-32000 with the
// mcpxStreamLost marker) is the opposite: the response never arrived, so
// the outcome is unknown.
func IsConclusiveUpstreamError(err error) bool {
	var re *rpcError
	if !errors.As(err, &re) {
		return false
	}
	return !isStreamLost(err)
}
