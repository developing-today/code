package pool

import (
	"context"
	"crypto/sha256"
	"encoding/hex"
	"encoding/json"
	"fmt"
	"net/url"
	"os"
	"path/filepath"
	"sort"
	"sync"
	"time"

	"github.com/developing-today/code/pkgs/mcpx/internal/config"
	"github.com/developing-today/code/pkgs/mcpx/internal/defaults"
	"github.com/developing-today/code/pkgs/mcpx/internal/mcpclient"
)

// UpstreamSessionsFile is the name of the persisted upstream-session
// table inside the daemon state directory, a sibling of sessions.json.
const UpstreamSessionsFile = "upstream-sessions.json"

const upstreamSessionsVersion = 1

// UpstreamSessionRecord is the resumable half of one legacy streamable
// HTTP upstream session, persisted across a cold restart (#1415, Phase 1
// of design #1414).
//
// The record type has NO field for headers or any resolved secret value.
// That absence is the security invariant: handoff headers can carry
// resolved credentials and travel over the takeover socket only. On
// restore the headers are re-derived from live config by the dial path
// (httpOptions / resolveAuth), exactly as for a fresh connection.
type UpstreamSessionRecord struct {
	// Server and Key are the record's identity: the pool's server name
	// and the scope key the instance serves -- the same identity the
	// lease table's owned map uses.
	Server string `json:"server"`
	Key    string `json:"key"`
	// LegacyLane separates the legacy-only lane's session from the
	// default lane's for the same key: under protocol: follow both can
	// hold a legacy session at once, and they are different sessions.
	LegacyLane  bool              `json:"legacyLane,omitempty"`
	URL         string            `json:"url"`
	Fingerprint string            `json:"fingerprint"`
	SessionID   string            `json:"sessionId"`
	Negotiated  string            `json:"negotiated,omitempty"`
	Session     mcpclient.Session `json:"session"`
	SavedAt     time.Time         `json:"savedAt"`
}

// SessionStore persists upstream session records. It is the session
// analogue of EraStore: one store is shared by every pool, opened by the
// daemon over the state directory.
type SessionStore interface {
	Get(server, key string, legacyLane bool) (UpstreamSessionRecord, bool)
	// List returns a snapshot of every persisted record, for reload cleanup.
	List() []UpstreamSessionRecord
	// Take is Get plus removal, atomically: a persisted session has
	// exactly one resumer. Two concurrent starts for the same identity
	// must never both resume the same upstream session -- when Acquire
	// later retires the duplicate start, closing it DELETEs the session
	// the survivor is using. The loser of a Take race initialises fresh,
	// as starts did before persistence existed, and the winner's start
	// re-saves the record when it completes.
	Take(server, key string, legacyLane bool) (UpstreamSessionRecord, bool)
	Put(rec UpstreamSessionRecord) error
	// Remove discards the record for one instance identity, for when
	// that instance's session has been definitively terminated (closed
	// with a DELETE): the record could only ever resume a dead
	// session, and for scope: call identities, which are never
	// acquired again, it would otherwise be retained -- and the whole
	// table rewritten on every save -- forever.
	Remove(server, key string, legacyLane bool)
}

// sessionFingerprint is a hash over the server config fields that define
// a session's meaning: the upstream URL, the transport, the resolved era
// preference, the NAMES of the declared headers, and the auth block's
// identifying names. Header values, tokens, passwords and every other
// resolved secret are never hashed or stored -- the fingerprint changes
// when the session's meaning changes, and cannot leak what it names.
func sessionFingerprint(c *config.Resolved, pref mcpclient.Preference) string {
	headerNames := make([]string, 0, len(c.Headers))
	for name := range c.Headers {
		headerNames = append(headerNames, name)
	}
	sort.Strings(headerNames)
	auth := struct {
		Set    bool     `json:"set"`
		Type   string   `json:"type,omitempty"`
		Header string   `json:"header,omitempty"`
		Param  string   `json:"param,omitempty"`
		Env    []string `json:"env,omitempty"`
	}{}
	if c.Auth != nil {
		auth.Set = true
		auth.Type = c.Auth.Type
		auth.Header = c.Auth.Header
		auth.Param = c.Auth.Param
		for name := range c.Auth.Env {
			auth.Env = append(auth.Env, name)
		}
		sort.Strings(auth.Env)
	}
	b, _ := json.Marshal(struct {
		URL       string   `json:"url"`
		Transport string   `json:"transport"`
		Protocol  string   `json:"protocol"`
		Headers   []string `json:"headers"`
		Auth      any      `json:"auth"`
	}{c.URL, c.Transport, string(pref), headerNames, auth})
	sum := sha256.Sum256(b)
	return hex.EncodeToString(sum[:])
}

