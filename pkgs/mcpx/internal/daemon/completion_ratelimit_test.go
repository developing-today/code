package daemon

import (
	"net/http"
	"net/http/httptest"
	"os"
	"strconv"
	"strings"
	"testing"

	"github.com/developing-today/code/pkgs/mcpx/internal/config"
	"github.com/developing-today/code/pkgs/mcpx/internal/pool"
	"github.com/developing-today/code/pkgs/mcpx/internal/settings"
)

// #2387 handler contract: for a server the registry knows, the
// limiter is consulted after shape validation and before the pool
// lookup, so a refused request is a 429 naming the limit -- it never
// reaches s.reg.Pool, let alone a pool lease or an upstream. An
// admitted request against a known server with no live pool is the
// pool's own 400, which is what makes the ordering observable.
// #6978 narrows the contract for names the registry does NOT know:
// they skip the limiter entirely (no buckets minted) and take the
// pool lookup's 400 directly.

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

// knownNoPoolRegistry knows the name "nope" (a configured view) but
// holds no pool for it, so Pool("nope") still misses: an admitted
// request is observable as the pool lookup's 400, a refused one as
// the limiter's 429 -- the refusal-before-pool ordering, exercised
// on a KNOWN server as #6978 requires.
func knownNoPoolRegistry() *Registry {
	return &Registry{
		pools: map[string]*pool.Pool{},
		views: map[string]*config.Resolved{"nope": {Namespace: "nope-ns"}},
		order: []string{"nope"},
	}
}

func TestHandleCompleteRateLimitRefusesBeforePool(t *testing.T) {
	s := &Server{set: completeLimitSet(t, "1", "0"), reg: knownNoPoolRegistry()}
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
	s := &Server{set: completeLimitSet(t, "0", "1"), reg: knownNoPoolRegistry()}
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
	s := &Server{set: completeLimitSet(t, "1", "1"), reg: knownNoPoolRegistry()}
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

// #3783: a PIDShared PID is the daemon's own -- every /v1/exec run
// reports it (MCPX_PID in the exec script env) -- so it must not key
// the per-caller completion window. Session-less exec callers fall
// through to their per-call ids and never refuse each other under
// completion.perMinute; the per-server window remains the backstop.
func TestCompletionCallerIgnoresSharedPID(t *testing.T) {
	sharedA := config.CallContext{PID: 4242, PIDShared: true, CallID: "anon-a"}
	sharedB := config.CallContext{PID: 4242, PIDShared: true, CallID: "anon-b"}
	if got, want := completionCaller(sharedA), "call:anon-a"; got != want {
		t.Fatalf("shared-pid caller A: got %q, want %q", got, want)
	}
	if completionCaller(sharedA) == completionCaller(sharedB) {
		t.Fatal("two shared-pid callers with distinct call ids share a key")
	}
	if got, want := completionCaller(config.CallContext{PID: 4242, CallID: "anon-a"}), "pid:4242"; got != want {
		t.Fatalf("own-pid caller: got %q, want %q", got, want)
	}
	withSession := config.CallContext{SessionID: "s1", PID: 4242, PIDShared: true, CallID: "anon-a"}
	if got, want := completionCaller(withSession), "session:s1"; got != want {
		t.Fatalf("session caller: got %q, want %q", got, want)
	}
}

func TestHandleCompleteSharedPIDCallersDoNotRefuseEachOther(t *testing.T) {
	s := &Server{set: completeLimitSet(t, "1", "0"), reg: knownNoPoolRegistry()}
	post := func(call string) *httptest.ResponseRecorder {
		body := `{"server":"nope","ref":{"type":"ref/prompt","name":"p"},` +
			`"argument":{"name":"a","value":""}}`
		req := httptest.NewRequest(http.MethodPost, "/v1/complete", strings.NewReader(body))
		// An exec-style caller: no session, and the PID it reports is
		// the daemon's own (in-process here, so this test's pid),
		// which callContext marks PIDShared.
		req.Header.Set("X-Mcpx-Pid", strconv.Itoa(os.Getpid()))
		req.Header.Set("X-Mcpx-Call", call)
		w := httptest.NewRecorder()
		s.handleComplete(w, req)
		return w
	}
	if w := post("call-a"); w.Code != http.StatusBadRequest {
		t.Fatalf("first exec caller: got %d (%s), want 400 (admitted, pool miss)", w.Code, w.Body)
	}
	if w := post("call-b"); w.Code != http.StatusBadRequest {
		t.Fatalf("second exec caller: got %d (%s), want 400 (admitted, pool miss) -- a shared daemon PID must not spend another caller's per-caller budget", w.Code, w.Body)
	}
	// A repeat under the SAME call id is the same per-call key, so
	// the window still binds it: the fix un-shares the daemon PID, it
	// does not disable the per-caller window.
	if w := post("call-a"); w.Code != http.StatusTooManyRequests {
		t.Fatalf("repeat same call id: got %d (%s), want 429 (same per-call key, limit 1)", w.Code, w.Body)
	}
}

// #6978: a name the registry does not know must not mint limiter
// buckets. Repeated calls under one unknown name never trip a
// window, and many distinct unknown names leave the bucket count
// at zero -- bucket growth is not request-controlled.
func TestHandleCompleteUnknownServersMintNoLimiterBuckets(t *testing.T) {
	s := &Server{set: completeLimitSet(t, "1", "1"), reg: &Registry{}}
	post := func(server string) *httptest.ResponseRecorder {
		body := `{"server":"` + server + `","ref":{"type":"ref/prompt","name":"p"},` +
			`"argument":{"name":"a","value":""},"session":"sess-1"}`
		req := httptest.NewRequest(http.MethodPost, "/v1/complete", strings.NewReader(body))
		w := httptest.NewRecorder()
		s.handleComplete(w, req)
		return w
	}
	for i := 0; i < 3; i++ {
		if w := post("nope"); w.Code != http.StatusBadRequest {
			t.Fatalf("repeat unknown name call %d: got %d (%s), want 400 (pool miss, never a limiter 429)", i, w.Code, w.Body)
		}
	}
	for i := 0; i < 20; i++ {
		name := "bogus-" + strconv.Itoa(i)
		if w := post(name); w.Code != http.StatusBadRequest {
			t.Fatalf("distinct unknown name %q: got %d (%s), want 400", name, w.Code, w.Body)
		}
	}
	if got := len(s.completeLimiter.buckets); got != 0 {
		t.Fatalf("limiter buckets after unknown-name calls: got %d, want 0", got)
	}
	// The same name under a registry that KNOWS it is still limited:
	// the gate is registry knowledge, not the name itself.
	known := &Server{set: completeLimitSet(t, "1", "0"), reg: knownNoPoolRegistry()}
	_ = postComplete(t, known, "sess-1")
	if w := postComplete(t, known, "sess-1"); w.Code != http.StatusTooManyRequests {
		t.Fatalf("known server second call: got %d, want 429", w.Code)
	}
}
