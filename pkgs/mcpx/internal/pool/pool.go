// Package pool manages the live MCP server processes behind each namespace.
//
// The central problem it solves is that MCP servers fall into two very
// different classes:
//
//   - Stateless (search, docs, database). One process can serve any number of
//     concurrent callers because JSON-RPC ids multiplex cleanly. Mode "shared".
//
//   - Stateful (chrome-devtools and friends). The server holds a browser, a
//     selected page, a scroll position. Two agents interleaving calls on one
//     process corrupt each other. Modes "pooled" and "session" give each
//     caller its own process.
//
// "session" mode is the important one: a lease is pinned to a session key for
// the whole script run, so navigate -> snapshot -> click all reach the same
// browser, while a concurrent run gets a different browser entirely.
package pool

import (
	"context"
	"crypto/rand"
	"encoding/hex"
	"encoding/json"
	"errors"
	"fmt"
	"github.com/developing-today/code/pkgs/mcpx/internal/defaults"
	"net/url"
	"os"
	"sort"
	"strings"
	"sync"
	"sync/atomic"
	"syscall"
	"time"

	"github.com/developing-today/code/pkgs/mcpx/internal/config"
	"github.com/developing-today/code/pkgs/mcpx/internal/mcpauth"
	"github.com/developing-today/code/pkgs/mcpx/internal/mcpclient"
)

const maxSchemaTTL = 24 * time.Hour

// Version reported to MCP servers during initialize.
var Version = "dev"

// ownServerName is the identity mcpx's own MCP server reports in
// serverInfo during initialize: internal/cli builds it with this name
// (mcpserver.New(backend, "mcpx", version)). An upstream answering to
// it is this daemon, another mcpx, or a proxy of one (#1877).
const ownServerName = "mcpx"

// OwnInstanceID is the instance identity of the MCP server this
// process itself serves, wired by internal/cli at startup (#1877).
// The self-connection guard compares an mcpx-claiming upstream's
// reported instance against it: equal is this very daemon (refuse),
// different is a distinct mcpx daemon (admit -- self-composition
// across daemons is a topology #1877 wants made safe, not banned).
// Empty means this process has not wired an identity, and the guard
// falls back to convicting on the aggregate surface alone.
var OwnInstanceID string

// Trace, when set, receives a line per tool call naming the instance that
// served it. The daemon wires this to its logger when MCPX_TRACE is set.
var Trace func(string, ...any)

// Lifecycle, when set, receives structured lifecycle events: a server
// starting, stopping or being reaped. Reported here rather than inferred from
// logs so that a consumer sees the same events the pool acted on.
var Lifecycle func(event string, attrs map[string]any)

func lifecycle(event string, attrs map[string]any) {
	if Lifecycle != nil {
		Lifecycle(event, attrs)
	}
}

// Instance is one live MCP server process (or remote session).
type Instance struct {
	ID        string
	Client    *mcpclient.Client
	transport mcpclient.Transport

	// key is the scope-resolved identity this instance serves. One live
	// instance per distinct key.
	key string
	// holders counts current callers. Exclusive sharing admits one; shared
	// sharing admits any number.
	holders   int
	lastUsed  time.Time
	startedAt time.Time
	trace     string
	calls     atomic.Int64
	// questions pending on this instance pause its calls' budgets.
	questions questions
	// eraSource is how this instance's era was settled: probe, cache or
	// forced.
	eraSource string
	// legacyLane marks an instance started legacy-only for legacy callers
	// under protocol: follow. Such an instance never serves a modern
	// caller, who would lose the modern session for nothing.
	legacyLane bool
	// subscribed are the resource URIs this instance has been asked to
	// report updates for. See watch.go.
	subMu      sync.Mutex
	subscribed map[string]bool
	monitored  bool
	// retiring marks a child being taken out of service, for a changed
	// executable or a restart: no new caller may take it, and it leaves the
	// list once it is closed. Guarded by Pool.mu. See execwatch.go.
	retiring bool
	// swapped is set, to how the child is being taken down, just before it is
	// closed, so a call that fails on it can say why.
	swapped atomic.Pointer[string]
}

// Trace is this instance's identifier, carried by every record about it.
func (i *Instance) Trace() string { return i.trace }

// PID returns the child process id for stdio instances, 0 otherwise.
func (i *Instance) PID() int {
	if st, ok := i.transport.(*mcpclient.StdioTransport); ok {
		return st.PID()
	}
	return 0
}

// Lease is a borrowed instance. Release must be called exactly once.
type Lease struct {
	inst *Instance
	pool *Pool
	done bool
}

// Client is the MCP session behind the lease.
func (l *Lease) Client() *mcpclient.Client { return l.inst.Client }

// InstanceID identifies the process serving this lease.
func (l *Lease) InstanceID() string { return l.inst.ID }

// Release returns the instance to the pool.
func (l *Lease) Release() {
	if l == nil || l.done {
		return
	}
	l.done = true
	l.pool.release(l.inst)
}

// Pool owns every instance of a single configured server.
type Pool struct {
	// Hooks receive what this pool's servers volunteer.
	Hooks *Hooks

	cfg *config.Resolved
	// exec is the identity of the program the children run: cfg.ExecIdentity()
	// as of New, then whatever the exec watch last found. execStamp is the
	// stat it was worked out from, and execMissing is set while the file is
	// gone. execMu guards all three. See PoolID and execwatch.go.
	execMu      sync.Mutex
	exec        string
	execStamp   config.ExecStamp
	execMissing bool
	// watching says the exec watch goroutine is running; it starts with the
	// pool's first live child. execEvery is the interval it was made with and
	// execStop ends it when the pool closes.
	watching  bool
	execEvery time.Duration
	execStop  chan struct{}

	// draining counts the children a lazy restart is closing in the
	// background. See WaitDrained.
	draining sync.WaitGroup

	// reqLimiter is this pool's one per-server rate-limit budget for
	// sampling and elicitation (#2192), built lazily from the hooks'
	// limits and handed to every client this pool creates. reqLimMu
	// guards it alone: installServerHooks is called with p.mu held on
	// the handoff path, so this must not take p.mu.
	reqLimMu   sync.Mutex
	reqLimiter *mcpclient.ServerRequestLimiter

	mu        sync.Mutex
	cond      *sync.Cond
	instances []*Instance
	starting  int
	// startingLegacy is how many of starting are legacy-lane starts.
	startingLegacy int
	seq            int
	closed         bool
	// parked and handed hold what a handoff took out of service; see Detach.
	parked, handed []*Instance

	flightMu sync.Mutex
	inflight map[string]int

	// schema cache
	schemaMu     sync.RWMutex
	tools        []mcpclient.Tool
	resources    []mcpclient.Resource
	prompts      []mcpclient.Prompt
	instructions string
	schemaAt     time.Time
	schemaErr    error
	// schemaDeadline is when a modern upstream's ttlMs hints say the
	// cached catalogue goes stale; zero means "never expires" (legacy
	// upstreams; disk seeds carry the deadline they were saved with).
	// schemaPrivate records
	// that some fetched list declared cacheScope "private" (#1601 Phase 1
	// stores the scope; enforcing it is Phase 2).
	schemaDeadline time.Time
	schemaPrivate  bool
	// refreshMu serialises catalogue refreshes so concurrent readers of
	// an expired cache collapse into one re-list instead of each running
	// their own (#1602: Acquire hands out concurrent leases, so nothing
	// else serialised this).
	refreshMu sync.Mutex

	// watched counts, per upstream resource URI, the subscribers that want
	// its updates; watchKey is the scope key they were resolved under.
	watchMu  sync.Mutex
	watched  map[string]int
	watchKey string

	// noLegacy is set once a legacy-only start failed under protocol:
	// follow; legacy callers then share the modern session.
	noLegacy bool

	lastErr   error
	failCount int
	// cooldownUntil throttles restart storms after repeated start failures.
	cooldownUntil time.Time
}

// New creates an empty pool. No process is started until first use.
func New(cfg *config.Resolved) *Pool {
	// The stamp is taken with the identity, before either is used, so a change
	// that lands after this is seen by the first check.
	st, id, _ := cfg.ExecState()
	p := &Pool{cfg: cfg, exec: id, execStamp: st, execEvery: ExecCheckInterval, execStop: make(chan struct{})}
	p.cond = sync.NewCond(&p.mu)
	return p
}

// Config exposes the resolved server config.
func (p *Pool) Config() *config.Resolved { return p.cfg }

// Name is the configured server name.
func (p *Pool) Name() string { return p.cfg.Name }