type sessionFileDoc struct {
	Version  int                              `json:"version"`
	Sessions map[string]UpstreamSessionRecord `json:"sessions"`
}

// SessionFile is a SessionStore held in memory and persisted to one JSON
// file, written with the sessions.go discipline: a versioned envelope,
// written to a temporary file and renamed into place at
// defaults.PrivateMode.
type SessionFile struct {
	path string
	mu   sync.Mutex
	m    map[string]UpstreamSessionRecord
}

func sessionStoreKey(server, key string, legacyLane bool) string {
	lane := "default"
	if legacyLane {
		lane = "legacy"
	}
	return server + "\x00" + key + "\x00" + lane
}

// OpenSessionFile loads the persisted upstream sessions at path.
//
// A missing file is simply an empty store. A file that is corrupt or of
// another version is ignored and logged through the pool lifecycle hook:
// a cold restart then re-initialises every upstream, exactly as a daemon
// without the file does, and the next Put rewrites the file whole.
func OpenSessionFile(path string) *SessionFile {
	f := &SessionFile{path: path, m: map[string]UpstreamSessionRecord{}}
	b, err := os.ReadFile(path)
	if err != nil {
		return f
	}
	var doc sessionFileDoc
	if err := json.Unmarshal(b, &doc); err != nil || doc.Version != upstreamSessionsVersion {
		lifecycle("upstream.session.storeIgnored", map[string]any{
			"path": path, "reason": "not a session table this daemon reads",
		})
		return f
	}
	for k, v := range doc.Sessions {
		if v.SessionID != "" {
			f.m[k] = v
		}
	}
	return f
}

// Get returns the persisted session for one instance identity, if any.
func (f *SessionFile) Get(server, key string, legacyLane bool) (UpstreamSessionRecord, bool) {
	f.mu.Lock()
	defer f.mu.Unlock()
	r, ok := f.m[sessionStoreKey(server, key, legacyLane)]
	return r, ok
}

// List returns a stable snapshot of the session table.
func (f *SessionFile) List() []UpstreamSessionRecord {
	f.mu.Lock()
	defer f.mu.Unlock()
	out := make([]UpstreamSessionRecord, 0, len(f.m))
	for _, rec := range f.m {
		out = append(out, rec)
	}
	return out
}

// writeLocked rewrites the file atomically from the in-memory table: a
// temporary file in place beside it, then a rename, so a crash mid-write
// leaves the old file or the new one and never half of each.
func (f *SessionFile) writeLocked() error {
	b, err := json.Marshal(sessionFileDoc{Version: upstreamSessionsVersion, Sessions: f.m})
	if err != nil {
		return err
	}
	if dir := filepath.Dir(f.path); dir != "" {
		if err := os.MkdirAll(dir, defaults.DirMode); err != nil {
			return err
		}
	}
	tmp := f.path + ".tmp"
	if err := os.WriteFile(tmp, b, defaults.PrivateMode); err != nil {
		return err
	}
	return os.Rename(tmp, f.path)
}

// Put records one session and rewrites the file.
func (f *SessionFile) Put(rec UpstreamSessionRecord) error {
	f.mu.Lock()
	defer f.mu.Unlock()
	f.m[sessionStoreKey(rec.Server, rec.Key, rec.LegacyLane)] = rec
	return f.writeLocked()
}

// Take returns the persisted session for one instance identity and
// removes it, so no second start can resume the same session. A failed
// rewrite still returns the record with the in-memory entry removed:
// the session then behaves as one the store never had if this start
// also fails, which is the pre-persistence behaviour, not an error
// worth surfacing to the caller.
func (f *SessionFile) Take(server, key string, legacyLane bool) (UpstreamSessionRecord, bool) {
	f.mu.Lock()
	defer f.mu.Unlock()
	k := sessionStoreKey(server, key, legacyLane)
	r, ok := f.m[k]
	if !ok {
		return UpstreamSessionRecord{}, false
	}
	delete(f.m, k)
	_ = f.writeLocked()
	return r, true
}

// Remove discards the persisted session for one instance identity.
// Removing an identity the store does not hold is a no-op and does not
// rewrite the file.
func (f *SessionFile) Remove(server, key string, legacyLane bool) {
	f.mu.Lock()
	defer f.mu.Unlock()
	k := sessionStoreKey(server, key, legacyLane)
	if _, ok := f.m[k]; !ok {
		return
	}
	delete(f.m, k)
	_ = f.writeLocked()
}

func (p *Pool) sessionStore() SessionStore {
	if h := p.Hooks; h != nil {
		return h.Sessions
	}
	return nil
}

