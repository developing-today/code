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