// Namespace is the TypeScript namespace for this server.
func (p *Pool) Namespace() string { return p.cfg.Namespace }

var errClosed = errors.New("pool closed")

// Acquire borrows an instance for a resolved scope key.
//
// The key decides *which* process; Sharing decides whether that process may
// serve more than one caller at once. Those are independent, which is why they
// are separate config axes.
func (p *Pool) Acquire(ctx context.Context, key string) (*Lease, error) {
	exclusive := p.cfg.Sharing == config.SharingExclusive
	legacy := p.followLegacy(ctx)

	p.mu.Lock()
	for {
		if p.closed {
			p.mu.Unlock()
			return nil, errClosed
		}
		p.reapDeadLocked()

		// An instance already serving this key is the only correct choice:
		// the key is the caller's identity, and a second process would mean a
		// second browser, a second index, a second anything.
		if in := p.findLaneLocked(key, legacy); in != nil {
			if !exclusive || in.holders == 0 {
				in.holders++
				in.lastUsed = time.Now()
				p.mu.Unlock()
				return &Lease{inst: in, pool: p}, nil
			}
			// Exclusive and busy: queue rather than start a rival process.
			if err := p.waitLocked(ctx); err != nil {
				p.mu.Unlock()
				return nil, err
			}
			continue
		}

		if p.laneSizeLocked(legacy) < p.cfg.Max {
			if cd := p.cooldownUntil; time.Now().Before(cd) {
				err := p.lastErr
				p.mu.Unlock()
				return nil, fmt.Errorf("server %q is in restart cooldown for %s: %w",
					p.cfg.Name, time.Until(cd).Truncate(time.Millisecond), err)
			}
			p.starting++
			if legacy {
				p.startingLegacy++
			}
			p.mu.Unlock()

			in, err := p.startLane(ctx, legacy)

			p.mu.Lock()
			p.starting--
			if legacy {
				p.startingLegacy--
			}
			if err != nil && legacy {
				// The server has no legacy session to offer. Remember that,
				// and serve this caller from the modern one as before.
				p.noLegacy = true
				legacy = false
				p.cond.Broadcast()
				continue
			}
			if err != nil {
				p.failCount++
				p.lastErr = err
				backoff := time.Duration(p.failCount) * defaults.RestartBackoffStep
				if backoff > 30*time.Second {
					backoff = defaults.RestartBackoffMax
				}
				p.cooldownUntil = time.Now().Add(backoff)
				p.cond.Broadcast()
				p.mu.Unlock()
				return nil, err
			}
			p.failCount = 0
			p.lastErr = nil
			p.cooldownUntil = time.Time{}
			// Another caller may have created this key while the lock was
			// released; keep theirs and retire the duplicate.
			if dup := p.findLaneLocked(key, legacy); dup != nil {
				p.cond.Broadcast()
				p.mu.Unlock()
				go in.Client.Close()
				p.mu.Lock()
				continue
			}
			in.key = key
			in.holders = 1
			in.lastUsed = time.Now()
			p.instances = append(p.instances, in)
			p.watchExecLocked()
			p.cond.Broadcast()
			p.mu.Unlock()
			return &Lease{inst: in, pool: p}, nil
		}

		// At capacity. An idle instance serving a key nobody is using can be
		// retired to make room, which is what keeps a per-call scope from
		// deadlocking at Max.
		if in := p.evictableLocked(legacy); in != nil {
			p.removeLocked(in)
			p.mu.Unlock()
			_ = in.Client.Close()
			p.mu.Lock()
			continue
		}

		if err := p.waitLocked(ctx); err != nil {
			p.mu.Unlock()
			return nil, err
		}
	}
}

// followLegacy reports whether this acquisition should get a legacy-only
// session: the server is configured protocol: follow, the caller speaks a
// legacy revision, and the server has not already refused initialize.
func (p *Pool) followLegacy(ctx context.Context) bool {
	if p.Preference() != mcpclient.PreferFollow || !mcpclient.CallerLegacy(ctx) {
		return false
	}
	p.mu.Lock()
	defer p.mu.Unlock()
	return !p.noLegacy
}

// findLaneLocked returns the instance serving key for a caller of the given
// era. A legacy caller takes any legacy session, including the default one
// of a server that only speaks legacy; a modern caller never takes one
// started legacy-only for somebody else.
func (p *Pool) findLaneLocked(key string, legacy bool) *Instance {
	for _, in := range p.instances {
		if in.key != key || in.retiring {
			continue
		}
		if legacy && (in.legacyLane || in.Client.Era == mcpclient.EraLegacy) || !legacy && !in.legacyLane {
			return in
		}
	}
	return nil
}

// findLocked returns the instance serving key, if any.
func (p *Pool) findLocked(key string) *Instance {
	for _, in := range p.instances {
		if in.key == key && !in.retiring {
			return in
		}
	}
	return nil
}

// laneSizeLocked counts the instances, running and starting, in one lane.
//
// Max bounds each lane separately. A legacy-lane instance is a second
// session kind for the same callers, not a rival for their slot: with a
// global scope (Max 1) a shared lane would mean a legacy call in progress
// stalls every modern call, and the reverse.
func (p *Pool) laneSizeLocked(legacy bool) int {
	n := 0
	for _, in := range p.instances {
		if in.legacyLane == legacy {
			n++
		}
	}
	if legacy {
		return n + p.startingLegacy
	}
	return n + p.starting - p.startingLegacy
}

// evictableLocked picks the least recently used instance with no holders
// in the given lane.
func (p *Pool) evictableLocked(legacy bool) *Instance {
	var best *Instance
	for _, in := range p.instances {
		if in.holders > 0 || in.legacyLane != legacy || in.retiring {
			continue
		}
		if best == nil || in.lastUsed.Before(best.lastUsed) {
			best = in
		}
	}
	return best
}

func (p *Pool) removeLocked(target *Instance) {
	kept := p.instances[:0]
	for _, in := range p.instances {
		if in != target {
			kept = append(kept, in)
		}
	}
	p.instances = kept
}

// waitLocked blocks on the condition variable but returns early if ctx ends.
func (p *Pool) waitLocked(ctx context.Context) error {
	if err := ctx.Err(); err != nil {
		return fmt.Errorf("waiting for a free %q instance (max=%d): %w", p.cfg.Name, p.cfg.Max, err)
	}
	// Wake the waiter when ctx ends. AfterFunc's stop does not wait for a
	// callback already running, and that is the point: this used a goroutine
	// that took p.mu to broadcast, and waited for it after Wait returned --
	// holding p.mu. A broadcast from elsewhere racing the cancellation left
	// the waiter holding the lock while the helper blocked on it, and the
	// pool was deadlocked for good. A late callback here just broadcasts
	// once the lock is free, which wakes nobody who minds.
	stop := context.AfterFunc(ctx, func() {
		p.mu.Lock()
		p.cond.Broadcast()
		p.mu.Unlock()
	})
	p.cond.Wait()
	stop()
	if err := ctx.Err(); err != nil {
		return fmt.Errorf("waiting for a free %q instance (max=%d): %w", p.cfg.Name, p.cfg.Max, err)
	}
	return nil
}

func (p *Pool) release(in *Instance) {
	p.mu.Lock()
	if in.holders > 0 {
		in.holders--
	}
	in.lastUsed = time.Now()
	p.cond.Broadcast()
	p.mu.Unlock()
}

// ReleaseKey stops the instance serving a key, if it is idle. Used when a
// script exits so a browser goes back immediately instead of waiting out the
// idle timer.
func (p *Pool) ReleaseKey(key string) int {
	if key == "" {
		return 0
	}
	p.mu.Lock()
	in := p.findLocked(key)
	if in == nil || in.holders > 0 {
		p.mu.Unlock()
		return 0
	}
	p.removeLocked(in)
	p.cond.Broadcast()
	p.mu.Unlock()
	p.stopped(in, "released")
	_ = in.Client.Close()
	return 1
}

func (p *Pool) reapDeadLocked() {
	kept := p.instances[:0]
	for _, in := range p.instances {
		if in.Client.Alive() {
			kept = append(kept, in)
			continue
		}
		if p.lastErr == nil {
			p.lastErr = in.Client.Err()
		}
		go in.Client.Close()
	}
	p.instances = kept
}

func (p *Pool) start(ctx context.Context) (*Instance, error) { return p.startLane(ctx, false) }

