package daemon

import (
	"net/http"
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
