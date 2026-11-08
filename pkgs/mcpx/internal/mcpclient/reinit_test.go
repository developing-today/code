package mcpclient_test

import (
	"errors"
	"fmt"
	"net/http"
	"net/http/httptest"
	"strings"
	"sync"
	"testing"

	"github.com/developing-today/code/pkgs/mcpx/internal/mcpclient"
)

// reinitUpstream is a fake legacy Streamable HTTP upstream whose sessions
// can be dropped mid-life, as a restart drops them: a request carrying a
// session id the upstream no longer holds is answered 404.
type reinitUpstream struct {
	version string

	mu        sync.Mutex
	seq       int
	live      map[string]bool
	stateless bool // never issue a session id
	call404   bool // answer every tools/call 404, session or not
	inits     int
	initSids  []string // session header each initialize carried
	callSids  []string // session header each tools/call carried
}

func (u *reinitUpstream) dropSessions() {
	u.mu.Lock()
	defer u.mu.Unlock()
	u.live = map[string]bool{}
}

func (u *reinitUpstream) counts() (inits int, initSids, callSids []string) {
	u.mu.Lock()
	defer u.mu.Unlock()
	return u.inits, append([]string(nil), u.initSids...), append([]string(nil), u.callSids...)
}

func (u *reinitUpstream) ServeHTTP(w http.ResponseWriter, r *http.Request) {
	if r.Method == http.MethodGet {
		w.WriteHeader(http.StatusMethodNotAllowed)
		return
	}
	if r.Method == http.MethodDelete {
		w.WriteHeader(http.StatusOK)
		return
	}
	if r.Method != http.MethodPost {
		w.WriteHeader(http.StatusMethodNotAllowed)
		return
	}
	s := decodePost(r)
	if len(s.id) == 0 {
		w.WriteHeader(http.StatusAccepted) // notification
		return
	}
	sid := s.header.Get("Mcp-Session-Id")
	u.mu.Lock()
	switch s.method {
	case "initialize":
		u.inits++
		u.initSids = append(u.initSids, sid)
		if !u.stateless {
			u.seq++
			newSid := fmt.Sprintf("sess-%d", u.seq)
			if u.live == nil {
				u.live = map[string]bool{}
			}
			u.live[newSid] = true
			w.Header().Set("Mcp-Session-Id", newSid)
		}
		u.mu.Unlock()
		writeJSON(w, result(s.id, initResult(u.version, map[string]any{"tools": map[string]any{}})))
	case "tools/call":
		u.callSids = append(u.callSids, sid)
		dead := u.call404 || (!u.stateless && !u.live[sid])
		u.mu.Unlock()
		if dead {
			w.WriteHeader(http.StatusNotFound)
			return
		}
		writeJSON(w, result(s.id, map[string]any{"content": []any{map[string]any{"type": "text", "text": "ok"}}}))
	default:
		u.mu.Unlock()
		writeJSON(w, result(s.id, map[string]any{"tools": []any{}}))
	}
}

