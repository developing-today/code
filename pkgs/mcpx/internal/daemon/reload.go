package daemon

import (
	"fmt"
	"sort"

	"github.com/developing-today/code/pkgs/mcpx/internal/config"
	"github.com/developing-today/code/pkgs/mcpx/internal/pool"
)

// Reload swaps in a new configuration without restarting anything that did
// not change.
//
// The question this answers is "can mcpx add a server live", and until now
// the answer was no: the configuration was read once at startup and nothing
// re-read it, so adding a server meant stopping a daemon that other callers
// were using. That is a bad trade for an edit to one line of JSON.
//
// A server whose process definition is unchanged keeps its pool, and
// therefore keeps its running child and its cached schemas. Only the entries
// that actually changed are stopped. Without that, reloading to add one
// server would restart every other one, and a stateful server -- a browser
// holding a session -- would lose it because a neighbour was added.
func (r *Registry) Reload(cfg *config.Config) (added, removed []string, err error) {
	servers, err := cfg.ResolveAll()
	if err != nil {
		return nil, nil, err
	}

	r.reloadMu.Lock()
	defer r.reloadMu.Unlock()
	r.mu.Lock()
	defer r.mu.Unlock()

	// Index the pools in use by their process identity, not by name, so a
	// rename that changes nothing about the child process keeps the child.
	// The identity a pool was made with is what it is matched on: a binary
	// that has since been replaced no longer matches, and its pool is
	// replaced, which is a restart of that one server and nothing else.
	byPoolID := map[string]*pool.Pool{}
	for _, p := range r.pools {
		byPoolID[p.PoolID()] = p
	}

	pools := make(map[string]*pool.Pool, len(servers))
	views := make(map[string]*config.Resolved, len(servers))
	var order []string
	fresh := map[string]*pool.Pool{}
	seen := map[string]string{}

	for _, s := range servers {
		if prev, dup := seen[s.Namespace]; dup {
			return nil, nil, fmt.Errorf(
				"servers %q and %q both map to namespace %q; set mcpx.namespace on one of them",
				prev, s.Name, s.Namespace)
		}
		seen[s.Namespace] = s.Name
		r.bindSecrets(s)

		id := s.ProcessID()
		p, kept := byPoolID[id]
		if !kept {
			p, kept = fresh[id]
		}
		if !kept {
			p = pool.New(s)
			p.Hooks = r.hooks
			fresh[id] = p
			added = append(added, s.Name)
		}
		pools[s.Name] = p
		views[s.Name] = s
		order = append(order, s.Name)
	}
	sort.Strings(order)

	if err := validateViews(order, views, pools); err != nil {
		return nil, nil, err
	}

	// Anything whose process definition is gone is stopped. Done after the
	// new map is built so a failure above leaves the daemon exactly as it
	// was rather than half-reloaded.
	keep := map[*pool.Pool]bool{}
	for _, p := range pools {
		keep[p] = true
	}
	var orphans []*pool.Pool
	removedPools := map[*pool.Pool]bool{}
	for name, p := range r.pools {
		if _, still := pools[name]; !still {
			removed = append(removed, name)
			removedPools[p] = true
		}
		if !keep[p] {
			orphans = append(orphans, p)
		}
	}
	sort.Strings(removed)
	r.cfg = cfg
	r.pools, r.views, r.order = pools, views, order
	r.hash = HashConfig(mustJSON(cfg.MCPServers))
	// Mark orphans closed before the replacement map is visible. A later
	// reload must not resume a record whose old client is still draining and
	// will DELETE that upstream session.
	for _, p := range orphans {
		p.CloseDrained()
	}
	if r.pendingSessionCleanup == nil {
		r.pendingSessionCleanup = map[string]int{}
	}
	for p := range removedPools {
		r.pendingSessionCleanup[p.Name()]++
	}

	// Drained, not closed: a call running on an orphan finishes on it, as it
	// does on a lazy restart. Close would cut one that ran past its grace.
	go func() {
		for _, p := range orphans {
			p.WaitDrained()
			if removedPools[p] {
				r.reloadMu.Lock()
				r.mu.RLock()
				_, reconfigured := r.pools[p.Name()]
				r.mu.RUnlock()
				if !reconfigured {
					p.CleanupRemovedSessionRecords()
				}
				if r.pendingSessionCleanup[p.Name()] <= 1 {
					delete(r.pendingSessionCleanup, p.Name())
				} else {
					r.pendingSessionCleanup[p.Name()]--
				}
				r.reloadMu.Unlock()
			}
		}
		r.reloadMu.Lock()
		r.mu.RLock()
		configured := make(map[string]bool, len(r.pools))
		for name := range r.pools {
			configured[name] = true
		}
		// A later reload can have no orphan pools while an earlier removed
		// pool is still draining. Keep those records out of the generic
		// discard path until the owner has attempted DELETE with its last
		// live config.
		for name := range r.pendingSessionCleanup {
			configured[name] = true
		}
		var sessions pool.SessionStore
		if r.hooks != nil {
			sessions = r.hooks.Sessions
		}
		r.mu.RUnlock()
		pool.DiscardUnconfiguredSessionRecords(sessions, configured)
		r.reloadMu.Unlock()
	}()
	return added, removed, nil
}

// Config returns the configuration currently in force.
func (r *Registry) Config() *config.Config {
	r.mu.RLock()
	defer r.mu.RUnlock()
	return r.cfg
}
