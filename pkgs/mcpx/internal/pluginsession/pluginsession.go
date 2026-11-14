// Package pluginsession implements the plugin-session foundation for
// mcpx's privileged plugin surface (#1873, Approach C; decisions 0001
// and 0003).
//
// A plugin session is the identity the daemon issues to a connecting
// plugin. Everything the session declares or creates -- its manifest
// hook subscriptions (decision 0001: hooks come from the manifest,
// are owned by the session, and there is no hook-registration route),
// its declared capabilities (decision 0003: declared, recorded and
// reportable; enforcement is later security work and deliberately
// not here), and any ownership later slices register on its behalf --
// is keyed to the session, listed through status introspection, and
// dropped atomically when the session ends. Close is the single
// teardown operation: the clean path (the plugin disconnects) and
// the crash path (the daemon notices the plugin is gone) both call
// it, so what a session owned can never outlive the session by
// accident.
//
// A plugin session is not the leasing/attribution session a caller
// names with MCPX_SESSION_ID: that session scopes pooled server
// instances, this one names a plugin's lifetime on the daemon
// (#1873 open question 1). The id prefix keeps the two apart in
// logs and events.
package pluginsession

import (
	"crypto/rand"
	"encoding/hex"
	"fmt"
	"sort"
	"sync"
	"time"
)

// IDPrefix prefixes every plugin session id, so a plugin session is
// distinguishable from a leasing session id wherever either appears.
const IDPrefix = "ps-"

// OwnedItem is one thing a plugin session owns: a registration a
// later slice created on the session's behalf (a hook subscription,
// a variable, a named command). Kind is the owning surface's word
// for the thing; Name identifies it within that surface.
type OwnedItem struct {
	Kind string `json:"kind"`
	Name string `json:"name"`
}

// Session is one live plugin session: its identity, what its plugin
// declared at open, and everything it owns. Snapshots returned by
// the registry are copies; mutating one changes nothing.
type Session struct {
	ID           string      `json:"id"`
	Plugin       string      `json:"plugin"`
	Capabilities []string    `json:"capabilities"`
	Hooks        []string    `json:"hooks"`
	Owned        []OwnedItem `json:"owned"`
	StartedAt    time.Time   `json:"startedAt"`
}

// Registry is the daemon's table of live plugin sessions. The zero
// value is ready. Every operation is safe for concurrent use, and
// teardown is atomic: a session and everything it owns disappear in
// the same locked step, so no observer can see a half-dropped
// session.
type Registry struct {
	mu      sync.Mutex
	byID    map[string]*Session
	counter uint64
}

// Open issues a new plugin session for the named plugin, recording
// the capabilities and hook subscriptions its manifest declares
// (declare-only, per decision 0003: recorded and reportable, never
// enforced here). Declarations are stored as given minus blanks and
// duplicates; the session owns them from this moment.
func (r *Registry) Open(plugin string, capabilities, hooks []string) Session {
	r.mu.Lock()
	defer r.mu.Unlock()
	r.initLocked()
	r.counter++
	s := &Session{
		ID:           r.newIDLocked(),
		Plugin:       plugin,
		Capabilities: cleanStrings(capabilities),
		Hooks:        cleanStrings(hooks),
		Owned:        []OwnedItem{},
		StartedAt:    time.Now().UTC(),
	}
	r.byID[s.ID] = s
	return snapshot(s)
}

// Get returns the live session with this id, if there is one.
func (r *Registry) Get(id string) (Session, bool) {
	r.mu.Lock()
	defer r.mu.Unlock()
	s, ok := r.byID[id]
	if !ok {
		return Session{}, false
	}
	return snapshot(s), true
}

// List returns every live session, oldest first. The result is
// never nil: no live sessions is an empty list.
func (r *Registry) List() []Session {
	r.mu.Lock()
	defer r.mu.Unlock()
	out := make([]Session, 0, len(r.byID))
	for _, s := range r.byID {
		out = append(out, snapshot(s))
	}
	sort.Slice(out, func(i, j int) bool {
		if !out[i].StartedAt.Equal(out[j].StartedAt) {
			return out[i].StartedAt.Before(out[j].StartedAt)
		}
		return out[i].ID < out[j].ID
	})
	return out
}

// Own records one owned item against a live session -- the
// registration a later slice (hook subscriptions resolved from the
// manifest, the vars store, the named-command registry) makes on
// the session's behalf. Recording the same item twice is a no-op.
// It reports false when the session is not live: a registration
// against a dead session is refused, never parked somewhere the
// teardown can no longer reach.
func (r *Registry) Own(id string, item OwnedItem) bool {
	r.mu.Lock()
	defer r.mu.Unlock()
	s, ok := r.byID[id]
	if !ok {
		return false
	}
	for _, o := range s.Owned {
		if o == item {
			return true
		}
	}
	s.Owned = append(s.Owned, item)
	return true
}

// Close ends a session and drops everything it owns, atomically:
// the session, its declarations and its owned items are removed in
// one locked step, and the returned snapshot is the full record of
// what was dropped. It reports false when the session was not live
// -- closing twice, or closing a session that never existed, drops
// nothing.
func (r *Registry) Close(id string) (Session, bool) {
	r.mu.Lock()
	defer r.mu.Unlock()
	s, ok := r.byID[id]
	if !ok {
		return Session{}, false
	}
	delete(r.byID, id)
	return snapshot(s), true
}

func (r *Registry) initLocked() {
	if r.byID == nil {
		r.byID = make(map[string]*Session)
	}
}

// newIDLocked mints a session id. The counter is mixed in so two
// ids minted in the same instant still differ even in the
// (practically impossible) case of a repeated random draw.
func (r *Registry) newIDLocked() string {
	var b [12]byte
	if _, err := rand.Read(b[:]); err != nil {
		return fmt.Sprintf("%s%d-%d", IDPrefix, time.Now().UnixNano(), r.counter)
	}
	return IDPrefix + hex.EncodeToString(b[:]) + fmt.Sprintf("-%d", r.counter)
}

func snapshot(s *Session) Session {
	out := *s
	out.Capabilities = append([]string{}, s.Capabilities...)
	out.Hooks = append([]string{}, s.Hooks...)
	out.Owned = append([]OwnedItem{}, s.Owned...)
	return out
}

// cleanStrings drops blanks and duplicates, preserving order: a
// manifest that declares the same capability twice has declared it
// once.
func cleanStrings(in []string) []string {
	out := []string{}
	seen := make(map[string]bool, len(in))
	for _, v := range in {
		if v == "" || seen[v] {
			continue
		}
		seen[v] = true
		out = append(out, v)
	}
	return out
}
