package pool_test

import (
	"encoding/json"
	"fmt"
	"net/http"
	"net/http/httptest"
	"os"
	"path/filepath"
	"strings"
	"sync"
	"testing"

	"github.com/developing-today/code/pkgs/mcpx/internal/config"
	"github.com/developing-today/code/pkgs/mcpx/internal/mcpauth"
	"github.com/developing-today/code/pkgs/mcpx/internal/pool"
)

// sessionPool builds a pool over url whose upstream sessions persist to
// store, as the daemon's pools do once upstreamHooks installs one.
func sessionPool(t *testing.T, srv *config.Server, store pool.SessionStore) *pool.Pool {
	t.Helper()
	cfg := &config.Config{MCPServers: map[string]*config.Server{"remote": srv}}
	r, err := cfg.Resolve("remote")
	if err != nil {
		t.Fatal(err)
	}
	p := pool.New(r)
	p.Hooks = &pool.Hooks{Sessions: store}
	t.Cleanup(p.Close)
	return p
}

func httpServerCfg(url string) *config.Server {
	return &config.Server{Name: "remote", URL: url + "/mcp"}
}

// TestAnUpstreamSessionSurvivesAColdRestart is the defining test of
// #1415: a legacy session is established, the daemon dies without a
// graceful shutdown (the first pool is never closed, so no DELETE is
// sent), and a new daemon -- a new pool over the store file as a new
// process finds it -- serves its first call on the original session.
func TestAnUpstreamSessionSurvivesAColdRestart(t *testing.T) {
	up := newSessionUpstream(t)
	path := filepath.Join(t.TempDir(), pool.UpstreamSessionsFile)

	first := sessionPool(t, httpServerCfg(up.URL), pool.OpenSessionFile(path))
	pingThrough(t, first, "k")

	second := sessionPool(t, httpServerCfg(up.URL), pool.OpenSessionFile(path))
	pingThrough(t, second, "k")

	up.mu.Lock()
	defer up.mu.Unlock()
	if up.issued != 1 {
		t.Fatalf("the upstream issued %d sessions; the cold restart should resume the first, not initialise again", up.issued)
	}
	if up.deletes != 0 {
		t.Fatalf("the cold restart ended the upstream session with %d DELETE(s)", up.deletes)
	}
	if len(up.served) == 0 {
		t.Fatal("no request after the cold restart reached the upstream")
	}
	for _, sid := range up.served {
		if sid != "sess-1" {
			t.Fatalf("a request after the cold restart carried session %q, want sess-1", sid)
		}
	}
}

// A session the pool definitively terminates leaves no persisted
// record behind: ReleaseKey closes the instance with a DELETE, so the
// record could only resume a dead session -- and for a scope: call
// key, which is never acquired again, it would otherwise be retained
// (and the whole table rewritten on every save) forever (Codex review
// on PR 5455).
func TestATerminatedSessionLeavesNoPersistedRecord(t *testing.T) {
	up := newSessionUpstream(t)
	path := filepath.Join(t.TempDir(), pool.UpstreamSessionsFile)
	store := pool.OpenSessionFile(path)
	p := sessionPool(t, httpServerCfg(up.URL), store)
	pingThrough(t, p, "k")

	if _, ok := store.Get("remote", "k", false); !ok {
		t.Fatal("no persisted record for the live session; the termination half of the test proves nothing")
	}
	if n := p.ReleaseKey("k"); n != 1 {
		t.Fatalf("ReleaseKey stopped %d instances, want 1", n)
	}
	if _, ok := store.Get("remote", "k", false); ok {
		t.Fatal("the persisted record survived the session's termination")
	}
	b, err := os.ReadFile(path)
	if err != nil {
		t.Fatal(err)
	}
	if strings.Contains(string(b), "sess-1") {
		t.Fatalf("the persisted file still names the terminated session:\n%s", b)
	}
	up.mu.Lock()
	defer up.mu.Unlock()
	if up.deletes != 1 {
		t.Fatalf("the upstream saw %d DELETE(s); the termination this guards is the one that ends the session", up.deletes)
	}
}