// installServerHooks wires what a server may ask of mcpx into the options its
// session is opened with. It must run before the handshake, where a server
// learns what mcpx can answer.
func (p *Pool) installServerHooks(opts *mcpclient.Options, ref *instanceRef) {
	h := p.Hooks
	if h == nil {
		return
	}
	if h.Elicit != nil {
		opts.OnServerRequest = func(ctx context.Context, method string, params json.RawMessage) (any, error) {
			// While a person or agent answers, the call that provoked the
			// question is not using its budget. See budget.go.
			if in := ref.ptr.Load(); in != nil {
				defer in.questions.asking()()
			}
			return h.Elicit(ctx, p.cfg.Name, ref.key(), method, params)
		}
	}
	if h.URLElicitation != nil {
		opts.OnURLElicitationRequired = func(ctx context.Context, elicitations []mcpclient.URLElicitation) (bool, error) {
			// Same budget pause as OnServerRequest: while a person or
			// agent answers, the interrupted call is not using its budget.
			if in := ref.ptr.Load(); in != nil {
				defer in.questions.asking()()
			}
			return h.URLElicitation(ctx, p.cfg.Name, ref.key(), elicitations)
		}
	}
	opts.Roots = h.Roots
	opts.ProbeTimeout = h.ProbeTimeout
	opts.ServerName = p.cfg.Name
	opts.RequestLimits = h.RequestLimits
	opts.RequestLimitsSet = h.RequestLimitsSet
	opts.ServerLimiter = p.serverLimiter()
}

// serverLimiter returns the pool's shared rate-limit budget, building
// it from the hooks' limits on first use. One pool serves one
// configured server, so the budget is keyed by that configuration by
// construction -- never by a name the server claims for itself.
func (p *Pool) serverLimiter() *mcpclient.ServerRequestLimiter {
	p.reqLimMu.Lock()
	defer p.reqLimMu.Unlock()
	if p.reqLimiter == nil {
		lim := mcpclient.DefaultRequestLimits()
		if p.Hooks != nil && (p.Hooks.RequestLimitsSet || p.Hooks.RequestLimits != (mcpclient.RequestLimits{})) {
			lim = p.Hooks.RequestLimits
		}
		p.reqLimiter = mcpclient.NewServerRequestLimiter(lim)
	}
	return p.reqLimiter
}

// subscribeServerEvents routes what a session volunteers to the pool's hooks.
// A session is a client-side object, so a successor subscribes its own.
func (p *Pool) subscribeServerEvents(cl *mcpclient.Client) {
	if h := p.Hooks; h != nil {
		cl.Subscribe(h.notifications(p.cfg.Name))
	}
}

// startLane starts an instance; legacy starts it legacy-only, for a legacy
// caller under protocol: follow.
func (p *Pool) startLane(ctx context.Context, legacy bool) (*Instance, error) {
	sctx, cancel := context.WithTimeout(ctx, p.cfg.StartTimeout)
	defer cancel()
	launched := time.Now()

	tr, err := p.dial(sctx)
	if err != nil {
		return nil, fmt.Errorf("server %q: %w", p.cfg.Name, err)
	}
	source := ""

	// The server-request handler and roots go in before the handshake, not
	// after it: the handshake is where capabilities are declared, and a
	// legacy server never asks again. Installed afterwards, sampling could
	// never be declared and the first roots/list could arrive before any
	// roots were set.
	pref := p.Preference()
	switch {
	case legacy:
		pref = mcpclient.ForceLegacy
	case pref == mcpclient.PreferFollow:
		pref = mcpclient.PreferModern
	}
	opts := mcpclient.Options{ClientName: "mcpx", ClientVersion: Version, Preference: pref}
	ref := &instanceRef{p: p}
	p.installServerHooks(&opts, ref)
	eraKey := Identity(p.cfg)
	var cached EraRecord
	var eras EraStore
	if h := p.Hooks; h != nil {
		eras = h.Eras
	}
	forced := pref == mcpclient.ForceLegacy || pref == mcpclient.ForceModern
	if eras != nil && !forced {
		if rec, ok := eras.Get(eraKey); ok {
			cached = rec
			opts.Cached = rec.Era
		}
	}
	var cl *mcpclient.Client
	if cached.Transport == TransportHTTPSSE && !p.cfg.Stdio() {
		// Last time this endpoint spoke only the deprecated HTTP+SSE
		// transport; going straight there saves the two refused POSTs.
		if cl, tr, err = p.connectSSE(sctx, tr, opts); err == nil {
			source = mcpclient.SourceCache
		} else if tr, err = p.dial(sctx); err != nil {
			return nil, fmt.Errorf("server %q: %w", p.cfg.Name, err)
		}
	}
	if cl == nil {
		cl, err = mcpclient.NewWithOptions(sctx, tr, opts)
		if err == nil {
			source = cl.Source
		}
	}
	var refused *mcpclient.LegacyHTTPRefusedError
	if errors.As(err, &refused) && !p.cfg.Stdio() {
		// Both a modern POST and a legacy initialize were refused with a
		// bare 400, 404 or 405: the signature of a server that speaks only
		// HTTP+SSE (2024-11-05). The fallback every later revision
		// describes is to GET the URL and expect an endpoint event.
		if cl, tr, err = p.connectSSE(sctx, tr, opts); err == nil {
			source = mcpclient.SourceProbe
			if cached.Era != "" && cached.Transport != TransportHTTPSSE {
				cl.CachedEraWrong = true
			}
		} else {
			err = fmt.Errorf("%v; http+sse fallback: %w", refused, err)
		}
	}
	if errors.Is(err, mcpclient.ErrClosedDuringProbe) {
		// A legacy server that exits on any first message but initialize.
		// The process is gone, so the only way to reach it is a new one
		// that hears initialize first -- and the cache is what keeps this
		// from happening on every start.
		_ = tr.Close()
		if tr, err = p.dial(sctx); err != nil {
			return nil, fmt.Errorf("server %q: %w", p.cfg.Name, err)
		}
		opts.Preference, opts.Cached = mcpclient.ForceLegacy, ""
		cl, err = mcpclient.NewWithOptions(sctx, tr, opts)
		source = mcpclient.SourceProbe
		if err == nil && cached.Era != "" && cached.Era != cl.Era {
			cl.CachedEraWrong = true
		}
	}
	if err != nil {
		_ = tr.Close()
		return nil, fmt.Errorf("server %q: %w", p.cfg.Name, p.oauthWaitError(err))
	}
	// Self-connection loop guard (#1877): see guardSelfConnection.
	if cl.ServerInfo.Name == ownServerName {
		if err := p.guardSelfConnection(sctx, cl, tr); err != nil {
			return nil, err
		}
	}
	if cl.CachedEraWrong {
		lifecycle("server.era.stale", map[string]any{
			"server": p.cfg.Name, "cached": string(cached.Era), "cachedAt": cached.At,
			"era": string(cl.Era), "negotiated": cl.Negotiated,
		})
	}
	if eras != nil && !forced && source != mcpclient.SourceCache {
		rec := EraRecord{Era: cl.Era, Version: cl.Negotiated, At: time.Now(), Source: source}
		if _, ok := tr.(*mcpclient.LegacySSETransport); ok {
			rec.Transport = TransportHTTPSSE
		}
		_ = eras.Put(eraKey, rec)
	}
	// Everything the server volunteers flows to whoever installed hooks: log
	// lines, progress, list changes, resource updates. Installed before the
	// instance is handed out, so nothing a server says in its first moments
	// is lost.
	p.subscribeServerEvents(cl)
	// Servers send nothing until asked, so a client that never sets a level
	// concludes a server emits no logs at all.
	if h := p.Hooks; h != nil && h.LogLevel != "" {
		_ = cl.SetLogLevel(sctx, h.LogLevel)
	}

	p.mu.Lock()
	p.seq++
	id := fmt.Sprintf("%s#%d", p.cfg.Name, p.seq)
	p.mu.Unlock()

	in := &Instance{
		ID:         id,
		Client:     cl,
		transport:  tr,
		trace:      newTraceID("srv"),
		startedAt:  time.Now(),
		lastUsed:   time.Now(),
		eraSource:  source,
		legacyLane: legacy,
	}
	ref.set(in)
	// A replacement instance -- after a restart, a crash, an eviction --
	// knows nothing of what its predecessor was subscribed to.
	p.resubscribe(sctx, in)
	lifecycle("server.start", map[string]any{
		"server": p.cfg.Name, "instance": in.ID, "pid": in.PID(),
		"trace": in.trace, "sharing": string(p.cfg.Sharing), "scope": string(p.cfg.Scope),
		"transport": transportName(p.cfg),
		"era":       string(cl.Era), "negotiated": cl.Negotiated, "eraSource": source,
		// Time to ready, not time to spawn: the event fires after the
		// handshake has answered, so this is when the server could first
		// take a call.
		"readyMs": float64(time.Since(launched).Microseconds()) / 1000,
	})
	return in, nil
}