// saveUpstreamSession persists the session the instance behind ref now
// holds, when it is a legacy HTTP session with an id. It runs when an
// instance finishes starting and whenever its transport adopts a new
// session id mid-flight. Best effort by design: resume is an
// optimisation, so a failed save is logged and non-fatal -- the daemon
// behaves exactly as one that never persisted.
func (p *Pool) saveUpstreamSession(eff *config.Resolved, key string, legacyLane bool, ref *instanceRef) {
	p.sessionMu.Lock()
	defer p.sessionMu.Unlock()
	if p.sessionsClosing.Load() {
		return
	}
	store := p.sessionStore()
	if store == nil {
		return
	}
	in := ref.ptr.Load()
	if in == nil || in.Client == nil || in.Client.Era != mcpclient.EraLegacy {
		return
	}
	hs, ok := in.Client.HTTPSession()
	if !ok || hs.SessionID == "" {
		return
	}
	rec := UpstreamSessionRecord{
		Server:      p.cfg.Name,
		Key:         key,
		LegacyLane:  legacyLane,
		URL:         sanitizedSessionURL(hs.URL),
		Fingerprint: sessionFingerprint(eff, p.Preference()),
		SessionID:   hs.SessionID,
		Negotiated:  hs.Negotiated,
		Session:     in.Client.SessionState(),
		SavedAt:     time.Now(),
	}
	if err := store.Put(rec); err != nil {
		lifecycle("upstream.session.saveFailed", map[string]any{
			"server": p.cfg.Name, "key": key, "reason": err.Error(),
		})
	}
}

// forgetUpstreamSession discards the persisted record for an instance
// whose session is being definitively terminated -- the pool is about
// to Close it, which DELETEs the upstream session. Call it at the
// termination points that retire a live instance for good (capacity
// eviction, ReleaseKey, the idle/orphan reap): without it the record
// outlives its session, and a per-call (scope: call) key's record is
// never Taken again, so it would be retained forever while every
// subsequent save rewrites the whole table. Shutdown paths (Close and
// friends) deliberately do NOT forget: surviving a restart is the
// record's whole purpose.
func (p *Pool) forgetUpstreamSession(in *Instance) {
	p.sessionMu.Lock()
	defer p.sessionMu.Unlock()
	store := p.sessionStore()
	if store == nil || in == nil {
		return
	}
	store.Remove(p.cfg.Name, in.key, in.legacyLane)
}

// CleanupRemovedSessionRecords runs after a removed pool has drained. It
// terminates each persisted session using credentials re-derived from the
// pool's last live config, then removes the record. A stale record is never
// sent to a potentially changed endpoint.
func (p *Pool) CleanupRemovedSessionRecords() {
	store := p.sessionStore()
	if store == nil {
		return
	}
	opts, optsErr := p.httpOptions()
	for _, rec := range store.List() {
		if rec.Server != p.cfg.Name {
			continue
		}
		var terminateErr error
		switch {
		case optsErr != nil:
			terminateErr = optsErr
		case rec.Fingerprint != sessionFingerprint(p.cfg, p.Preference()) || rec.URL != sanitizedSessionURL(opts.URL):
			terminateErr = fmt.Errorf("last live config no longer matches the persisted session")
		default:
			terminateErr = mcpclient.TerminateHTTPSession(mcpclient.HTTPHandoff{
				SessionID: rec.SessionID, Negotiated: rec.Negotiated,
				URL: opts.URL, Headers: opts.Headers,
			})
		}
		store.Remove(rec.Server, rec.Key, rec.LegacyLane)
		if terminateErr != nil {
			lifecycle("upstream.session.leftToExpire", map[string]any{
				"server": rec.Server, "key": rec.Key, "session": rec.SessionID,
				"reason": terminateErr.Error(),
			})
			continue
		}
		lifecycle("upstream.session.terminated", map[string]any{
			"server": rec.Server, "key": rec.Key, "session": rec.SessionID,
			"reason": "server removed from configuration",
		})
	}
}

// DiscardUnconfiguredSessionRecords removes records for servers that were
// already absent from the registry before this reload, so there is no last
// live config from which to re-derive credentials for a DELETE.
func DiscardUnconfiguredSessionRecords(store SessionStore, configured map[string]bool) {
	if store == nil {
		return
	}
	for _, rec := range store.List() {
		if configured[rec.Server] {
			continue
		}
		store.Remove(rec.Server, rec.Key, rec.LegacyLane)
		lifecycle("upstream.session.leftToExpire", map[string]any{
			"server": rec.Server, "key": rec.Key, "session": rec.SessionID,
			"reason": "server absent from the last live configuration",
		})
	}
}