func TestRemovedServerTerminatesAndForgetsPersistedSessions(t *testing.T) {
	events := lifecycleCapture(t, "upstream.session.terminated", "upstream.session.leftToExpire")
	up := newSessionUpstream(t)
	store := pool.OpenSessionFile(filepath.Join(t.TempDir(), pool.UpstreamSessionsFile))
	p := sessionPool(t, httpServerCfg(up.URL), store)
	pingThrough(t, p, "k")

	p.CleanupRemovedSessionRecords()
	if _, ok := store.Get("remote", "k", false); ok {
		t.Fatal("removed server's persisted session survived reload cleanup")
	}
	up.mu.Lock()
	deletes := up.deletes
	up.mu.Unlock()
	if deletes != 1 {
		t.Fatalf("upstream received %d DELETEs, want one best-effort termination", deletes)
	}
	if got := events("upstream.session.terminated"); len(got) != 1 {
		t.Fatalf("termination events = %v, want one successful event", got)
	}
}

func TestUnconfiguredSessionRecordIsRemovedAndLeftToExpire(t *testing.T) {
	events := lifecycleCapture(t, "upstream.session.leftToExpire")
	store := pool.OpenSessionFile(filepath.Join(t.TempDir(), pool.UpstreamSessionsFile))
	if err := store.Put(pool.UpstreamSessionRecord{Server: "gone", Key: "k", SessionID: "sess-orphan"}); err != nil {
		t.Fatal(err)
	}

	pool.DiscardUnconfiguredSessionRecords(store, map[string]bool{})
	if _, ok := store.Get("gone", "k", false); ok {
		t.Fatal("record for a server absent from the live config survived cleanup")
	}
	if got := events("upstream.session.leftToExpire"); len(got) != 1 {
		t.Fatalf("left-to-expire events = %v, want one", got)
	}
}

// A record whose config fingerprint no longer matches the live config is
// discarded: the restart initialises fresh and no error is surfaced.
func TestAColdRestartDiscardsARecordWhoseConfigChanged(t *testing.T) {
	up := newSessionUpstream(t)
	path := filepath.Join(t.TempDir(), pool.UpstreamSessionsFile)

	first := sessionPool(t, httpServerCfg(up.URL), pool.OpenSessionFile(path))
	pingThrough(t, first, "k")

	changed := httpServerCfg(up.URL)
	changed.Headers = map[string]string{"X-Extra": "1"}
	second := sessionPool(t, changed, pool.OpenSessionFile(path))
	pingThrough(t, second, "k")

	up.mu.Lock()
	defer up.mu.Unlock()
	if up.issued != 2 {
		t.Fatalf("the upstream issued %d sessions; a fingerprint mismatch should initialise fresh", up.issued)
	}
	last := up.served[len(up.served)-1]
	if last != "sess-2" {
		t.Fatalf("the request after the restart carried session %q, want the fresh sess-2", last)
	}
}

// A session the upstream forgot while the daemon was down is refused by
// the resume probe and the start falls back to a fresh initialize: the
// first call after the restart still succeeds.
func TestAColdRestartFallsBackWhenTheUpstreamForgotTheSession(t *testing.T) {
	up := newSessionUpstream(t)
	path := filepath.Join(t.TempDir(), pool.UpstreamSessionsFile)

	first := sessionPool(t, httpServerCfg(up.URL), pool.OpenSessionFile(path))
	pingThrough(t, first, "k")

	up.expireAll()

	second := sessionPool(t, httpServerCfg(up.URL), pool.OpenSessionFile(path))
	pingThrough(t, second, "k")

	up.mu.Lock()
	defer up.mu.Unlock()
	if up.issued != 2 {
		t.Fatalf("the upstream issued %d sessions; a forgotten session should be replaced by a fresh initialize", up.issued)
	}
	last := up.served[len(up.served)-1]
	if last != "sess-2" {
		t.Fatalf("the request after the restart carried session %q, want the fresh sess-2", last)
	}
}