// ownMetaTools are the tools mcpx's aggregate MCP surface always
// carries (mcpserver.Server.Tools, non-ExtrasOnly). Their presence in
// an upstream's catalogue is what makes that upstream mcpx itself
// rather than a namesake.
var ownMetaTools = map[string]bool{
	"mcpx_namespaces": true, "mcpx_catalog": true, "mcpx_types": true,
	"mcpx_search": true, "mcpx_call": true, "mcpx_exec": true,
	"mcpx_log": true, "mcpx_stats": true, "mcpx_registry": true,
	"mcpx_status": true, "mcpx_batch": true,
}

// ownReactiveTools are the activation pair mcpx's reactive surface
// (mcpserver.reactiveSurface) exposes before any tool is activated:
// request_tools searches and activates the aggregate's tools, and
// batch_call executes across them. A reactive aggregate initially
// lists neither the mcpx_* tools nor its upstreams' tools, so the
// pair -- both names together, never either alone -- is its identity
// independently of which tools happen to be activated right now
// (Codex review on PR 2975).
var ownReactiveTools = []string{"request_tools", "batch_call"}

// guardSelfConnection decides whether an upstream that identifies as
// mcpx may be served (#1877). Identity decides first, when both sides
// report one: an instance equal to this process's own is this daemon
// connecting to itself -- the loop that grows tools/list on every
// reload -- and is refused without ever listing its tools; a
// different instance is a distinct mcpx daemon, admitted as a peer
// (its catalogue is finite: the aggregate meta tools plus its own
// upstreams, none of them this daemon). When either side reports no
// instance -- an mcpx predating instance identity, or a process that
// never wired one -- identity cannot clear the claimant, so the
// aggregate surface decides instead (refuseIfAggregate): a claimant
// serving mcpx's meta tools with no distinguishable identity is
// treated as self, the safe side of the ambiguity.
func (p *Pool) guardSelfConnection(ctx context.Context, cl *mcpclient.Client, tr mcpclient.Transport) error {
	own, peer := OwnInstanceID, cl.ServerInfo.Instance
	if own != "" && peer != "" {
		if peer != own {
			lifecycle("server.peer_mcpx_admitted", map[string]any{
				"server": p.cfg.Name, "serverInfoName": cl.ServerInfo.Name,
				"serverInfoVersion": cl.ServerInfo.Version, "instance": peer,
			})
			return nil
		}
		err := fmt.Errorf("server %q: refusing self-connection: upstream is this daemon itself (instance %q matches this process); an entry pointing at mcpx's own MCP endpoint would proxy mcpx's tools back into mcpx and grow tools/list on every reload (#1877)", p.cfg.Name, peer)
		lifecycle("server.self_connection_refused", map[string]any{
			"server": p.cfg.Name, "serverInfoName": cl.ServerInfo.Name,
			"serverInfoVersion": cl.ServerInfo.Version, "instance": peer, "via": "instance",
		})
		p.dropSchemas(err)
		_ = cl.Close()
		_ = tr.Close()
		return err
	}
	return p.refuseIfAggregate(ctx, cl, tr)
}

// refuseIfAggregate lists a self-claiming upstream's tools once and
// refuses the instance when the aggregate meta surface is among them
// (#1877). It also drops any cached catalogue for this server: an
// installation upgraded with a self entry already in its schema cache
// would otherwise keep advertising the proxied tools indefinitely.
// A claimant whose tools cannot be listed is refused too -- the
// identity check could not clear it, and a catalogue that cannot be
// listed could not be served either.
func (p *Pool) refuseIfAggregate(ctx context.Context, cl *mcpclient.Client, tr mcpclient.Transport) error {
	tools, lerr := cl.ListTools(ctx)
	meta := ""
	if lerr == nil {
		names := make(map[string]bool, len(tools))
		for _, t := range tools {
			names[t.Name] = true
			if ownMetaTools[t.Name] && meta == "" {
				meta = t.Name
			}
		}
		if meta == "" {
			reactive := true
			for _, n := range ownReactiveTools {
				if !names[n] {
					reactive = false
					break
				}
			}
			if reactive {
				meta = "request_tools+batch_call (reactive surface)"
			}
		}
		if meta == "" {
			lifecycle("server.namesake_admitted", map[string]any{
				"server": p.cfg.Name, "serverInfoName": cl.ServerInfo.Name,
				"serverInfoVersion": cl.ServerInfo.Version,
			})
			return nil
		}
	}
	err := fmt.Errorf("server %q: refusing self-connection: upstream identifies as %q (mcpx itself) and serves mcpx's aggregate surface (%s); an entry pointing at mcpx's own MCP endpoint would proxy mcpx's tools back into mcpx and grow tools/list on every reload (#1877)", p.cfg.Name, cl.ServerInfo.Name, meta)
	if lerr != nil {
		err = fmt.Errorf("server %q: refusing self-connection: upstream identifies as %q (mcpx itself) and its tools could not be listed to clear the identity check: %w (#1877)", p.cfg.Name, cl.ServerInfo.Name, lerr)
	}
	lifecycle("server.self_connection_refused", map[string]any{
		"server": p.cfg.Name, "serverInfoName": cl.ServerInfo.Name,
		"serverInfoVersion": cl.ServerInfo.Version, "metaTool": meta,
	})
	p.dropSchemas(err)
	_ = cl.Close()
	_ = tr.Close()
	return err
}

// dropSchemas forgets the cached catalogue and records why, so a
// refused server stops being advertised from cache (#1877).
func (p *Pool) dropSchemas(err error) {
	p.schemaMu.Lock()
	p.tools, p.resources, p.prompts = nil, nil, nil
	p.instructions, p.schemaAt, p.schemaDeadline = "", time.Time{}, time.Time{}
	p.schemaErr = err
	p.schemaMu.Unlock()
}

// connectSSE closes tr and connects over the HTTP+SSE transport instead,
// legacy-only: that transport predates the modern era.
func (p *Pool) connectSSE(ctx context.Context, tr mcpclient.Transport, opts mcpclient.Options) (*mcpclient.Client, mcpclient.Transport, error) {
	_ = tr.Close()
	ho, err := p.httpOptions()
	if err != nil {
		return nil, tr, err
	}
	sse, err := mcpclient.NewLegacySSE(ctx, ho)
	if err != nil {
		return nil, tr, err
	}
	opts.Preference, opts.Cached = mcpclient.ForceLegacy, ""
	cl, err := mcpclient.NewWithOptions(ctx, sse, opts)
	if err != nil {
		_ = sse.Close()
		return nil, tr, err
	}
	return cl, sse, nil
}

// dial creates the transport for one instance. ctx is the caller's
// startup context: it bounds any nix: program-spec resolution at spawn,
// so a slow build cannot outlive the startup budget that owns it.
func (p *Pool) dial(ctx context.Context) (mcpclient.Transport, error) {
	auth, err := p.resolveAuth()
	if err != nil {
		return nil, err
	}
	if p.cfg.Stdio() && p.cfg.Mcpx != nil && p.cfg.Mcpx.Supervised {
		return mcpclient.NewSupervised(mcpclient.SupervisedOptions{
			Name:       p.cfg.Name,
			Command:    p.cfg.Command,
			Args:       p.cfg.Args,
			Env:        overlay(overlay(p.cfg.Env, p.runtimeEnv()), auth.Env),
			Cwd:        p.cfg.Cwd,
			InheritEnv: true,
			Context:    ctx,
		})
	}
	if p.cfg.Stdio() {
		return mcpclient.NewStdio(mcpclient.StdioOptions{
			Command:    p.cfg.Command,
			Args:       p.cfg.Args,
			Env:        overlay(overlay(p.cfg.Env, p.runtimeEnv()), auth.Env),
			Cwd:        p.cfg.Cwd,
			InheritEnv: true,
			Context:    ctx,
		})
	}
	ho, err := p.httpOptions()
	if err != nil {
		return nil, err
	}
	return mcpclient.NewHTTP(ho)
}

// variable is the value this server's configuration reads for name: the value
// the daemon holds in memory for it, else the daemon's environment. Credentials
// declared with auth are read this way, so an HTTP server gets the same secret
// a stdio child does.
func (p *Pool) variable(name string) string {
	if p.cfg.Runtime != nil {
		if v, ok := p.cfg.Runtime(name); ok && v != "" {
			return v
		}
	}
	return os.Getenv(name)
}

