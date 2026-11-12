package cli

import (
	"context"
	"encoding/json"
	"net/http"
	"net/http/httptest"
	"strings"
	"testing"
	"time"

	"github.com/developing-today/code/pkgs/mcpx/internal/daemon"
	"github.com/developing-today/code/pkgs/mcpx/internal/mcpserver"
)

// pollWith serves one fixed ask-poll reply from a fake daemon and
// returns the Outcome Poll makes of it.
func pollWith(t *testing.T, reply string) mcpserver.Outcome {
	t.Helper()
	daemonSrv := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		if strings.HasPrefix(r.URL.Path, "/v1/ask/") {
			_, _ = w.Write([]byte(reply))
			return
		}
		http.Error(w, `{"error":"unexpected"}`, http.StatusNotFound)
	}))
	defer daemonSrv.Close()
	app := &App{client: NewClientAt(daemon.Paths{}, "", daemonSrv.URL), connected: true}
	out, err := daemonAsker{app: app}.Poll(context.Background(), "call-1", time.Millisecond)
	if err != nil {
		t.Fatalf("Poll: %v", err)
	}
	return out
}

// #2435 (a): when the daemon's reply carries the structured upstream
// error, the ask path relays it verbatim -- code, message and data --
// instead of reconstructing what it can from the rendered text (the
// heuristic cannot recover data at all).
func TestAskPollPrefersTheStructuredUpstreamError(t *testing.T) {
	out := pollWith(t, `{"done":true,`+
		`"error":"mcp error -32602: the text rendering of the failure",`+
		`"upstream":{"code":-32602,"message":"structured message","data":{"hint":"verbatim"}}}`)
	if !out.IsError {
		t.Fatalf("the failed call is not a tool error: %+v", out)
	}
	if out.Upstream == nil {
		t.Fatal("no upstream error on the outcome")
	}
	if out.Upstream.Code != -32602 || out.Upstream.Message != "structured message" {
		t.Fatalf("upstream not verbatim: %+v", out.Upstream)
	}
	if string(out.Upstream.Data) != `{"hint":"verbatim"}` {
		t.Fatalf("upstream data lost or altered: %s", out.Upstream.Data)
	}
}

// #2435 (a): the -32021 classification keeps the structured data --
// requiredCapabilities is what the client acts on.
func TestAskPollMissingCapabilityKeepsItsData(t *testing.T) {
	out := pollWith(t, `{"done":true,"error":"call failed",`+
		`"upstream":{"code":-32021,"message":"MissingRequiredClientCapabilityError",`+
		`"data":{"requiredCapabilities":{"sampling":{}}}}}`)
	up, ok := out.Err.(*mcpserver.UpstreamError)
	if !ok {
		t.Fatalf("Err = %T, want the structured UpstreamError", out.Err)
	}
	var data struct {
		Required map[string]any `json:"requiredCapabilities"`
	}
	if err := json.Unmarshal(up.Data, &data); err != nil || data.Required == nil {
		t.Fatalf("-32021 data lost: %s (%v)", up.Data, err)
	}
	if out.Upstream == nil || string(out.Upstream.Data) != string(up.Data) {
		t.Fatalf("outcome upstream not verbatim: %+v", out.Upstream)
	}
}

// #2435 (a): a reply without the structured field falls back to the
// text heuristic, exactly as before.
func TestAskPollFallsBackToTheTextHeuristic(t *testing.T) {
	out := pollWith(t, `{"done":true,"error":"mcp error -32602: no such tool"}`)
	if out.Upstream == nil {
		t.Fatal("no upstream error on the outcome")
	}
	if out.Upstream.Code != -32602 || out.Upstream.Message != "no such tool" {
		t.Fatalf("heuristic reconstruction changed: %+v", out.Upstream)
	}
	if len(out.Upstream.Data) != 0 {
		t.Fatalf("the heuristic must not invent data: %s", out.Upstream.Data)
	}
}