// A corrupt or version-mismatched store file is ignored and logged, and
// the daemon starts and operates normally without it.
func TestASessionStoreIgnoresACorruptOrForeignFile(t *testing.T) {
	for name, contents := range map[string]string{
		"corrupt":   "this is not a session table",
		"foreign":   `{"version": 99, "sessions": {}}`,
		"truncated": `{"version": 1, "sessions": {"remote`,
	} {
		t.Run(name, func(t *testing.T) {
			events := lifecycleCapture(t, "upstream.session.storeIgnored")
			up := newSessionUpstream(t)
			path := filepath.Join(t.TempDir(), pool.UpstreamSessionsFile)
			if err := os.WriteFile(path, []byte(contents), 0o600); err != nil {
				t.Fatal(err)
			}
			p := sessionPool(t, httpServerCfg(up.URL), pool.OpenSessionFile(path))
			pingThrough(t, p, "k")

			up.mu.Lock()
			issued := up.issued
			up.mu.Unlock()
			if issued != 1 {
				t.Fatalf("the upstream issued %d sessions; want one fresh initialize", issued)
			}
			if got := events("upstream.session.storeIgnored"); len(got) != 1 {
				t.Fatalf("want the ignored store file logged once, got %v", got)
			}
		})
	}
}

// The persisted file must never carry a resolved header or secret value:
// the record type has no field for one, and this scans the written bytes
// for the sentinel the config resolved, as #1415's acceptance requires.
func TestAPersistedSessionRecordCarriesNoSecretValue(t *testing.T) {
	const sentinel = "SENTINEL-SECRET-VALUE-1415"
	up := newSessionUpstream(t)
	path := filepath.Join(t.TempDir(), pool.UpstreamSessionsFile)

	srv := httpServerCfg(up.URL)
	srv.Headers = map[string]string{"Authorization": "Bearer " + sentinel}
	srv.Auth = &mcpauth.Auth{Type: "bearer", Token: sentinel}
	p := sessionPool(t, srv, pool.OpenSessionFile(path))
	pingThrough(t, p, "k")

	b, err := os.ReadFile(path)
	if err != nil {
		t.Fatal(err)
	}
	if strings.Contains(string(b), sentinel) {
		t.Fatalf("the persisted session file contains the resolved secret value:\n%s", b)
	}
	if !strings.Contains(string(b), "sess-1") {
		t.Fatalf("the persisted session file does not name the session it should resume:\n%s", b)
	}
	var doc struct {
		Version  int            `json:"version"`
		Sessions map[string]any `json:"sessions"`
		Raw      map[string]any `json:"-"`
	}
	if err := json.Unmarshal(b, &doc); err != nil || doc.Version != 1 || len(doc.Sessions) != 1 {
		t.Fatalf("unexpected session file shape: err=%v doc=%+v", err, doc)
	}
}

// rotatingUpstream rotates the session id on every answered call: each
// reply installs a new Mcp-Session-Id and forgets the previous one.
type rotatingUpstream struct {
	*httptest.Server
	mu     sync.Mutex
	live   map[string]bool
	issued int
	served []string
}

func newRotatingUpstream(t *testing.T) *rotatingUpstream {
	t.Helper()
	u := &rotatingUpstream{live: map[string]bool{}}
	u.Server = httptest.NewServer(http.HandlerFunc(u.serve))
	t.Cleanup(u.Close)
	return u
}

func (u *rotatingUpstream) serve(w http.ResponseWriter, r *http.Request) {
	u.mu.Lock()
	defer u.mu.Unlock()
	sid := r.Header.Get("Mcp-Session-Id")
	switch r.Method {
	case http.MethodDelete:
		if u.live[sid] {
			delete(u.live, sid)
			w.WriteHeader(http.StatusOK)
			return
		}
		w.WriteHeader(http.StatusNotFound)
	case http.MethodPost:
		var req struct {
			ID     json.RawMessage `json:"id"`
			Method string          `json:"method"`
		}
		_ = json.NewDecoder(r.Body).Decode(&req)
		switch {
		case req.Method == "initialize":
			u.issued++
			sid = fmt.Sprintf("sess-%d", u.issued)
			u.live[sid] = true
			w.Header().Set("Mcp-Session-Id", sid)
			rpcReply(w, req.ID, map[string]any{
				"protocolVersion": "2025-03-26",
				"capabilities":    map[string]any{"tools": map[string]any{}},
				"serverInfo":      map[string]any{"name": "upstream", "version": "1"},
			})
		case req.Method == "notifications/initialized":
			w.WriteHeader(http.StatusAccepted)
		case req.Method == "server/discover":
			rpcError(w, req.ID, -32601, "method not found")
		case !u.live[sid]:
			w.WriteHeader(http.StatusNotFound)
		default:
			u.served = append(u.served, sid)
			delete(u.live, sid)
			u.issued++
			next := fmt.Sprintf("sess-%d", u.issued)
			u.live[next] = true
			w.Header().Set("Mcp-Session-Id", next)
			rpcReply(w, req.ID, map[string]any{})
		}
	default:
		w.WriteHeader(http.StatusMethodNotAllowed)
	}
}

