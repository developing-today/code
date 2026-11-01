package pool

import (
	"context"
	"errors"
	"fmt"
	"sync"
	"time"
)

// Timeouts that bound one upstream request, and which one wins.
//
// Five settings can end a call, and they used to contradict each other:
// pool.callTimeout wrapped the whole request, including the minutes a person
// spends answering a question the server asked, so a call whose requestState
// was still valid for half an hour died at two minutes. The precedence is now:
//
//  1. The caller's own context (an HTTP client hanging up, a task's TTL, a
//     cancelled MCP request). Always wins; nothing here extends it.
//  2. pool.callTimeout: the budget for the upstream *working*. It counts only
//     while no question is pending on the instance serving the call, which is
//     what stops a runaway tool without also stopping a person who is typing.
//     A caller may request a shorter value for one call (--pool-call-timeout,
//     #1864 item 4): the configured value is the ceiling, so the effective
//     budget is the smaller of the two -- a call may lower its bound, never
//     raise it. exec.timeout is not in this list: it bounds a whole script
//     run, not one upstream call. The run's total work is capped by
//     exec.timeout and each call inside it by this budget, so for any one
//     call whichever bound is tighter is the one that fires.
//  3. elicit.ttl: how long one broker question lives. It bounds each pause
//     in (2), so a call can never be held open forever by an unanswered
//     question: the question expires, the server gets "cancel", and the
//     budget resumes.
//  4. proto.askTimeout: how long mcpx serve waits for an MCP client to
//     answer a question it was handed inline, before leaving it to the
//     broker. It is inside (3), not beside it.
//  5. consumer.askTimeout / elicit.handlerTimeout: questions mcpx itself
//     asks (confirmations, disambiguation) and the external handler program.
//     They happen before or beside the upstream request, not inside it, so
//     (2) never sees them.
//
// proto.askTTL and proto.stateTTL bound the /v1/ask task and the requestState
// token; they are the caller's context in (1) for an interruptible call.

// errCallTimeout is the cause recorded when pool.callTimeout runs out.
var errCallTimeout = errors.New("pool.callTimeout exceeded")

type callTimeoutKey struct{}

// WithCallTimeout carries a caller's requested per-call pool.callTimeout
// (#1864 item 4) to upstream. The value is a request, not an order:
// upstream clamps it to the pool's configured CallTimeout (the ceiling
// rule #1864 adopts -- a caller may lower the bound for one call, never
// raise it above what the daemon's operator configured). It rides the
// context, so it scopes to exactly one call and cannot leak into the
// next call served by the same pooled instance.
func WithCallTimeout(ctx context.Context, d time.Duration) context.Context {
	if d <= 0 {
		return ctx
	}
	return context.WithValue(ctx, callTimeoutKey{}, d)
}

// RequestedCallTimeout returns the per-call value WithCallTimeout put on
// ctx, when one is there.
func RequestedCallTimeout(ctx context.Context) (time.Duration, bool) {
	d, ok := ctx.Value(callTimeoutKey{}).(time.Duration)
	return d, ok && d > 0
}

// callTimeout is the effective budget for one upstream request: the
// pool's configured CallTimeout, lowered by a caller's per-call request
// when one is present and smaller. A configured value of zero or less
// means the operator set no bound, so a per-call request then applies
// exactly as asked.
func (p *Pool) callTimeout(ctx context.Context) time.Duration {
	cfg := p.cfg.CallTimeout
	req, ok := RequestedCallTimeout(ctx)
	if !ok {
		return cfg
	}
	if cfg <= 0 || req < cfg {
		return req
	}
	return cfg
}

// questions tracks the server-initiated requests pending on an instance.
type questions struct {
	mu      sync.Mutex
	n       int
	changed chan struct{}
}

// state returns whether a question is pending, and a channel closed on the
// next change. Read together so a change between the two cannot be missed.
func (q *questions) state() (bool, <-chan struct{}) {
	q.mu.Lock()
	defer q.mu.Unlock()
	if q.changed == nil {
		q.changed = make(chan struct{})
	}
	return q.n > 0, q.changed
}

func (q *questions) add(d int) {
	q.mu.Lock()
	q.n += d
	if q.changed != nil {
		close(q.changed)
	}
	q.changed = make(chan struct{})
	q.mu.Unlock()
}

// asking marks a question pending until the returned func is called.
func (q *questions) asking() func() {
	q.add(1)
	var once sync.Once
	return func() { once.Do(func() { q.add(-1) }) }
}

// budget is context.WithTimeout whose clock stops while q has a question
// pending. The timeout surfaces as a cause, so ctx.Err() is Canceled and
// callers go through finish to see DeadlineExceeded.
func budget(ctx context.Context, d time.Duration, q *questions) (context.Context, context.CancelFunc) {
	cctx, cancel := context.WithCancelCause(ctx)
	if d <= 0 {
		return cctx, func() { cancel(nil) }
	}
	go func() {
		remaining := d
		for {
			paused, changed := q.state()
			if paused {
				select {
				case <-changed:
					continue
				case <-cctx.Done():
					return
				}
			}
			started := time.Now()
			t := time.NewTimer(remaining)
			select {
			case <-cctx.Done():
				t.Stop()
				return
			case <-t.C:
				cancel(errCallTimeout)
				return
			case <-changed:
				t.Stop()
				remaining -= time.Since(started)
			}
		}
	}()
	return cctx, func() { cancel(nil) }
}

// upstream starts one request on a lease: it counts the request as in flight
// on its key and gives it the call budget. finish must be called with the
// request's error; it reports a spent budget as DeadlineExceeded, which is
// what callers of the old WithTimeout matched on.
func (p *Pool) upstream(ctx context.Context, key string, lease *Lease) (context.Context, func(error) error) {
	p.track(key, 1)
	eff := p.callTimeout(ctx)
	cctx, cancel := budget(ctx, eff, &lease.inst.questions)
	return cctx, func(err error) error {
		timedOut := context.Cause(cctx) == errCallTimeout
		cancel()
		p.track(key, -1)
		if err != nil && timedOut {
			return fmt.Errorf("%w: %v (%s)", context.DeadlineExceeded, errCallTimeout, eff)
		}
		return err
	}
}

func (p *Pool) track(key string, d int) {
	p.flightMu.Lock()
	defer p.flightMu.Unlock()
	if p.inflight == nil {
		p.inflight = map[string]int{}
	}
	p.inflight[key] += d
	if p.inflight[key] <= 0 {
		delete(p.inflight, key)
	}
}

// InFlight is how many upstream requests are running on key, from every
// caller: /v1/call, exec, the CLI, ask calls. A question arriving on key can
// be attributed to one call only when this is 1.
func (p *Pool) InFlight(key string) int {
	p.flightMu.Lock()
	defer p.flightMu.Unlock()
	return p.inflight[key]
}
