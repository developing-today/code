package daemon

import (
	"context"
	"errors"
	"sync"
	"time"

	"github.com/developing-today/code/pkgs/mcpx/internal/config"
	"github.com/developing-today/code/pkgs/mcpx/internal/defaults"
	"github.com/developing-today/code/pkgs/mcpx/internal/mcpclient"
	"github.com/developing-today/code/pkgs/mcpx/internal/pool"
)

// RegistryHandoff is the registry's pools taken out of service for a successor.
type RegistryHandoff struct {
	Pools    []pool.PoolHandoff
	detached []*pool.Pool
}

// Detach takes every pool out of service. Names may share a pool, which is
// detached once. A pool with calls still in flight is waited for, up to wait,
// since detaching it would fail those calls.
func (r *Registry) Detach(wait time.Duration) (*RegistryHandoff, error) {
	r.mu.RLock()
	seen := map[*pool.Pool]bool{}
	var pools []*pool.Pool
	for _, name := range r.order {
		if p := r.pools[name]; !seen[p] {
			seen[p] = true
			pools = append(pools, p)
		}
	}
	r.mu.RUnlock()

	deadline := time.Now().Add(wait)
	for {
		h, err := detachAll(pools)
		if !errors.Is(err, pool.ErrBusy) || !time.Now().Before(deadline) {
			return h, err
		}
		time.Sleep(defaults.HandoffRetryInterval)
	}
}

func detachAll(pools []*pool.Pool) (*RegistryHandoff, error) {
	h := &RegistryHandoff{}
	for _, p := range pools {
		ph, err := p.Detach()
		if err != nil {
			h.Restore()
			return nil, err
		}
		h.detached = append(h.detached, p)
		if len(ph.Instances) > 0 {
			h.Pools = append(h.Pools, *ph)
		}
	}
	return h, nil
}

// Commit closes what was not handed on, once the successor has taken the rest.
func (h *RegistryHandoff) Commit() {
	for _, p := range h.detached {
		p.Commit()
	}
}

// Restore puts every detached pool back, for a handoff the successor refused.
func (h *RegistryHandoff) Restore() {
	for _, p := range h.detached {
		p.Restore()
	}
	h.detached = nil
}

// Adoption is a successor's children, held until the handoff commits. A child
// is served by the pool with the same identity; one whose pool this
// configuration no longer has is closed after commit, and its server starts
// again from the new definition on its next use. An upstream HTTP session
// whose pool is gone has no child to close: it is ended upstream after
// commit instead, from the endpoint its handoff carries.
type Adoption struct {
	all     []*pool.Adopted
	byPool  map[*pool.Pool][]*pool.Adopted
	seq     map[*pool.Pool]int
	orphans []*pool.Adopted
	// supervisedOrphans are handed supervised lanes no pool here
	// adopted: their server changed or left the config, so the child
	// under its supervisor is stale. Nothing is contacted about them
	// until Attach, so Abort leaves them entirely with the
	// predecessor, which still owns them.
	supervisedOrphans []supervisedOrphan
	// orphanSessions are handed HTTP sessions no pool here adopted. Nothing
	// is contacted about them until Attach, so Abort leaves them entirely
	// with the predecessor, which still owns them.
	orphanSessions []orphanSession
	// orphanWG tracks the background orphan-session terminations Attach
	// starts, so a caller can wait for them with WaitOrphanTerminations:
	// tests, and the Server's orderly exit paths (Server keeps the
	// committed Adoption for exactly this, #298).
	orphanWG sync.WaitGroup
	logf     func(string, ...any)
}

// orphanSession is a handed upstream HTTP session whose pool this
// configuration no longer has, with the name of the server it belonged to.
type orphanSession struct {
	server string
	h      pool.InstanceHandoff
}

// supervisedOrphan is a handed supervised lane whose pool this
// configuration no longer has: the server name and scope key naming
// the supervisor socket the stale child runs under. Legacy follow
// lanes run under their own socket (see pool.SupervisedLaneKey), so
// the lane flag travels too.
type supervisedOrphan struct {
	server string
	key    string
	legacy bool
}