// runtimeEnv is the values the daemon holds in memory for the secrets this
// server requires, for the environment of the child it is about to start. Only
// the names the server asks for are passed: a child is not handed every secret
// the daemon has, and a value is never written into the configuration, which
// is shown to people.
func (p *Pool) runtimeEnv() map[string]string {
	if p.cfg.Runtime == nil {
		return nil
	}
	var out map[string]string
	for _, name := range p.cfg.SecretNames() {
		if v, ok := p.cfg.Runtime(name); ok && v != "" {
			if out == nil {
				out = map[string]string{}
			}
			out[name] = v
		}
	}
	return out
}

// resolveAuth turns the server's `auth` block into the headers, query
// parameters and environment a connection carries. A credential that names an
// unset variable fails here, before a request, rather than as a 401 later.
func (p *Pool) resolveAuth() (*mcpauth.Resolved, error) {
	r, err := p.cfg.Auth.ResolveWith(p.variable)
	if err != nil {
		return nil, fmt.Errorf("%s: auth: %w", p.cfg.Name, err)
	}
	if len(r.Missing) > 0 {
		return nil, fmt.Errorf("%s: auth: %s not set in the daemon's environment",
			p.cfg.Name, strings.Join(r.Missing, ", "))
	}
	if r.NeedsOAuth {
		return nil, fmt.Errorf("%s: auth: the OAuth flow is not implemented; "+
			"use type bearer with a token you obtained, or header", p.cfg.Name)
	}
	return r, nil
}

// httpOptions is the URL and headers for an HTTP connection with the auth
// block applied. Auth headers are applied over `headers`, because a
// credential declared as auth is the more specific statement.
func (p *Pool) httpOptions() (mcpclient.HTTPOptions, error) {
	auth, err := p.resolveAuth()
	if err != nil {
		return mcpclient.HTTPOptions{}, err
	}
	u := p.cfg.URL
	if len(auth.Query) > 0 {
		parsed, err := url.Parse(u)
		if err != nil {
			return mcpclient.HTTPOptions{}, fmt.Errorf("%s: url: %w", p.cfg.Name, err)
		}
		q := parsed.Query()
		for k, v := range auth.Query {
			q.Set(k, v)
		}
		parsed.RawQuery = q.Encode()
		u = parsed.String()
	}
	return mcpclient.HTTPOptions{URL: u, Headers: overlay(p.cfg.Headers, auth.Headers)}, nil
}

// overlay returns base with over applied on top, without mutating either.
func overlay(base, over map[string]string) map[string]string {
	if len(over) == 0 {
		return base
	}
	out := make(map[string]string, len(base)+len(over))
	for k, v := range base {
		out[k] = v
	}
	for k, v := range over {
		out[k] = v
	}
	return out
}

// Schemas returns the cached tool and resource lists, fetching them on first
// call. While the cache is fresh every later call is a map read, which is
// what makes `mcpx ls` instant. A modern upstream's cache stops being fresh
// at schemaDeadline; a legacy (or disk-seeded) cache has no deadline and is
// served until Invalidate, exactly as before #1602.
func (p *Pool) Schemas(ctx context.Context) ([]mcpclient.Tool, []mcpclient.Resource, error) {
	p.schemaMu.RLock()
	if !p.schemaAt.IsZero() && (p.schemaDeadline.IsZero() || time.Now().Before(p.schemaDeadline)) {
		t, r, e := p.tools, p.resources, p.schemaErr
		p.schemaMu.RUnlock()
		return t, r, e
	}
	staleAt := p.schemaAt
	p.schemaMu.RUnlock()
	return p.refreshSchemas(ctx, &staleAt)
}

// RefreshSchemas re-reads tools/resources from a live instance.
//
// On failure of a FIRST fetch the cache timestamp is deliberately left
// unset. Recording a timestamp would make a server that never started
// look like one that genuinely exposes no tools, and that state would
// then be persisted and reloaded, permanently hiding the failure. A
// failed re-read of an expired cache is different (#1603): the stale
// catalogue is served, the error is recorded, and the timestamp and
// deadline are left untouched so the next read retries.
func (p *Pool) RefreshSchemas(ctx context.Context) ([]mcpclient.Tool, []mcpclient.Resource, error) {
	return p.refreshSchemas(ctx, nil)
}

// refreshSchemas does the re-list. When staleGen is non-nil it names the
// cache generation the caller found stale: if another goroutine already
// refreshed past that generation while this one waited on refreshMu, its
// result is served instead of listing again -- concurrent reads of one
// expired cache collapse into a single refresh. A nil staleGen is an
// explicit refresh and always re-lists.
func (p *Pool) refreshSchemas(ctx context.Context, staleGen *time.Time) ([]mcpclient.Tool, []mcpclient.Resource, error) {
	p.refreshMu.Lock()
	defer p.refreshMu.Unlock()
	if staleGen != nil {
		p.schemaMu.RLock()
		if !p.schemaAt.IsZero() && !p.schemaAt.Equal(*staleGen) {
			t, r, e := p.tools, p.resources, p.schemaErr
			p.schemaMu.RUnlock()
			return t, r, e
		}
		p.schemaMu.RUnlock()
	}
	// Reading a catalogue is not a caller's work, so it borrows the scope's
	// own key. For a global scope that is the shared instance everyone uses;
	// for anything narrower it is a throwaway, stopped again below so a
	// per-session server does not keep a process nobody asked for.
	key := p.schemaKey()
	lease, err := p.Acquire(ctx, key)
	if err != nil {
		return p.refreshFailed(staleGen, err)
	}
	defer lease.Release()

	cl := lease.Client()
	tools, toolsHints, terr := cl.ListToolsWithHints(ctx)
	if terr != nil {
		return p.refreshFailed(staleGen, fmt.Errorf("server %q tools/list: %w", p.cfg.Name, terr))
	}
	var resources []mcpclient.Resource
	var resHints mcpclient.CacheHints
	resFetched := false
	if cl.Supports("resources") {
		var rerr error
		resources, resHints, rerr = cl.ListResourcesWithHints(ctx)
		// As before, a partial list is kept even on error; it just
		// contributes no hint, so its freshness is not trusted.
		resFetched = rerr == nil
	}
	// Prompts are the part of MCP that is not tools: a server saying "here
	// is the wording that works for this". A server publishing a good one
	// has encoded expertise that would otherwise be rediscovered by whoever
	// writes the request, and returning an empty list -- which is what mcpx
	// did -- throws that away.
	var prompts []mcpclient.Prompt
	var promptHints mcpclient.CacheHints
	promptsFetched := false
	if cl.Supports("prompts") {
		var perr error
		prompts, promptHints, perr = cl.ListPromptsWithHints(ctx)
		// ListPrompts swallows method-not-found itself, so an error here
		// is not expected; treat any as "not fetched" for hint purposes.
		promptsFetched = perr == nil
		_ = promptsFetched
	}

	sort.Slice(tools, func(i, j int) bool { return tools[i].Name < tools[j].Name })
	sort.Slice(resources, func(i, j int) bool { return resources[i].Name < resources[j].Name })
	sort.Slice(prompts, func(i, j int) bool { return prompts[i].Name < prompts[j].Name })

	// Freshness (#1602): only a modern upstream's hints count. The
	// deadline is the earliest among the lists actually fetched in this
	// refresh: a list that declared no ttlMs -- or 0, or a negative --
	// makes the catalogue stale immediately; an unfetched kind (or one
	// whose list error was swallowed) contributes nothing. Positive modern
	// hints are capped before duration conversion to bound freshness and
	// prevent overflow. Legacy refreshes keep a zero deadline: their cache
	// never expires.
	fetchedAt := time.Now()
	var deadline time.Time
	var private bool
	if cl.Era == mcpclient.EraModern {
		minTTL := int64(-1) // sentinel: no fetched list yet
		consider := func(h mcpclient.CacheHints) {
			eff := int64(0)
			if h.HasTTL && h.TTLMs > 0 {
				eff = h.TTLMs
			}
			if minTTL < 0 || eff < minTTL {
				minTTL = eff
			}
			private = private || h.Private
		}
		consider(toolsHints)
		if resFetched {
			consider(resHints)
		}
		if promptsFetched {
			consider(promptHints)
		}
		if minTTL < 0 {
			minTTL = 0
		}
		if maxTTL := maxSchemaTTL.Milliseconds(); minTTL > maxTTL {
			minTTL = maxTTL
		}
		deadline = fetchedAt.Add(time.Duration(minTTL) * time.Millisecond)
	}

	p.schemaMu.Lock()
	p.prompts = prompts
	p.tools, p.resources, p.schemaErr, p.schemaAt = tools, resources, nil, fetchedAt
	p.schemaDeadline, p.schemaPrivate = deadline, private
	p.instructions = cl.Instructions
	p.schemaMu.Unlock()

	lease.Release()
	lease.done = true // Release is idempotent, but be explicit before ReleaseKey
	if p.cfg.Scope != config.ScopeGlobal {
		p.ReleaseKey(key)
	}
	return tools, resources, nil
}

