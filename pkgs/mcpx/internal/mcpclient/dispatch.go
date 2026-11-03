package mcpclient

import (
	"context"
	"encoding/json"
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
