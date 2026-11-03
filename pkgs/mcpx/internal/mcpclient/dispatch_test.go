package mcpclient

import (
	"context"
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
