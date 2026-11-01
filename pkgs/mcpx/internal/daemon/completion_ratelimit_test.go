package daemon

import (
	"net/http"
	"net/http/httptest"
	"strings"
	"testing"

	"github.com/developing-today/code/pkgs/mcpx/internal/config"
	"github.com/developing-today/code/pkgs/mcpx/internal/pool"
	"github.com/developing-today/code/pkgs/mcpx/internal/settings"
)

// #2387 handler contract: the limiter is consulted after shape
// validation and before the pool lookup, so a refused request is a 429
// naming the limit even for a server that does not exist -- it never
// reaches s.reg.Pool, let alone a pool lease or an upstream. An admitted
// request against the same unknown server is the pool's own 400, which
// is what makes the ordering observable.

func completeLimitSet(t *testing.T, perMinute, serverPerMinute string) *settings.Set {
	t.Helper()
	sch, err := settings.New(settings.Registry())
	if err != nil {
		t.Fatal(err)
	}
	return settings.NewSet(sch).WithOverrides(map[string]string{
		"completion.perMinute":       perMinute,
		"completion.serverPerMinute": serverPerMinute,
	}, "test")
}

func postComplete(t *testing.T, s *Server, session string) *httptest.ResponseRecorder {
	t.Helper()
	body := `{"server":"nope","ref":{"type":"ref/prompt","name":"p"},` +
		`"argument":{"name":"a","value":""},"session":"` + session + `"}`
	req := httptest.NewRequest(http.MethodPost, "/v1/complete", strings.NewReader(body))
	w := httptest.NewRecorder()
	s.handleComplete(w, req)
	return w
}

func TestHandleCompleteRateLimitRefusesBeforePool(t *testing.T) {
	s := &Server{set: completeLimitSet(t, "1", "0"), reg: &Registry{}}
	if w := postComplete(t, s, "sess-1"); w.Code != http.StatusBadRequest {
		t.Fatalf("first request: got %d (%s), want 400 from the pool lookup it reached", w.Code, w.Body)
	}
	w := postComplete(t, s, "sess-1")
	if w.Code != http.StatusTooManyRequests {
		t.Fatalf("second request: got %d (%s), want 429", w.Code, w.Body)
	}
	if !strings.Contains(w.Body.String(), "completion rate limit") ||
		!strings.Contains(w.Body.String(), "completion.perMinute") {
		t.Fatalf("429 must name the exhausted limit, got %s", w.Body)
	}
	// A second caller against the same server is still served (reaches
	// the pool lookup) until the per-server window trips -- here it is
	// disabled, so the second caller is admitted outright.
	if w := postComplete(t, s, "sess-2"); w.Code != http.StatusBadRequest {
		t.Fatalf("second caller: got %d (%s), want 400 (admitted, pool miss)", w.Code, w.Body)
	}
}

func TestHandleCompleteServerWindowBindsAcrossCallers(t *testing.T) {
	s := &Server{set: completeLimitSet(t, "0", "1"), reg: &Registry{}}
	if w := postComplete(t, s, "sess-1"); w.Code != http.StatusBadRequest {
		t.Fatalf("first request: got %d, want 400 (admitted)", w.Code)
	}
	w := postComplete(t, s, "sess-2")
	if w.Code != http.StatusTooManyRequests ||
		!strings.Contains(w.Body.String(), "completion.serverPerMinute") {
		t.Fatalf("second caller: got %d (%s), want 429 naming completion.serverPerMinute", w.Code, w.Body)
	}
}

func TestHandleCompleteRateLimitHotChangeAndZeroDisables(t *testing.T) {
	s := &Server{set: completeLimitSet(t, "1", "1"), reg: &Registry{}}
	_ = postComplete(t, s, "sess-1")
	if w := postComplete(t, s, "sess-1"); w.Code != http.StatusTooManyRequests {
		t.Fatalf("at limit 1: got %d, want 429", w.Code)
	}
	// Hot: a settings change takes effect on the very next request, and
	// 0 disables both windows.
	s.set = completeLimitSet(t, "0", "0")
	if w := postComplete(t, s, "sess-1"); w.Code != http.StatusBadRequest {
		t.Fatalf("after disabling: got %d (%s), want 400 (admitted, pool miss)", w.Code, w.Body)
	}
}

// The namespace spelling of a server resolves to the same pool, so it
// must share the same rate-limit budget: switching spellings after
// exhausting one is not a fresh budget (Codex review, PR #3730).
func TestHandleCompleteRateLimitCanonicalizesServerSpelling(t *testing.T) {
	reg := &Registry{
		pools: map[string]*pool.Pool{},
		views: map[string]*config.Resolved{"srv": {Namespace: "ns"}},
		order: []string{"srv"},
	}
	if got := reg.CanonicalName("srv"); got != "srv" {
		t.Fatalf("CanonicalName(srv) = %q", got)
	}
	if got := reg.CanonicalName("ns"); got != "srv" {
		t.Fatalf("CanonicalName(ns) = %q, want srv", got)
	}
	if got := reg.CanonicalName("nope"); got != "nope" {
		t.Fatalf("CanonicalName(nope) = %q, want unchanged", got)
	}
	s := &Server{set: completeLimitSet(t, "1", "0"), reg: reg}
	post := func(server string) *httptest.ResponseRecorder {
		body := `{"server":"` + server + `","ref":{"type":"ref/prompt","name":"p"},` +
			`"argument":{"name":"a","value":""},"session":"sess-1"}`
		req := httptest.NewRequest(http.MethodPost, "/v1/complete", strings.NewReader(body))
		w := httptest.NewRecorder()
		s.handleComplete(w, req)
		return w
	}
	// Neither spelling has a pool, so an admitted request is the pool
	// lookup's 400 under either spelling; only a refusal is a 429.
	if w := post("srv"); w.Code != http.StatusBadRequest {
		t.Fatalf("first request under the configured name: got %d (%s), want 400 (admitted)", w.Code, w.Body)
	}
	if w := post("ns"); w.Code != http.StatusTooManyRequests {
		t.Fatalf("namespace spelling after exhausting the name: got %d (%s), want 429 (shared budget)", w.Code, w.Body)
	}
}