// When an upstream rotates the session id mid-flight, the record follows:
// a cold restart resumes the rotated session, not the original one.
func TestASessionIDRotatedMidFlightIsTheOnePersisted(t *testing.T) {
	up := newRotatingUpstream(t)
	path := filepath.Join(t.TempDir(), pool.UpstreamSessionsFile)

	first := sessionPool(t, httpServerCfg(up.URL), pool.OpenSessionFile(path))
	// The ping is answered on sess-1 and rotates the transport to sess-2;
	// the adoption hook must save sess-2 before the restart.
	pingThrough(t, first, "k")

	second := sessionPool(t, httpServerCfg(up.URL), pool.OpenSessionFile(path))
	pingThrough(t, second, "k")

	up.mu.Lock()
	defer up.mu.Unlock()
	// sess-1: the first pool's ping, which rotated the session to sess-2.
	// sess-2: the restarted pool's resume probe -- proof it resumed the
	// rotated session, not the original; its own ping then serves sess-3.
	if len(up.served) != 3 || up.served[0] != "sess-1" || up.served[1] != "sess-2" || up.served[2] != "sess-3" {
		t.Fatalf("served sessions = %v; want [sess-1 sess-2 sess-3] -- the restart should resume the rotated session", up.served)
	}
}

// A persisted session has exactly one resumer: Take hands the record to
// the first start and removes it, so a concurrent start for the same
// identity initialises fresh instead of resuming the same session --
// whose DELETE-on-close would otherwise kill the survivor's session
// when Acquire retires the duplicate (Codex review on PR 5455).
func TestASessionRecordHasExactlyOneResumer(t *testing.T) {
	path := filepath.Join(t.TempDir(), pool.UpstreamSessionsFile)
	store := pool.OpenSessionFile(path)
	rec := pool.UpstreamSessionRecord{Server: "remote", Key: "k", SessionID: "sess-1"}
	if err := store.Put(rec); err != nil {
		t.Fatal(err)
	}
	if got, ok := store.Take("remote", "k", false); !ok || got.SessionID != "sess-1" {
		t.Fatalf("first Take = %+v, %v; want the sess-1 record", got, ok)
	}
	if _, ok := store.Take("remote", "k", false); ok {
		t.Fatal("second Take returned the same session to a second resumer")
	}
	reopened := pool.OpenSessionFile(path)
	if _, ok := reopened.Get("remote", "k", false); ok {
		t.Fatal("the taken record is still in the persisted file")
	}
}

// Query auth resolves its credential into the transport URL, so the
// persisted record must not carry it: the URL is stored with its query
// stripped (Codex review on PR 5455). The header/bearer sentinel scan
// above cannot see this mode.
func TestAPersistedSessionRecordCarriesNoQueryCredential(t *testing.T) {
	const sentinel = "SENTINEL-QUERY-SECRET-1415"
	up := newSessionUpstream(t)
	path := filepath.Join(t.TempDir(), pool.UpstreamSessionsFile)

	srv := httpServerCfg(up.URL)
	srv.Auth = &mcpauth.Auth{Type: "query", Param: "api_key", ParamValue: sentinel}
	p := sessionPool(t, srv, pool.OpenSessionFile(path))
	pingThrough(t, p, "k")

	b, err := os.ReadFile(path)
	if err != nil {
		t.Fatal(err)
	}
	if strings.Contains(string(b), sentinel) {
		t.Fatalf("the persisted session file contains the resolved query credential:\n%s", b)
	}
	if !strings.Contains(string(b), "sess-1") {
		t.Fatalf("the persisted session file does not name the session it should resume:\n%s", b)
	}
}

// A session adopted in a takeover is persisted like one this daemon
// started (Codex review on PR 5455): the adopting pool has the store,
// the predecessor did not, and after Attach the record names the
// adopted session, so a cold restart after the takeover can resume it.
func TestAnAdoptedSessionIsPersistedByTheAdoptingPool(t *testing.T) {
	up := newSessionUpstream(t)
	path := filepath.Join(t.TempDir(), pool.UpstreamSessionsFile)

	old := upstreamPool(t, up.URL)
	pingThrough(t, old, "k")
	successor := sessionPool(t, httpServerCfg(up.URL), pool.OpenSessionFile(path))
	handOff(t, old, successor)

	b, err := os.ReadFile(path)
	if err != nil {
		t.Fatalf("the adopting pool wrote no session file: %v", err)
	}
	if !strings.Contains(string(b), "sess-1") {
		t.Fatalf("the adopting pool's session file does not name the adopted session:\n%s", b)
	}
}