// refreshFailed records a failed refresh (#1603). When the caller was
// re-reading an expired cache generation, the stale catalogue is served
// and nil is returned: the error lives in schemaErr (surfaced via
// Status), and schemaAt/schemaDeadline are left untouched so the next
// read retries the refresh. Otherwise — a failed first fetch — the
// error is returned and, per RefreshSchemas, no timestamp is recorded.
func (p *Pool) refreshFailed(staleGen *time.Time, err error) ([]mcpclient.Tool, []mcpclient.Resource, error) {
	p.schemaMu.Lock()
	defer p.schemaMu.Unlock()
	p.schemaErr = err
	if staleGen != nil && !p.schemaAt.IsZero() && p.schemaAt.Equal(*staleGen) {
		return p.tools, p.resources, nil
	}
	return nil, nil, err
}

// schemaKey is the instance a catalogue read borrows.
func (p *Pool) schemaKey() string {
	key, _ := p.cfg.Scope.Key(config.CallContext{CallID: "schema"})
	return key
}

// SetSchemas seeds the cache from disk so the daemon can answer discovery
// queries without starting a single child process. The deadline and
// scope are the ones the persisted entry carried (#1603), except that an
// overlong deadline is bounded to 24 hours after the fetch time. The
// deadline is absolute, so a seed whose deadline has already passed
// starts expired and the first Schemas call re-reads instead of serving
// it as fresh.
func (p *Pool) SetSchemas(tools []mcpclient.Tool, resources []mcpclient.Resource, prompts []mcpclient.Prompt, instructions string, at time.Time, deadline time.Time, private bool) {
	deadline = boundedSchemaDeadline(at, deadline)
	p.schemaMu.Lock()
	p.tools, p.resources, p.prompts = tools, resources, prompts
	p.instructions, p.schemaAt, p.schemaErr = instructions, at, nil
	p.schemaDeadline, p.schemaPrivate = deadline, private
	p.schemaMu.Unlock()
}

// boundedSchemaDeadline keeps disk-seeded caches under the same freshness
// ceiling as deadlines computed from modern upstream ttlMs hints. Older
// cache files may already contain an unbounded deadline; zero (legacy) and
// already-earlier deadlines retain their existing semantics.
func boundedSchemaDeadline(at, deadline time.Time) time.Time {
	if !at.IsZero() && !deadline.IsZero() && deadline.After(at.Add(maxSchemaTTL)) {
		return at.Add(maxSchemaTTL)
	}
	return deadline
}

// Instructions returns the server's own guidance, cached from initialize.
func (p *Pool) Instructions() string {
	p.schemaMu.RLock()
	defer p.schemaMu.RUnlock()
	return p.instructions
}

// SchemaExpired reports whether the cached catalogue has passed the
// freshness deadline a modern upstream set for it (#1602). A cache
// with no deadline (legacy upstream, or none fetched yet) never
// expires. Callers that must not decide from stale data — the write
// policy gate (#1979) classifying a call by its annotations — check
// this and refresh first; RefreshSchemas returns a refresh error
// rather than serving the stale catalogue, so they can fail closed.
func (p *Pool) SchemaExpired() bool {
	p.schemaMu.RLock()
	defer p.schemaMu.RUnlock()
	return !p.schemaAt.IsZero() && !p.schemaDeadline.IsZero() && !time.Now().Before(p.schemaDeadline)
}

// CachedSchemas returns whatever is cached without triggering a fetch.
func (p *Pool) CachedSchemas() ([]mcpclient.Tool, []mcpclient.Resource, time.Time) {
	p.schemaMu.RLock()
	defer p.schemaMu.RUnlock()
	return p.tools, p.resources, p.schemaAt
}

// CachedAll is everything a catalogue read produced, for the disk cache.
// Prompts are part of it: a cache that held only tools and resources left a
// restarted daemon serving no prompts at all, because it counted itself warm
// and never listed them again. The deadline and scope ride along (#1603):
// the disk cache persists them for public catalogues and uses the scope
// to keep private ones off disk entirely.
func (p *Pool) CachedAll() ([]mcpclient.Tool, []mcpclient.Resource, []mcpclient.Prompt, time.Time, time.Time, bool) {
	p.schemaMu.RLock()
	defer p.schemaMu.RUnlock()
	return p.tools, p.resources, p.prompts, p.schemaAt, p.schemaDeadline, p.schemaPrivate
}

// Invalidate forgets the cached schema, so the next read fetches it afresh.
//
// Called when a server announces its list changed. Keeping the old schema
// would have mcpx describe tools that no longer exist, or omit ones that
// now do, until somebody thought to run refresh.
func (p *Pool) Invalidate() {
	p.schemaMu.Lock()
	p.schemaAt = time.Time{}
	p.schemaDeadline, p.schemaPrivate = time.Time{}, false
	p.schemaMu.Unlock()
}

// SchemaPrivate reports whether the last refresh fetched a list that
// declared cacheScope "private". Stored in Phase 1 (#1602); Phase 2
// (#1603) is what enforces scope.
func (p *Pool) SchemaPrivate() bool {
	p.schemaMu.RLock()
	defer p.schemaMu.RUnlock()
	return p.schemaPrivate
}

// CachedTemplates returns the resource templates last seen.
//
// Templates arrive mixed with static resources from ListResources, which
// appends them; they are separated here by the field only a template has.
func (p *Pool) CachedTemplates() []mcpclient.Resource {
	p.schemaMu.RLock()
	defer p.schemaMu.RUnlock()
	var out []mcpclient.Resource
	for _, r := range p.resources {
		if r.URITemplate != "" {
			out = append(out, r)
		}
	}
	return out
}

// CachedPrompts returns the prompts last seen.
func (p *Pool) CachedPrompts() []mcpclient.Prompt {
	p.schemaMu.RLock()
	defer p.schemaMu.RUnlock()
	return p.prompts
}

// GetPrompt renders one prompt on a leased instance.
func (p *Pool) GetPrompt(ctx context.Context, sessionKey, name string, args map[string]string) (json.RawMessage, error) {
	lease, err := p.Acquire(ctx, sessionKey)
	if err != nil {
		return nil, err
	}
	defer lease.Release()
	cctx, finish := p.upstream(ctx, sessionKey, lease)
	res, err := lease.Client().GetPrompt(cctx, name, args)
	return res, finish(err)
}

// Hooks receive what servers volunteer.
//
// Installed on every instance a pool starts. The daemon supplies one set and
// fans the results onto its event bus; a pool built for a test supplies none.
type Hooks struct {
	OnMessage         func(server string, m mcpclient.ServerMessage)
	OnProgress        func(server string, p mcpclient.Progress)
	OnListChanged     func(server, kind string)
	OnResourceUpdated func(server, uri string)
	// OnElicitationComplete fires when a url-mode flow finishes.
	OnElicitationComplete func(server, id string)
	// Elicit answers server-initiated requests: elicitation and sampling.
	// key is the scope key of the instance the question arrived on, which is
	// how a question is attributed to the call that provoked it: one
	// connection serves one key, so whoever is calling on that key is who
	// the server is asking.
	Elicit func(ctx context.Context, server, key, method string, params json.RawMessage) (any, error)
	// URLElicitation opens the URL elicitations an upstream's -32042
	// tools/call error names, parallel to Elicit: server and key say
	// which connection's call was interrupted. True only when every
	// entry was accepted; anything else fails the call with no retry.
	URLElicitation func(ctx context.Context, server, key string, elicitations []mcpclient.URLElicitation) (bool, error)
	Roots          []mcpclient.Root
	// LogLevel is requested from every server that supports logging.
	LogLevel string
	// Protocol is the era preference for a server whose config names none.
	Protocol mcpclient.Preference
	// ProbeTimeout bounds an unanswered stdio server/discover; zero means
	// the default.
	ProbeTimeout time.Duration
	// Eras remembers each server configuration's era across starts. Nil
	// means every start probes.
	Eras EraStore
	// RequestLimits bounds each server's sampling and elicitation
	// requests (#2192). The zero value means mcpclient's defaults,
	// unless RequestLimitsSet marks it explicitly supplied.
	RequestLimits mcpclient.RequestLimits
	// RequestLimitsSet marks RequestLimits as explicitly supplied,
	// including all-zero (enforcement disabled).
	RequestLimitsSet bool
}

