package daemon

import (
	"context"
	"net/http"
	"net/http/httptest"
	"testing"

	"github.com/dezren39/mcpx/internal/config"
	"github.com/dezren39/mcpx/internal/tasks"
)

// The /v1/tasks admission window must key by the same session identity
// task creation uses: callContext reads X-Mcpx-Session, so a caller
// identified only by that header must not fall into the shared unbound
// window (Codex finding on #2869).
func TestTaskRequestorHonorsSessionHeader(t *testing.T) {
	req := func(url string, headers map[string]string) *http.Request {
		r, _ := http.NewRequest(http.MethodGet, url, nil)
		for k, v := range headers {
			r.Header.Set(k, v)
		}
		return r
	}
	if got := taskRequestor(req("/v1/tasks", map[string]string{"X-Mcpx-Session": "sess-1"})); got != tasks.Caller("sess-1") {
		t.Errorf("X-Mcpx-Session keys admission, got %v", got)
	}
	if got := taskRequestor(req("/v1/tasks?session=q", map[string]string{"X-Mcpx-Session": "sess-1"})); got != tasks.Caller("q") {
		t.Errorf("the query param keeps precedence, got %v", got)
	}
	if got := taskRequestor(req("/v1/tasks", map[string]string{"X-Mcpx-Run": "run-1"})); got != tasks.Caller("run-1") {
		t.Errorf("the run header is the next fallback, got %v", got)
	}
	if got := taskRequestor(req("/v1/tasks", nil)); got != tasks.UnboundRequestor {
		t.Errorf("no identity lands in the unbound bucket, got %v", got)
	}
}

// Anonymous ask-seam tasks count against their own capped bucket so a
// cap of 1 still permits one logical ask operation (its MCP wrapper
// counts against unbound); identified ask callers keep their session.
func TestAskRequestor(t *testing.T) {
	if got := askRequestor(config.CallContext{}); got != tasks.AskRequestor {
		t.Errorf("anonymous ask counts against the ask bucket, got %v", got)
	}
	if got := askRequestor(config.CallContext{SessionID: "sess-9"}); got != tasks.Ask("sess-9") {
		t.Errorf("an identified ask counts against its session in the ask role, got %v", got)
	}
	// The identified ask bucket is NOT the caller bucket for the same
	// session: the wrapper charges Caller(sess-9), the backing charges
	// Ask(sess-9), and a cap of 1 must leave room for both.
	if askRequestor(config.CallContext{SessionID: "sess-9"}) == tasks.Caller("sess-9") {
		t.Error("ask and caller roles for one identity must be distinct requestors")
	}
}

// A session-less reporting call charges its run, not the shared
// unbound bucket: with the per-requestor cap at one, one run's pending
// reporting call must not refuse another run's first, a second call
// from the same run is refused, and an explicit session takes
// precedence over the run header. (Codex on #2869: the fix in
// handleCallReporting needs the failing-first regression — removing
// the run fallback leaves every case below charging unbound, so the
// same-run refusal never fires.)
func TestHandleCallReportingChargesRunForSessionlessCallers(t *testing.T) {
	srv, err := NewServer(handoffOptions(handoffPaths(t),
		&config.Config{MCPServers: map[string]*config.Server{}}, "v"))
	if err != nil {
		t.Fatal(err)
	}
	store := srv.taskStore()
	store.MaxPerRequestor = 1
	occupy := func(r tasks.Requestor) {
		t.Helper()
		tsk, err := store.Start(r, 60_000, func(ctx context.Context) (any, *tasks.Fault) {
			<-ctx.Done()
			return nil, nil
		})
		if err != nil {
			t.Fatalf("occupy %v: %v", r, err)
		}
		t.Cleanup(func() { store.Cancel(tsk.TaskID) })
	}
	occupy(tasks.Caller("run-1"))
	occupy(tasks.Caller("sess-1"))

	call := func(session, run string) int {
		t.Helper()
		req := httptest.NewRequest(http.MethodPost, "/v1/call", nil)
		if run != "" {
			req.Header.Set("X-Mcpx-Run", run)
		}
		w := httptest.NewRecorder()
		srv.handleCallReporting(w, req, callReq{Server: "nope", Tool: "x"},
			config.CallContext{SessionID: session}, map[string]any{})
		return w.Code
	}
	// Another run's first call is admitted (and fails on the unknown
	// server, not on the cap).
	if code := call("", "run-2"); code != http.StatusBadRequest {
		t.Errorf("run-2 first call: status %d, want 400 (admitted)", code)
	}
	// The same run's second call is refused by the cap.
	if code := call("", "run-1"); code != http.StatusTooManyRequests {
		t.Errorf("run-1 second call: status %d, want 429 (cap)", code)
	}
	// An explicit session takes precedence over the run header: this
	// charges the free sess-2, not the occupied run-1.
	if code := call("sess-2", "run-1"); code != http.StatusBadRequest {
		t.Errorf("sess-2 via run-1: status %d, want 400 (admitted)", code)
	}
	// And a call for the occupied session is refused whatever run it
	// names.
	if code := call("sess-1", "run-2"); code != http.StatusTooManyRequests {
		t.Errorf("sess-1 via run-2: status %d, want 429 (cap)", code)
	}
}