// reinitUpstream forgets every session on expireAll and answers the
// next initialize with a different protocol version, so a test can see
// which protocol state a save paired with the replacement session id.
type reinitUpstream struct {
	*httptest.Server
	mu      sync.Mutex
	live    map[string]bool
	issued  int
	version string
}

func newReinitUpstream(t *testing.T) *reinitUpstream {
	t.Helper()
	u := &reinitUpstream{live: map[string]bool{}, version: "2025-03-26"}
	u.Server = httptest.NewServer(http.HandlerFunc(u.serve))
	t.Cleanup(u.Close)
	return u
}

func (u *reinitUpstream) expireAll(nextVersion string) {
	u.mu.Lock()
	defer u.mu.Unlock()
	u.live = map[string]bool{}
	u.version = nextVersion
}

func (u *reinitUpstream) serve(w http.ResponseWriter, r *http.Request) {
	u.mu.Lock()
	defer u.mu.Unlock()
	sid := r.Header.Get("Mcp-Session-Id")
	switch r.Method {
	case http.MethodDelete:
		if u.live[sid] {
			delete(u.live, sid)
			w.WriteHeader(http.StatusOK)
			return
		}
		w.WriteHeader(http.StatusNotFound)
	case http.MethodPost:
		var req struct {
			ID     json.RawMessage `json:"id"`
			Method string          `json:"method"`
		}
		_ = json.NewDecoder(r.Body).Decode(&req)
		switch {
		case req.Method == "initialize":
			u.issued++
			sid = fmt.Sprintf("sess-%d", u.issued)
			u.live[sid] = true
			w.Header().Set("Mcp-Session-Id", sid)
			rpcReply(w, req.ID, map[string]any{
				"protocolVersion": u.version,
				"capabilities":    map[string]any{"tools": map[string]any{}},
				"serverInfo":      map[string]any{"name": "upstream", "version": "1"},
			})
		case req.Method == "notifications/initialized":
			w.WriteHeader(http.StatusAccepted)
		case req.Method == "server/discover":
			rpcError(w, req.ID, -32601, "method not found")
		case !u.live[sid]:
			w.WriteHeader(http.StatusNotFound)
		default:
			rpcReply(w, req.ID, map[string]any{})
		}
	default:
		w.WriteHeader(http.StatusMethodNotAllowed)
	}
}

// When a session expires mid-flight and the client reinitialises, the
// persisted record must pair the NEW session id with the NEW session's
// protocol state. The transport's adoption hook fires at the initialize
// response headers -- before finishLegacy installs the recovered
// session's negotiated version -- so a save taken then would persist
// the old version against the new id (Codex review on PR 5455).
func TestARecoveredSessionIsPersistedWithItsOwnProtocolState(t *testing.T) {
	up := newReinitUpstream(t)
	path := filepath.Join(t.TempDir(), pool.UpstreamSessionsFile)
	p := sessionPool(t, httpServerCfg(up.URL), pool.OpenSessionFile(path))
	pingThrough(t, p, "k")

	up.expireAll("2025-06-18")
	pingThrough(t, p, "k")

	b, err := os.ReadFile(path)
	if err != nil {
		t.Fatal(err)
	}
	var doc struct {
		Sessions map[string]struct {
			SessionID  string `json:"sessionId"`
			Negotiated string `json:"negotiated"`
		} `json:"sessions"`
	}
	if err := json.Unmarshal(b, &doc); err != nil {
		t.Fatal(err)
	}
	if len(doc.Sessions) != 1 {
		t.Fatalf("session file holds %d records, want 1:\n%s", len(doc.Sessions), b)
	}
	for _, rec := range doc.Sessions {
		if rec.SessionID != "sess-2" || rec.Negotiated != "2025-06-18" {
			t.Fatalf("persisted record = session %q negotiated %q; want sess-2 with its own 2025-06-18", rec.SessionID, rec.Negotiated)
		}
	}
}
