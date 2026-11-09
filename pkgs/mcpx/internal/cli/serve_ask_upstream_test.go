package cli

import (
	"context"
	"encoding/json"
	"net/http"
	"net/http/httptest"
	"testing"
	"time"

	"github.com/developing-today/code/pkgs/mcpx/internal/daemon"
	"github.com/developing-today/code/pkgs/mcpx/internal/mcpserver"
)

// Oracle: the #2434 Phase 1 parity contract — an upstream error that
// reaches the client through the gateway (the ask path) must arrive
// with the same code, message and data as the pass-through error
// itself; a daemon reply without the structured field (the older
// shape) still surfaces through the text fallback.

func pollAgainst(t *testing.T, reply string) mcpserver.Outcome {
	t.Helper()
	daemonSrv := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		_, _ = w.Write([]byte(reply))
	}))
	defer daemonSrv.Close()
	app := &App{client: NewClientAt(daemon.Paths{}, "", daemonSrv.URL), connected: true}
	out, err := daemonAsker{app: app}.Poll(context.Background(), "call-1", time.Millisecond)
	if err != nil {
		t.Fatal(err)
	}
	return out
}

func TestAskPollPrefersStructuredUpstreamError(t *testing.T) {
	reply := `{"done":true,"error":"mcp error -32021: missing",` +
		`"upstream":{"code":-32021,"message":"missing",` +
		`"data":{"requiredCapabilities":{"sampling":{}}}}}`
	out := pollAgainst(t, reply)
	if !out.IsError {
		t.Fatal("a failed upstream call is an error outcome")
	}
	if out.Upstream == nil || out.Upstream.Code != -32021 ||
		out.Upstream.Message != "missing" ||
		string(out.Upstream.Data) != `{"requiredCapabilities":{"sampling":{}}}` {
		t.Fatalf("upstream = %+v, want the structured error carried whole", out.Upstream)
	}
	// The -32021 protocol-error classification uses the same structured
	// error, with its data intact and no serve_ask special case.
	if out.Err == nil || out.Err.Error() != "missing" {
		t.Fatalf("Err = %v, want the structured -32021", out.Err)
	}
}

func TestAskPollStructuredErrorMatchesPassThrough(t *testing.T) {
	// The pass-through shape: the same error as a daemon CallErrorBody
	// carries it, which relayedFault turns into the client-facing error.
	body, _ := json.Marshal(daemon.CallErrorBody{Error: "mcp error -32021",
		Upstream: &daemon.UpstreamError{Code: -32021, Message: "missing",
			Data: json.RawMessage(`{"requiredCapabilities":{"sampling":{}}}`)}})
	var parsed daemon.CallErrorBody
	if err := json.Unmarshal(body, &parsed); err != nil {
		t.Fatal(err)
	}
	reply := `{"done":true,"error":"mcp error -32021: missing",` +
		`"upstream":{"code":-32021,"message":"missing",` +
		`"data":{"requiredCapabilities":{"sampling":{}}}}}`
	out := pollAgainst(t, reply)
	if out.Upstream == nil || out.Upstream.Code != parsed.Upstream.Code ||
		out.Upstream.Message != parsed.Upstream.Message ||
		string(out.Upstream.Data) != string(parsed.Upstream.Data) {
		t.Fatalf("gateway error %+v != pass-through %+v", out.Upstream, parsed.Upstream)
	}
}

func TestAskPollFallsBackToTextForOlderDaemonReply(t *testing.T) {
	// No "upstream" field: the pre-Phase-1 reply shape.
	reply := `{"done":true,"error":"mcp error -32603: boom"}`
	out := pollAgainst(t, reply)
	if !out.IsError {
		t.Fatal("a failed upstream call is an error outcome")
	}
	if out.Upstream == nil || out.Upstream.Code != -32603 ||
		out.Upstream.Message != "boom" || len(out.Upstream.Data) != 0 {
		t.Fatalf("upstream = %+v, want the text-parsed fallback", out.Upstream)
	}
}
