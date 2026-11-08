package daemon

import (
	"bytes"
	"encoding/json"
	"io"
	"log"
	"net/http"
	"net/http/httptest"
	"strings"
	"testing"
)

// dryRunServer is a Server whose registry knows one tool with an
// output schema, and no servers at all: any real call fails, so a
// successful answer can only have come from dry-run interception.
func dryRunServer() *Server {
	return &Server{logger: log.New(io.Discard, "", 0), reg: &Registry{testTools: []ToolInfo{
		{Server: "demo", Namespace: "demo", Tool: "echo",
			OutputSchema: json.RawMessage(`{"type":"object","properties":{"message":{"type":"string"}}}`)},
	}}}
}

func TestHandleCallDryRunInterceptsAndReports(t *testing.T) {
	s := dryRunServer()
	frames := collectFrames(s, "run-dry")
	s.callRegistry().RegisterDryRun("run-dry")

	body := []byte(`{"server":"demo","tool":"echo","args":{"message":"hi"}}`)
	req := httptest.NewRequest(http.MethodPost, "/v1/call", bytes.NewReader(body))
	req.Header.Set("X-Mcpx-Run", "run-dry")
	w := httptest.NewRecorder()
	s.handleCall(w, req)

	if w.Code != http.StatusOK {
		t.Fatalf("dry-run call: status %d, want 200 (body %s)", w.Code, w.Body.String())
	}
	var res struct {
		Result json.RawMessage `json:"result"`
	}
	if err := json.Unmarshal(w.Body.Bytes(), &res); err != nil {
		t.Fatal(err)
	}
	if !strings.Contains(string(res.Result), `"dryRun":true`) ||
		!strings.Contains(string(res.Result), `"message":""`) {
		t.Fatalf("result must be the schema-shaped synthetic, got %s", res.Result)
	}
	if len(*frames) != 1 || (*frames)[0].Call == nil || !(*frames)[0].Call.DryRun || !(*frames)[0].Call.OK {
		t.Fatalf("the intercepted call must be reported as an ok dry-run frame, got %+v", *frames)
	}
}

func TestHandleCallWithoutDryRunStillExecutes(t *testing.T) {
	s := dryRunServer()
	body := []byte(`{"server":"demo","tool":"echo","args":{"message":"hi"}}`)
	req := httptest.NewRequest(http.MethodPost, "/v1/call", bytes.NewReader(body))
	req.Header.Set("X-Mcpx-Run", "run-live")
	w := httptest.NewRecorder()
	s.handleCall(w, req)
	if w.Code == http.StatusOK {
		t.Fatalf("a live call to an unknown server must fail, got 200: %s", w.Body.String())
	}
}

// safePrefixServer knows a safe tool, a destructive tool, and
// another safe tool, on a server that does not exist: attempted
// calls fail, so a skipped item is distinguishable from a run one.
func safePrefixServer() *Server {
	return &Server{logger: log.New(io.Discard, "", 0), reg: &Registry{testTools: []ToolInfo{
		{Server: "demo", Namespace: "demo", Tool: "read",
			Annotations: json.RawMessage(`{"readOnlyHint":true,"destructiveHint":false}`)},
		{Server: "demo", Namespace: "demo", Tool: "delete",
			Annotations: json.RawMessage(`{"destructiveHint":true}`)},
		{Server: "demo", Namespace: "demo", Tool: "list",
			Annotations: json.RawMessage(`{"readOnlyHint":true}`)},
	}}}
}

type batchReply struct {
	Results []struct {
		Index   int    `json:"index"`
		Status  string `json:"status"`
		Skipped bool   `json:"skipped"`
		Reason  string `json:"reason"`
	} `json:"results"`
	SafePrefix *struct {
		StoppedAt int    `json:"stoppedAt"`
		Reason    string `json:"reason"`
	} `json:"safePrefix"`
}