func (h *Hooks) notifications(server string) mcpclient.Notifications {
	var n mcpclient.Notifications
	if h.OnMessage != nil {
		n.OnMessage = func(m mcpclient.ServerMessage) { h.OnMessage(server, m) }
	}
	if h.OnProgress != nil {
		n.OnProgress = func(p mcpclient.Progress) { h.OnProgress(server, p) }
	}
	if h.OnListChanged != nil {
		n.OnListChanged = func(kind string) { h.OnListChanged(server, kind) }
	}
	if h.OnResourceUpdated != nil {
		n.OnResourceUpdated = func(uri string) { h.OnResourceUpdated(server, uri) }
	}
	if h.OnElicitationComplete != nil {
		n.OnElicitationComplete = func(id string) { h.OnElicitationComplete(server, id) }
	}
	// Warnings go where every other server event goes, so a tool that
	// vanished from the catalogue for invalid annotations says why.
	n.OnWarning = func(w mcpclient.Warning) {
		lifecycle("server.warning", map[string]any{"server": server, "tool": w.Tool, "reason": w.Reason})
	}
	return n
}

// Preference is which protocol era this server is probed for first: its own
// protocol key, else the daemon-wide upstream.protocol, else the built-in
// default.
func (p *Pool) Preference() mcpclient.Preference {
	if p.cfg.Protocol != "" {
		return mcpclient.Preference(p.cfg.Protocol)
	}
	if h := p.Hooks; h != nil && h.Protocol != "" {
		return h.Protocol
	}
	return mcpclient.Preference(defaults.UpstreamProtocol)
}

// Call runs a tool on a leased instance.
func (p *Pool) Call(ctx context.Context, sessionKey, tool string, args any) (json.RawMessage, error) {
	lease, err := p.Acquire(ctx, sessionKey)
	if err != nil {
		return nil, err
	}
	defer lease.Release()
	lease.inst.calls.Add(1)
	if Trace != nil {
		Trace("call %s.%s session=%q instance=%s pid=%d", p.cfg.Name, tool, sessionKey, lease.inst.ID, lease.inst.PID())
	}

	// The serving client may never have listed the catalogue itself --
	// refreshSchemas lists on the schema lease's instance, which for
	// session- or call-scoped scopes is a throwaway, and a restarted
	// global instance starts empty behind the retained cache. Seed its
	// taskSupport snapshot from the cached catalogue so a "required"
	// tool is recognised on the client that actually serves the call
	// (#1556 Codex review).
	p.schemaMu.RLock()
	catalogue := p.tools
	p.schemaMu.RUnlock()
	lease.Client().SeedToolExec(catalogue)

	cctx, finish := p.upstream(ctx, sessionKey, lease)
	started := time.Now()
	res, err := lease.Client().CallTool(cctx, tool, args)
	err = p.replacedErr(lease.inst, finish(err))
	// Reported from here rather than from the daemon's HTTP handler because
	// this is the only place that knows which instance served the call. The
	// handler sees a namespace; the log wants the process, so that a slow call
	// can be traced back to the server that was started for it.
	attrs := map[string]any{
		"server": p.cfg.Name, "tool": tool, "instance": lease.inst.ID,
		"pid": lease.inst.PID(), "session": sessionKey,
		"durationMs": float64(time.Since(started).Microseconds()) / 1000,
		"ok":         err == nil,
		"trace":      newTraceID("cal"), "trace.parent": lease.inst.trace,
	}
	if err != nil {
		attrs["error"] = err.Error()
	}
	lifecycle("mcp.call", attrs)
	return res, err
}

// ReadResource reads a resource URI on a leased instance.
func (p *Pool) ReadResource(ctx context.Context, sessionKey, uri string) (json.RawMessage, error) {
	lease, err := p.Acquire(ctx, sessionKey)
	if err != nil {
		return nil, err
	}
	defer lease.Release()

	cctx, finish := p.upstream(ctx, sessionKey, lease)
	res, err := lease.Client().ReadResource(cctx, uri)
	return res, p.replacedErr(lease.inst, finish(err))
}

// ReapIdle stops instances that nobody holds and that have gone quiet, plus
// any whose watched pid has exited. Min keeps a floor of warm instances.
func (p *Pool) ReapIdle(now time.Time) int {
	p.mu.Lock()
	var stop []*Instance
	kept := p.instances[:0]
	for _, in := range p.instances {
		// A child being replaced is closed by the replacement, not reaped.
		if in.retiring {
			kept = append(kept, in)
			continue
		}
		// An instance carrying resource subscriptions is not idle: it is
		// the only thing that will ever report those resources changing.
		expired := in.holders == 0 && now.Sub(in.lastUsed) > p.cfg.IdleTimeout &&
			len(p.instances) > p.cfg.Min && !in.hasSubscriptions()
		// A pid-scoped instance belongs to a process. When that process is
		// gone the instance has no possible future caller, so it goes
		// immediately rather than waiting out the idle timer.
		orphaned := in.holders == 0 && p.cfg.Scope.WatchesPID() && !keyPIDAlive(in.key)
		if expired || orphaned {
			stop = append(stop, in)
			continue
		}
		kept = append(kept, in)
	}
	p.instances = kept
	p.cond.Broadcast()
	p.mu.Unlock()

	for _, in := range stop {
		p.stopped(in, "idle")
		_ = in.Client.Close()
	}
	return len(stop)
}

// keyPIDAlive reports whether the process a pid-scoped key names still exists.
// An unparseable key is treated as alive so a bug here cannot kill instances.
func keyPIDAlive(key string) bool {
	pid, ok := config.PIDOf(key)
	if !ok {
		return true
	}
	proc, err := os.FindProcess(pid)
	if err != nil {
		return false
	}
	return proc.Signal(syscall.Signal(0)) == nil
}

// InstanceStatus is a diagnostic snapshot.
type InstanceStatus struct {
	ID        string `json:"id"`
	PID       int    `json:"pid,omitempty"`
	Holders   int    `json:"holders"`
	Key       string `json:"key,omitempty"`
	Calls     int64  `json:"calls"`
	UptimeSec int    `json:"uptimeSec"`
	IdleSec   int    `json:"idleSec"`
}

// Status describes the pool.
type Status struct {
	Name            string           `json:"name"`
	Namespace       string           `json:"namespace"`
	Sharing         string           `json:"sharing"`
	Scope           string           `json:"scope"`
	Max             int              `json:"max"`
	Live            int              `json:"live"`
	Tools           int              `json:"tools"`
	SchemaAge       string           `json:"schemaAge,omitempty"`
	LastError       string           `json:"lastError,omitempty"`
	Instances       []InstanceStatus `json:"instances,omitempty"`
	RequiresSecrets []string         `json:"requiresSecrets,omitempty"`
}

// Status snapshots the pool for `mcpx status`.
func (p *Pool) Status() Status {
	p.mu.Lock()
	st := Status{
		Name:      p.cfg.Name,
		Namespace: p.cfg.Namespace,
		Sharing:   string(p.cfg.Sharing),
		Scope:     string(p.cfg.Scope),
		Max:       p.cfg.Max,
		Live:      len(p.instances),
	}
	if p.cfg != nil && len(p.cfg.RequiresSecrets) > 0 {
		st.RequiresSecrets = p.cfg.RequiresSecrets
	}
	if p.lastErr != nil {
		st.LastError = p.lastErr.Error()
	}
	now := time.Now()
	for _, in := range p.instances {
		st.Instances = append(st.Instances, InstanceStatus{
			ID:        in.ID,
			PID:       in.PID(),
			Holders:   in.holders,
			Key:       in.key,
			Calls:     in.calls.Load(),
			UptimeSec: int(now.Sub(in.startedAt).Seconds()),
			IdleSec:   int(now.Sub(in.lastUsed).Seconds()),
		})
	}
	p.mu.Unlock()

	p.schemaMu.RLock()
	st.Tools = len(p.tools)
	if !p.schemaAt.IsZero() {
		st.SchemaAge = time.Since(p.schemaAt).Truncate(time.Second).String()
	}
	if p.schemaErr != nil && st.LastError == "" {
		st.LastError = p.schemaErr.Error()
	}
	p.schemaMu.RUnlock()
	return st
}