// sanitizedSessionURL strips everything credential-shaped from a
// transport URL before it is persisted: the query string (auth.type
// query resolves its credential INTO the URL -- see httpOptions) and
// any userinfo. The record never needs them: restoration dials from
// live config, which re-derives credentials, and HTTPTransport.Resume
// does not dial the handoff URL. An unparseable URL persists as empty
// rather than risk carrying a credential into the file.
func sanitizedSessionURL(raw string) string {
	u, err := url.Parse(raw)
	if err != nil {
		return ""
	}
	u.User = nil
	u.RawQuery = ""
	u.ForceQuery = false
	u.Fragment = ""
	return u.String()
}

// installSessionPersistence wires a started or adopted instance's
// session into the store: save whenever its transport adopts a new
// session id mid-flight, save again once a legacy recovery has fully
// settled (the adoption hook fires at initialize-response headers,
// before the recovered session's protocol state is installed -- see
// mcpclient.SetSessionSettledHook), and save the session it now holds.
// Used by startLane and by the takeover path (resumeInstance), so a
// session adopted from a predecessor is persisted exactly like one
// this daemon started.
func (p *Pool) installSessionPersistence(eff *config.Resolved, key string, legacyLane bool, ref *instanceRef, cl *mcpclient.Client, tr mcpclient.Transport) {
	if p.sessionStore() == nil {
		return
	}
	if ht, ok := tr.(*mcpclient.HTTPTransport); ok {
		ht.SetSessionAdoptedHook(func(string) {
			p.saveUpstreamSession(eff, key, legacyLane, ref)
		})
	}
	if cl != nil {
		cl.SetSessionSettledHook(func() {
			p.saveUpstreamSession(eff, key, legacyLane, ref)
		})
	}
}

// resumeUpstreamSession tries to serve this start from the persisted
// record for the instance, through the identical calls the takeover
// restore path makes (see AdoptSessions and attachSession): the
// transport -- already dialled from live config, so its headers are the
// live ones -- is seeded with ht.Resume and the client is built with
// mcpclient.Resume. There is no parallel resume implementation.
//
// The resumed session is probed before it is served, as Attach probes.
// One the upstream no longer knows is abandoned without a DELETE and the
// start falls back to a fresh dial, which the returned transport
// reflects. A nil client with a nil error means "no resume": start fresh
// on the returned transport. An error is only a failed fallback dial.
func (p *Pool) resumeUpstreamSession(ctx context.Context, tr mcpclient.Transport, eff *config.Resolved, rs config.ResolvedScope, legacyLane bool, opts mcpclient.Options) (*mcpclient.Client, mcpclient.Transport, error) {
	store := p.sessionStore()
	if store == nil {
		return nil, tr, nil
	}
	ht, ok := tr.(*mcpclient.HTTPTransport)
	if !ok {
		return nil, tr, nil
	}
	// Take, not Get: this start now owns the record, and a concurrent
	// start for the same identity initialises fresh instead of resuming
	// the same session (see SessionStore.Take). Every exit below is
	// safe: a refused resume redials fresh and the fresh start saves
	// its own record; a fingerprint mismatch discards the record,
	// which is what the mismatch path always intended.
	rec, ok := store.Take(p.cfg.Name, rs.Key, legacyLane)
	if !ok || rec.SessionID == "" {
		return nil, tr, nil
	}
	if rec.Fingerprint != sessionFingerprint(eff, p.Preference()) {
		// The configuration that gave this session its meaning has
		// changed: the record is discarded and the start initialises
		// fresh, with no error surfaced.
		lifecycle("upstream.session.fingerprintMismatch", map[string]any{
			"server": p.cfg.Name, "key": rs.Key, "session": rec.SessionID,
		})
		return nil, tr, nil
	}
	ht.Resume(mcpclient.HTTPHandoff{SessionID: rec.SessionID, Negotiated: rec.Negotiated, URL: rec.URL})
	cl := mcpclient.Resume(tr, rec.Session, opts)
	cl.Activate()
	pctx, cancel := context.WithTimeout(ctx, resumeProbeTimeout)
	err := cl.Ping(pctx)
	cancel()
	if err != nil {
		cl.Abandon()
		lifecycle("upstream.session.resumeRefused", map[string]any{
			"server": p.cfg.Name, "key": rs.Key, "session": rec.SessionID,
			"reason": err.Error(),
		})
		ntr, derr := p.dial(ctx, eff, rs.Root)
		if derr != nil {
			return nil, tr, fmt.Errorf("redial after a refused resume: %w", derr)
		}
		return nil, ntr, nil
	}
	lifecycle("upstream.session.resumed", map[string]any{
		"server": p.cfg.Name, "key": rs.Key, "session": rec.SessionID,
	})
	return cl, tr, nil
}