func runBatch(t *testing.T, s *Server, run string, body string) batchReply {
	t.Helper()
	req := httptest.NewRequest(http.MethodPost, "/v1/batch", bytes.NewReader([]byte(body)))
	if run != "" {
		req.Header.Set("X-Mcpx-Run", run)
	}
	w := httptest.NewRecorder()
	s.handleBatch(w, req)
	if w.Code != http.StatusOK {
		t.Fatalf("batch: status %d (body %s)", w.Code, w.Body.String())
	}
	var res batchReply
	if err := json.Unmarshal(w.Body.Bytes(), &res); err != nil {
		t.Fatal(err)
	}
	return res
}

func TestHandleBatchSafePrefixStopsBeforeTheDestructiveCall(t *testing.T) {
	s := safePrefixServer()
	frames := collectFrames(s, "run-sp")
	res := runBatch(t, s, "run-sp", `{"safePrefix":true,"stopOnError":false,"calls":[
		{"server":"demo","tool":"read","args":{}},
		{"server":"demo","tool":"delete","args":{}},
		{"server":"demo","tool":"list","args":{}}
	]}`)
	if len(res.Results) != 3 {
		t.Fatalf("results = %d, want 3 (every item answered)", len(res.Results))
	}
	if res.Results[0].Skipped {
		t.Fatalf("the safe first call must be attempted, got %+v", res.Results[0])
	}
	for _, j := range []int{1, 2} {
		if !res.Results[j].Skipped || res.Results[j].Status != "skipped" ||
			!strings.Contains(res.Results[j].Reason, "demo.delete") {
			t.Fatalf("item %d must be skipped naming the destructive call, got %+v", j, res.Results[j])
		}
	}
	if res.SafePrefix == nil || res.SafePrefix.StoppedAt != 1 {
		t.Fatalf("safePrefix report must name index 1, got %+v", res.SafePrefix)
	}
	// Only the attempted call produced a call frame.
	if len(*frames) != 1 {
		t.Fatalf("frames = %d, want only the safe call reported", len(*frames))
	}
}

func TestHandleBatchSafePrefixDestructiveFirstRunsNothing(t *testing.T) {
	s := safePrefixServer()
	frames := collectFrames(s, "run-sp0")
	res := runBatch(t, s, "run-sp0", `{"safePrefix":true,"stopOnError":false,"calls":[
		{"server":"demo","tool":"delete","args":{}},
		{"server":"demo","tool":"read","args":{}}
	]}`)
	if res.SafePrefix == nil || res.SafePrefix.StoppedAt != 0 {
		t.Fatalf("stoppedAt must be 0, got %+v", res.SafePrefix)
	}
	for _, r := range res.Results {
		if !r.Skipped {
			t.Fatalf("nothing may run, got %+v", res.Results)
		}
	}
	if len(*frames) != 0 {
		t.Fatalf("no call frames when nothing ran, got %d", len(*frames))
	}
}

func TestHandleBatchSafePrefixAllSafeRunsFully(t *testing.T) {
	s := safePrefixServer()
	res := runBatch(t, s, "", `{"safePrefix":true,"stopOnError":false,"calls":[
		{"server":"demo","tool":"read","args":{}},
		{"server":"demo","tool":"list","args":{}}
	]}`)
	if res.SafePrefix != nil {
		t.Fatalf("an all-safe batch must not report a stop, got %+v", res.SafePrefix)
	}
	for _, r := range res.Results {
		if r.Skipped {
			t.Fatalf("an all-safe batch runs fully, got %+v", res.Results)
		}
	}
}

func TestHandleBatchWithoutSafePrefixRunsTheDestructiveCall(t *testing.T) {
	s := safePrefixServer()
	frames := collectFrames(s, "run-nsp")
	res := runBatch(t, s, "run-nsp", `{"stopOnError":false,"calls":[
		{"server":"demo","tool":"delete","args":{}}
	]}`)
	if len(res.Results) != 1 || res.Results[0].Skipped {
		t.Fatalf("without safePrefix the call is attempted, got %+v", res.Results)
	}
	if len(*frames) != 1 {
		t.Fatalf("the attempted call must be reported, got %d frames", len(*frames))
	}
}