// RestartResult says what a restart did, instance by instance.
type RestartResult struct {
	Stopped int `json:"stopped"`
	// Started names the replacement instances that came up and answered
	// initialize.
	Started []string `json:"started,omitempty"`
	// Failed lists keys whose replacement did not come up, with why --
	// including the server's own stderr when it printed one.
	Failed []RestartFailure `json:"failed,omitempty"`
	// Skipped lists keys deliberately not replaced: a pid-scoped owner that
	// has exited, a per-call instance whose call is over.
	Skipped []RestartFailure `json:"skipped,omitempty"`
	// Note explains a restart that started nothing on purpose.
	Note string `json:"note,omitempty"`
}

// Restart stops every instance and, unless lazy, starts a replacement for
// each under the same scope key and waits for it to initialize. Whatever the
// executable holds, the children are replaced: this is the operator's forced
// restart, and an identical binary gets a new process all the same. With
// nothing running, a global-scope pool starts one instance so a restart
// verifies the server comes up. A scoped pool with nothing running has no
// caller identity to start one for, so it starts nothing and says so in Note.
//
// Unless lazy, each child is taken out of service the way an executable change
// takes one out (see retire): calls in flight on it have ExecDrainTimeout to
// finish, and the replacement is started before this returns.
//
// Lazy is a fast restart, not a cut-off. The children leave the list at once,
// so the next call starts a fresh child without waiting for them, and this
// returns without waiting either. Each child is retired in the background: its
// calls in flight finish on it, and it is closed when the last one ends or
// ExecDrainTimeout passes, whichever is first. A lazy child does not count
// toward Max while it drains, so for that window a lane can hold more than Max
// processes. That is the price of not making the new call wait.
func (p *Pool) Restart(ctx context.Context, lazy bool) RestartResult {
	p.mu.Lock()
	// A copy: retire removes children from p.instances in place.
	stop := append([]*Instance(nil), p.instances...)
	p.failCount = 0
	p.lastErr = nil
	p.cooldownUntil = time.Time{}
	// Taken now, not when each is reached, so no new caller takes a child that
	// is about to go. An instance already retiring for an executable change is
	// closed by that replacement, and is not retired twice.
	var retiring []*Instance
	busy := 0
	for _, in := range stop {
		if in.holders > 0 {
			busy++
		}
		if !in.retiring {
			in.retiring = true
			if !lazy {
				p.reserveSlotLocked(in)
			}
			retiring = append(retiring, in)
		}
	}
	if lazy {
		p.instances = nil
	}
	p.cond.Broadcast()
	p.mu.Unlock()
	if lazy {
		drain := ExecDrainTimeout
		for _, in := range retiring {
			p.draining.Add(1)
			go func() {
				defer p.draining.Done()
				p.retire(in, drain, "restart", "restarted on request")
			}()
		}
		res := RestartResult{Stopped: len(stop)}
		if busy > 0 {
			res.Note = fmt.Sprintf("%d instance(s) still serving calls: they finish on the old process, which is closed after them or after limits.execDrainTimeout", busy)
		}
		return res
	}
	var keys []string
	seen := map[string]bool{}
	for _, in := range stop {
		if !seen[in.key] {
			seen[in.key] = true
			keys = append(keys, in.key)
		}
	}
	for _, in := range retiring {
		p.retire(in, ExecDrainTimeout, "restart", "restarted on request")
		p.mu.Lock()
		p.releaseSlotLocked(in)
		p.cond.Broadcast()
		p.mu.Unlock()
	}
	res := RestartResult{Stopped: len(stop)}
	if len(keys) == 0 {
		if p.cfg.Scope != config.ScopeGlobal {
			res.Note = fmt.Sprintf("no instance running; scope %q starts one per caller on its next call", p.cfg.Scope)
			return res
		}
		keys = []string{"global"}
	}
	for _, key := range keys {
		switch {
		case strings.HasPrefix(key, "call:"):
			res.Skipped = append(res.Skipped, RestartFailure{Key: key, Error: "per-call instance; its call is over"})
			continue
		case p.cfg.Scope.WatchesPID() && !keyPIDAlive(key):
			res.Skipped = append(res.Skipped, RestartFailure{Key: key, Error: "owning process has exited"})
			continue
		}
		id, err := p.startFor(ctx, key)
		if err != nil {
			res.Failed = append(res.Failed, RestartFailure{Key: key, Error: err.Error()})
			continue
		}
		res.Started = append(res.Started, id)
	}
	return res
}

// RestartFailure is one key a restart did not bring back.
type RestartFailure struct {
	Key   string `json:"key"`
	Error string `json:"error"`
}

// startFor brings up the instance for key and lets it go idle. Acquire, not
// start, so capacity, a concurrent caller and a subscription monitor
// replacing the same key all converge on one process.
func (p *Pool) startFor(ctx context.Context, key string) (string, error) {
	lease, err := p.Acquire(ctx, key)
	if err != nil {
		return "", err
	}
	id := lease.inst.ID
	lease.Release()
	return id, nil
}

// WaitDrained blocks until every child a lazy restart took out of service is
// closed: its calls have ended, or the drain window has passed. A lazy restart
// returns before this, so a caller that needs the old children gone (a test
// that restores a global hook, say) waits here.
func (p *Pool) WaitDrained() { p.draining.Wait() }

// Close shuts the pool down permanently, closing instances after in-flight work completes.
func (p *Pool) Close() {
	p.CloseGraceful(defaults.HTTPShutdownGrace)
}

// CloseGraceful marks the pool closed so no new leases are acquired,
// waits for active holders to finish their work up to the specified timeout,
// and then closes the instances. Ongoing requests are preserved without cancellation.
func (p *Pool) CloseGraceful(timeout time.Duration) {
	p.mu.Lock()
	if !p.closed {
		p.closed = true
		close(p.execStop)
	}
	stop := p.instances
	p.instances = nil
	p.cond.Broadcast()
	p.mu.Unlock()

	deadline := time.Now().Add(timeout)
	for _, in := range stop {
		// Wait for ongoing holders to complete
		for {
			p.mu.Lock()
			busy := in.holders > 0
			p.mu.Unlock()
			if !busy || time.Now().After(deadline) {
				break
			}
			time.Sleep(10 * time.Millisecond)
		}
		_ = in.Client.Close()
	}
}

// CloseDrained shuts a pool down the way a lazy restart retires its children,
// for a pool that a configuration change has replaced or removed. Each child
// is taken out of service at once, and is closed when the calls on it end or
// ExecDrainTimeout passes, whichever is first.
//
// CloseGraceful, which Close uses, waits only its own shorter grace and then
// closes the connection, failing whatever call is still reading from it. A
// tuning change or a removed server cut a call that ran past that grace.
// WaitDrained covers this as well as a lazy restart.
func (p *Pool) CloseDrained() {
	p.closeDrained(ExecDrainTimeout, "reconfigured", "replaced by a configuration change")
}

// CloseDrainedWithin is CloseDrained for a daemon that is shutting down. drain
// is the time calls in flight have, which the daemon passes as what is left of
// its drain window once the HTTP server has stopped taking calls. The window
// is ExecDrainTimeout, the same as a reload's, so a stop is bounded by it.
func (p *Pool) CloseDrainedWithin(drain time.Duration) {
	p.closeDrained(drain, "shutdown", "stopped because the daemon is shutting down")
}

func (p *Pool) closeDrained(drain time.Duration, stopReason, callErr string) {
	p.mu.Lock()
	if !p.closed {
		p.closed = true
		close(p.execStop)
	}
	stop := p.instances
	p.instances = nil
	for _, in := range stop {
		in.retiring = true
	}
	p.cond.Broadcast()
	p.mu.Unlock()
	for _, in := range stop {
		p.draining.Add(1)
		go func(in *Instance) {
			defer p.draining.Done()
			p.retire(in, drain, stopReason, callErr)
		}(in)
	}
}

// newTraceID mints an identifier. Kept here rather than imported so the pool
// does not depend on the logging package.
func newTraceID(prefix string) string {
	var b [8]byte
	if _, err := rand.Read(b[:]); err != nil {
		return prefix + "-0000000000000000"
	}
	return prefix + "-" + hex.EncodeToString(b[:])
}

func transportName(c *config.Resolved) string {
	if c.Stdio() {
		return "stdio"
	}
	if c.Transport != "" {
		return c.Transport
	}
	return "http"
}

// stopped reports a server going away, with why.
func (p *Pool) stopped(in *Instance, reason string) {
	lifecycle("server.stop", map[string]any{
		"server": p.cfg.Name, "instance": in.ID, "pid": in.PID(),
		"trace": in.trace, "reason": reason,
		"calls": in.calls.Load(), "uptimeSec": int(time.Since(in.startedAt).Seconds()),
	})
}
