// Package pluginsession provides manifest-hook resolution for plugin sessions (#1873 slice 2,
// #10046): the declarations Registry.Open records become live bus
// subscribers, owned by the session and dropped by its Close.
//
// The #1872 machinery question, answered: #1872 (config-declared
// hooks) is closed and its machinery HAS landed on main --
// internal/hooks parses the `hooks` setting and its Runner
// subscribes per hook and is wired into the daemon. This slice does
// not depend on that Runner, because a Runner hook's only actions
// are child processes (`run`, `module`): a plugin is not a child
// process the daemon spawns per event, and no executor in
// internal/hooks delivers to one. What this slice reuses is the
// layer both hook families share -- the events bus itself, and
// hooks.Hook.SubFilter / hooks.HookableKinds, so a manifest
// declaration and a config declaration can never disagree about
// which events match. What it builds is the minimal session-scoped
// piece that does not exist: subscribe at session open, deliver to
// the owning plugin, unsubscribe at session Close.
//
// A manifest hook declaration is one events.Filter kind prefix,
// verbatim (decision 0001: with phase "observe", `on` is a kind
// prefix -- "elicit" matches every elicitation event, "*" matches
// every hookable kind). Observe is the only phase a manifest hook
// can be: it fires after the fact and can never slow, veto or
// rewrite what it watched. A declaration that names no hookable
// kind -- a blocking phase name like "onSuccess", which belongs to
// the enumerated hook points and not to the bus -- stays recorded
// on the session exactly as the foundation left it and subscribes
// to nothing.
//
// Delivery is an event on the same bus: kind plugin.hook, Session
// set to the owning plugin session's id, Data carrying the Delivery
// (which declaration fired, and the event itself). The plugin hears
// its deliveries over its own surface -- the /v1/events stream
// filtered to its session -- and nobody else's: the delivery's
// Session is the session that declared the hook, so one session's
// hooks are never another's. Delivery events are never hook
// triggers (the forward loop skips them), so a hook cannot retrigger
// itself through its own delivery.
package pluginsession

import (
	"encoding/json"
	"strings"
	"sync"

	"github.com/developing-today/code/pkgs/mcpx/internal/events"
	"github.com/developing-today/code/pkgs/mcpx/internal/hooks"
)

// DeliveryKind is the bus kind of a manifest-hook delivery.
const DeliveryKind events.Kind = "plugin.hook"

// Delivery is the Data of a DeliveryKind event: one manifest hook
// of one plugin session fired, and this is the event it saw.
type Delivery struct {
	Plugin string       `json:"plugin"`
	Hook   string       `json:"hook"`
	Event  events.Event `json:"event"`
}

// Resolver binds a Registry to an events bus: Open resolves the
// session's manifest hooks into live subscribers, Close drops them
// with the session. The zero value is not usable; NewResolver
// builds one. A Resolver with a nil bus resolves nothing -- the
// declarations stay recorded, as the foundation left them -- which
// is what a bus-less daemon (tests construct them bare) gets.
type Resolver struct {
	reg *Registry
	bus *events.Bus

	mu   sync.Mutex
	live map[string]*sessionSubs
}

type sessionSubs struct {
	done chan struct{}
	subs []*events.Subscription
}

// NewResolver builds a Resolver over reg and bus.
func NewResolver(reg *Registry, bus *events.Bus) *Resolver {
	return &Resolver{reg: reg, bus: bus, live: map[string]*sessionSubs{}}
}

// Open opens a plugin session and resolves its manifest hooks:
// each hookable declaration becomes a live bus subscription,
// registered on the session as an owned hook item, so status
// introspection shows what the session's hooks are doing. The
// returned snapshot carries the owned items.
func (r *Resolver) Open(plugin string, capabilities, hookDecls []string) Session {
	sess := r.reg.Open(plugin, capabilities, hookDecls)
	if r.bus == nil || len(sess.Hooks) == 0 {
		return sess
	}
	entry := &sessionSubs{done: make(chan struct{})}
	for _, decl := range sess.Hooks {
		if !hookableDecl(decl) {
			continue
		}
		sub, _ := r.bus.Subscribe(hooks.Hook{On: []string{decl}}.SubFilter(), 0)
		if !r.reg.Own(sess.ID, OwnedItem{Kind: "hook", Name: decl}) {
			sub.Close()
			continue
		}
		entry.subs = append(entry.subs, sub)
		go r.forward(sess, decl, sub, entry.done)
	}
	if len(entry.subs) > 0 {
		r.mu.Lock()
		r.live[sess.ID] = entry
		r.mu.Unlock()
	}
	if snap, ok := r.reg.Get(sess.ID); ok {
		return snap
	}
	return sess
}

// Close ends a session and drops its subscribers with it: the
// forward loops stop first, the subscriptions leave the bus, and
// only then does the registry drop the session and everything it
// owns -- so no delivery can be published for a session that no
// longer exists, and a post-close event fires nothing. It reports
// false when the session was not live.
func (r *Resolver) Close(id string) (Session, bool) {
	r.mu.Lock()
	entry, ok := r.live[id]
	if ok {
		delete(r.live, id)
	}
	r.mu.Unlock()
	if ok {
		close(entry.done)
		for _, sub := range entry.subs {
			sub.Close()
		}
	}
	return r.reg.Close(id)
}

// forward is one subscription's delivery loop: every event the bus
// matched for this declaration is republished as a DeliveryKind
// event addressed to the owning session. It ends when the session
// closes (done, or the subscription's channel closing under it).
func (r *Resolver) forward(sess Session, decl string, sub *events.Subscription, done <-chan struct{}) {
	for {
		select {
		case <-done:
			return
		case e, ok := <-sub.C:
			if !ok {
				return
			}
			if e.Kind == DeliveryKind {
				continue
			}
			data, err := json.Marshal(Delivery{Plugin: sess.Plugin, Hook: decl, Event: e})
			if err != nil {
				continue
			}
			r.bus.Publish(events.Event{Kind: DeliveryKind, Session: sess.ID, Data: data})
		}
	}
}

// hookableDecl reports whether a manifest declaration can match at
// least one hookable kind, under the Filter rule (exact, or prefix
// followed by a dot, or "*") -- the same test internal/hooks
// applies to config declarations, over the same kind list. The
// hook.* kinds are excluded: a hook on them could retrigger hook
// machinery forever.
func hookableDecl(decl string) bool {
	if decl == "*" {
		return true
	}
	if decl == "hook" || strings.HasPrefix(decl, "hook.") {
		return false
	}
	for _, k := range hooks.HookableKinds() {
		ks := string(k)
		if ks == decl || strings.HasPrefix(ks, decl+".") {
			return true
		}
	}
	return false
}