// Adopt takes the children of every handed pool. Nothing reads them yet.
func (r *Registry) Adopt(hs []pool.PoolHandoff) (*Adoption, error) {
	byID := map[string]*pool.Pool{}
	for _, name := range r.order {
		p := r.pools[name]
		if _, ok := byID[p.PoolID()]; !ok {
			byID[p.PoolID()] = p
		}
	}
	a := &Adoption{byPool: map[*pool.Pool][]*pool.Adopted{}, seq: map[*pool.Pool]int{}, logf: r.logf}
	for _, h := range hs {
		ads, err := pool.Adopt(h.Instances)
		a.all = append(a.all, ads...)
		if err != nil {
			a.Abort()
			return nil, err
		}
		p, ok := byID[h.PoolID]
		if !ok {
			a.orphans = append(a.orphans, ads...)
			for _, in := range h.Instances {
				if in.HTTP != nil {
					a.orphanSessions = append(a.orphanSessions, orphanSession{server: h.Server, h: in})
				}
				if in.Supervised {
					// A changed or removed server's old child runs
					// on, under its supervisor: nobody adopts it,
					// so Attach terminates it there.
					a.supervisedOrphans = append(a.supervisedOrphans, supervisedOrphan{server: h.Server, key: in.Key, legacy: in.LegacyLane})
				}
			}
			continue
		}
		sessions, err := p.AdoptSessions(h.Instances)
		a.all = append(a.all, sessions...)
		if err != nil {
			a.Abort()
			return nil, err
		}
		ads = append(ads, sessions...)
		a.byPool[p] = append(a.byPool[p], ads...)
		if h.Seq > a.seq[p] {
			a.seq[p] = h.Seq
		}
	}
	return a, nil
}

// Abort gives every adopted child back, for a handoff that did not commit.
// Orphan sessions were never contacted, so there is nothing to give back:
// the predecessor keeps serving them.
func (a *Adoption) Abort() {
	for _, ad := range a.all {
		ad.Abort()
	}
	a.all = nil
	a.orphanSessions = nil
}

// Attach serves the adopted children and closes the orphans. Call it only
// after the handoff has committed.
//
// Orphaned upstream sessions are ended in the background, not here: each
// DELETE is bounded only by the per-session delete timeout (5s), so running
// them synchronously in series would delay the successor daemon serving by
// N x 5s — worst exactly when an upstream is unreachable and the DELETE can
// never succeed (#291). Termination is best-effort (a failure already just
// logs upstream.session.orphaned and leaves the session to expire), so
// nothing about correctness needs it done before serving — only the abort
// path needs no DELETE before commit, and Attach runs strictly after
// commit, so starting the sends here preserves that.
func (a *Adoption) Attach() {
	for p, ads := range a.byPool {
		p.Attach(ads, a.seq[p])
	}
	for _, o := range a.orphans {
		o.Close()
	}
	for _, o := range a.supervisedOrphans {
		// The lane's server changed or left the config: its old
		// child is stale, and the successor's pool (if any) must
		// spawn fresh rather than adopt it. Terminate it under
		// its supervisor now that the handoff has committed —
		// the predecessor has detached, so nothing else serves
		// it. A missing socket just means it is already gone.
		sock := mcpclient.SupervisorSocketPathForInstance(mcpclient.StateDir(), o.server, pool.SupervisedLaneKey(o.key, o.legacy))
		if err := mcpclient.TerminateSupervisorSocket(sock); err != nil && a.logf != nil {
			a.logf("takeover: terminating stale supervised child for %s: %v", o.server, err)
		}
	}
	for _, s := range a.orphanSessions {
		a.orphanWG.Add(1)
		go func(s orphanSession) {
			defer a.orphanWG.Done()
			pool.TerminateOrphanedSession(s.server, s.h)
		}(s)
	}
	a.orphanSessions = nil
}

// WaitOrphanTerminations blocks until every background orphan-session
// termination Attach started has finished. Serving never waits for it;
// the Server's orderly exit paths do (bounded, via
// Server.waitOrphanTerminations), and so do tests.
func (a *Adoption) WaitOrphanTerminations() {
	a.orphanWG.Wait()
}

// AdoptSupervised runs the #9962 state-dir scan over every pool: a
// supervised server whose supervisor is already running (left behind
// by a previous daemon) is adopted by attach, so its child keeps its
// PID across the restart and no duplicate is spawned. Called at daemon
// startup and again after a takeover commits — at startup a live
// predecessor may still be attached (the scan then no-ops); after the
// commit the predecessor has detached and the attach succeeds.
func (r *Registry) AdoptSupervised(ctx context.Context) {
	r.mu.RLock()
	seen := map[*pool.Pool]bool{}
	var pools []*pool.Pool
	for _, name := range r.order {
		if p := r.pools[name]; !seen[p] {
			seen[p] = true
			pools = append(pools, p)
		}
	}
	r.mu.RUnlock()
	for _, p := range pools {
		p.AdoptRunningSupervised(ctx)
	}
}