// https://modelcontextprotocol.io/specification/2025-11-25/basic/transports#session-management
// (#1637: a legacy upstream that forgets the session mid-life is recovered
// by exactly one re-initialize and one retry.)
func TestLegacySession404Recovery(t *testing.T) {
	for _, v := range []string{"2025-03-26", "2025-06-18", "2025-11-25"} {
		t.Run(v+"/transport/client-reinitializes-on-session-404-and-retries-once", func(t *testing.T) {
			u := &reinitUpstream{version: v}
			srv := httptest.NewServer(u)
			t.Cleanup(srv.Close)
			c := dialURL(t, srv.URL, mcpclient.ForceLegacy, mcpclient.Options{})
			var changed []string
			var mu sync.Mutex
			c.Subscribe(mcpclient.Notifications{OnListChanged: func(kind string) {
				mu.Lock()
				changed = append(changed, kind)
				mu.Unlock()
			}})
			if _, err := c.CallTool(ctx5(), "x", nil); err != nil {
				t.Fatal(err)
			}
			u.dropSessions() // the upstream restarted and forgot sess-1
			out, err := c.CallTool(ctx5(), "x", nil)
			if err != nil {
				t.Fatalf("call after session drop: %v", err)
			}
			if !strings.Contains(string(out), "ok") {
				t.Fatalf("out %s", out)
			}
			inits, initSids, callSids := u.counts()
			if inits != 2 {
				t.Fatalf("initializes = %d, want exactly 2 (connect + one recovery)", inits)
			}
			if initSids[1] != "" {
				t.Fatalf("recovery initialize carried session id %q, want none", initSids[1])
			}
			if len(callSids) != 3 || callSids[1] != "sess-1" || callSids[2] != "sess-2" {
				t.Fatalf("tools/call session ids %v, want [sess-1 sess-1 sess-2]", callSids)
			}
			mu.Lock()
			defer mu.Unlock()
			if len(changed) != 3 {
				t.Fatalf("list-changed invalidations %v, want tools/resources/prompts", changed)
			}
		})
	}

	t.Run("concurrent-calls-share-one-reinitialize", func(t *testing.T) {
		u := &reinitUpstream{version: "2025-11-25"}
		srv := httptest.NewServer(u)
		t.Cleanup(srv.Close)
		c := dialURL(t, srv.URL, mcpclient.ForceLegacy, mcpclient.Options{})
		if _, err := c.CallTool(ctx5(), "x", nil); err != nil {
			t.Fatal(err)
		}
		u.dropSessions()
		const n = 8
		errs := make([]error, n)
		var wg sync.WaitGroup
		for i := range n {
			wg.Add(1)
			go func() {
				defer wg.Done()
				_, errs[i] = c.CallTool(ctx5(), "x", nil)
			}()
		}
		wg.Wait()
		for i, err := range errs {
			if err != nil {
				t.Fatalf("call %d: %v", i, err)
			}
		}
		if inits, _, _ := u.counts(); inits != 2 {
			t.Fatalf("initializes = %d, want exactly 2 for %d concurrent 404s", inits, n)
		}
	})

	t.Run("stateless-404-is-an-ordinary-error", func(t *testing.T) {
		u := &reinitUpstream{version: "2025-11-25", stateless: true, call404: true}
		srv := httptest.NewServer(u)
		t.Cleanup(srv.Close)
		c := dialURL(t, srv.URL, mcpclient.ForceLegacy, mcpclient.Options{})
		_, err := c.CallTool(ctx5(), "x", nil)
		var he *mcpclient.HTTPStatusError
		if !errors.As(err, &he) || he.Status != http.StatusNotFound {
			t.Fatalf("err = %v, want HTTP 404 as today", err)
		}
		if inits, _, _ := u.counts(); inits != 1 {
			t.Fatalf("initializes = %d, want 1: no session held, no recovery", inits)
		}
	})

	t.Run("second-404-surfaces-without-another-reinitialize", func(t *testing.T) {
		u := &reinitUpstream{version: "2025-11-25", call404: true}
		srv := httptest.NewServer(u)
		t.Cleanup(srv.Close)
		c := dialURL(t, srv.URL, mcpclient.ForceLegacy, mcpclient.Options{})
		_, err := c.CallTool(ctx5(), "x", nil)
		var he *mcpclient.HTTPStatusError
		if !errors.As(err, &he) || he.Status != http.StatusNotFound {
			t.Fatalf("err = %v, want HTTP 404", err)
		}
		if inits, _, _ := u.counts(); inits != 2 {
			t.Fatalf("initializes = %d, want 2 (connect + the single recovery attempt)", inits)
		}
	})

	t.Run("modern-404-is-not-recovered", func(t *testing.T) {
		rec := &recorder{}
		srv := modernHTTP(t, rec, func() []any { return []any{headerTool} }, func(s seen) (int, any) {
			return http.StatusNotFound, map[string]any{"detail": "no such tool"}
		})
		c := dialURL(t, srv.URL, mcpclient.ForceModern, mcpclient.Options{})
		_, err := c.CallTool(ctx5(), "sql", map[string]any{"query": "select 1"})
		var he *mcpclient.HTTPStatusError
		if !errors.As(err, &he) || he.Status != http.StatusNotFound {
			t.Fatalf("err = %v, want HTTP 404 as today", err)
		}
		if got := len(rec.all("tools/call")); got != 1 {
			t.Fatalf("tools/call posts = %d, want 1: a modern 404 is never retried", got)
		}
		if got := len(rec.all("initialize")); got != 0 {
			t.Fatalf("initialize posts = %d, want 0", got)
		}
	})
}
