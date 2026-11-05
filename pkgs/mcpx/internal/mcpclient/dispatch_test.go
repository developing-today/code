package mcpclient

import (
	"context"
	"encoding/json"
	"net/http"
	"net/http/httptest"
	"sync/atomic"
	"testing"
)

func TestHTTPTransportReportsDispatchedToolCall(t *testing.T) {
	server := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		w.Header().Set("Content-Type", "application/json")
		_, _ = w.Write([]byte(`{"jsonrpc":"2.0","id":1,"result":{}}`))
	}))
	defer server.Close()
	transport, err := NewHTTP(HTTPOptions{URL: server.URL})
	if err != nil {
		t.Fatal(err)
	}
	defer transport.Close()

	var dispatched atomic.Bool
	ctx := WithToolCallDispatchHook(context.Background(), func() { dispatched.Store(true) })
	frame := []byte(`{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"create_issue","arguments":{"title":"private-argument"}}}`)
	if err := transport.Send(ctx, frame); err != nil {
		t.Fatal(err)
	}
	if !dispatched.Load() {
		t.Fatal("successful tools/call write did not mark the request dispatched")
	}
}

func TestHTTPTransportDoesNotMarkPreDispatchFailure(t *testing.T) {
	var dispatched atomic.Bool
	transport := &HTTPTransport{url: "://invalid", hc: &http.Client{}, closed: make(chan struct{})}
	ctx := WithToolCallDispatchHook(context.Background(), func() { dispatched.Store(true) })
	err := transport.Send(ctx, []byte(`{"jsonrpc":"2.0","id":1,"method":"tools/call"}`))
	if err == nil {
		t.Fatal("Send() unexpectedly accepted an invalid URL")
	}
	if dispatched.Load() {
		t.Fatal("request was marked dispatched before any bytes were written")
	}
}

// A conclusive upstream answer -- the server received the call and rejected
// it -- means the write's outcome is known, so the unknown-outcome ledger
// must not record it. Only a response that never arrived is unknown.
func TestIsConclusiveUpstreamError(t *testing.T) {
	tests := []struct {
		name string
		err  error
		want bool
	}{
		{
			name: "invalid params rejection",
			err:  &rpcError{Code: -32602, Message: "invalid params"},
			want: true,
		},
		{
			name: "internal error answer",
			err:  &rpcError{Code: -32603, Message: "the tool failed"},
			want: true,
		},
		{
			name: "stream lost",
			err: &rpcError{Code: codeStreamLost, Message: "the server closed the response stream before answering",
				Data: json.RawMessage(`{"mcpxStreamLost":true}`)},
			want: false,
		},
		{
			name: "server sent -32000 itself",
			err:  &rpcError{Code: -32000, Message: "connection closed"},
			want: true,
		},
		{
			name: "cancellation",
			err:  context.Canceled,
			want: false,
		},
		{
			name: "deadline",
			err:  context.DeadlineExceeded,
			want: false,
		},
	}
	for _, tc := range tests {
		t.Run(tc.name, func(t *testing.T) {
			if got := IsConclusiveUpstreamError(tc.err); got != tc.want {
				t.Fatalf("IsConclusiveUpstreamError(%v) = %v, want %v", tc.err, got, tc.want)
			}
		})
	}
}