// AdoptSupervisedAfterTakeover is the takeover-commit adoption scan with a
// bounded retry for the transient already-attached window. The predecessor
// closes its supervisor socket on commit, but the supervisor clears the
// attachment asynchronously on EOF, so an immediate scan can be refused
// while the detach is still in flight. Only that refusal is retried, for
// ~2s at 50ms intervals (well under limits.takeoverTimeout=15s); every
// other quiet no-op stays quiet, and a persistent already-attached
// refusal expires to the same quiet no-op the startup scan would do.
//
// Supervised lanes travel in the handoff as metadata (see
// pool.InstanceHandoff.Supervised): each is attached per key, so session
// and other non-default lanes survive the takeover with the same child.
// A pool with no supervised handoff still gets the legacy default-lane
// scan, which covers a predecessor that detached before supervised
// metadata existed.
func (r *Registry) AdoptSupervisedAfterTakeover(ctx context.Context, hs []pool.PoolHandoff) {
	byID := map[string]*pool.Pool{}
	r.mu.RLock()
	for _, name := range r.order {
		if p := r.pools[name]; byID[p.PoolID()] == nil {
			byID[p.PoolID()] = p
		}
	}
	r.mu.RUnlock()
	claimed := map[*pool.Pool]bool{}
	for _, h := range hs {
		p, ok := byID[h.PoolID]
		if !ok {
			continue
		}
		for _, in := range h.Instances {
			if !in.Supervised {
				continue
			}
			claimed[p] = true
			rs := config.ResolvedScope{Key: in.Key, Root: in.Root, Cwd: in.Cwd}
			adoptSupervisedWithDetachRetry(ctx, func(ctx context.Context) (bool, bool) {
				return p.AdoptSupervisedLane(ctx, rs, in.LegacyLane)
			})
		}
	}
	r.mu.RLock()
	seen := map[*pool.Pool]bool{}
	var pools []*pool.Pool
	for _, name := range r.order {
		if p := r.pools[name]; !seen[p] {
			seen[p] = true
			pools = append(pools, p)
		}
	}
	r.mu.RUnlock()
	for _, p := range pools {
		if claimed[p] {
			continue
		}
		adoptSupervisedWithDetachRetry(ctx, p.AdoptRunningSupervisedDetailed)
	}
}

// adoptDetachRetryBudget caps the transient-detach retry; the interval
// sets the poll cadence. Both stay well under takeoverTimeout.
const (
	adoptDetachRetryBudget   = 2 * time.Second
	adoptDetachRetryInterval = 50 * time.Millisecond
)

// adoptSupervisedWithDetachRetry runs adopt once, retrying only the
// transient already-attached refusal until budget expiry or ctx done.
// It reports whether the pool was adopted.
func adoptSupervisedWithDetachRetry(ctx context.Context, adopt func(context.Context) (bool, bool)) bool {
	adopted, alreadyAttached := adopt(ctx)
	if adopted || !alreadyAttached {
		return adopted
	}
	deadline := time.Now().Add(adoptDetachRetryBudget)
	for alreadyAttached && time.Now().Before(deadline) {
		select {
		case <-ctx.Done():
			return false
		case <-time.After(adoptDetachRetryInterval):
		}
		adopted, alreadyAttached = adopt(ctx)
		if adopted || !alreadyAttached {
			return adopted
		}
	}
	return false
}

// ReapOrphanedSupervisors runs the reap half of the #9963
// adopt-or-reap scan at daemon startup: a supervisor socket no
// configured supervised server maps to belongs to a server that
// left the config (or opted out), so its supervisor and child are
// terminated and the reap logged; a socket whose supervisor refuses
// (a daemon attached, a protocol-version mismatch) and stale socket
// files are logged and left or removed accordingly. See
// mcpclient.ScanSupervisorSockets.
func (r *Registry) ReapOrphanedSupervisors() {
	var configured []string
	for _, name := range r.Names() {
		if v, ok := r.View(name); ok && v.Stdio() && v.Mcpx.SupervisedEnabled() {
			configured = append(configured, name)
		}
	}
	mcpclient.ScanSupervisorSockets(mcpclient.StateDir(), configured, r.logf)
}
