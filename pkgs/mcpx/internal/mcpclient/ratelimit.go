package mcpclient

import (
	"fmt"
	"sync"
	"time"
)

// Rate limits on server-initiated requests (#2192, split of #1957;
// design #2333).
//
// The sampling and elicitation security considerations say a client
// SHOULD bound how often a server may ask: an upstream server that can
// make unlimited sampling requests has a cost-amplification and
// denial-of-service surface, and elicitation is an attention channel a
// buggy or malicious server could flood. Before this, mcpx answered as
// often as a server asked.
//
// Enforcement sits at the entry of Client.answer, the one function
// both eras converge on (a legacy server sends the request on the
// wire, a modern one returns it inside an input_required result), so
// an over-limit question reaches no handler, broker, or person on
// either path.
//
// Scopes, per design #2333:
//   - per-session: one Client is one session with one server, so the
//     session counter lives on the Client and dies with it. Sampling
//     only.
//   - per-server: the pool owns one ServerRequestLimiter per
//     configured server and hands it to every client it builds for
//     that server through Options. The key is therefore the pool's
//     configured name -- never the server's self-reported
//     ServerInfo.Name, which is spoofable. Clients built outside the
//     pool construct their own limiter, which degenerates to
//     per-session strength; recorded in the design as acceptable.
//
// Counting: a question counts when it is admitted, once, against both
// scopes it belongs to -- whether it is later answered, declined, or
// expires, the bounded resource (model spend, human attention, broker
// slots) is spent on the asking. A refused question counts against
// neither scope. Each input_required entry counts separately, because
// each is answered separately.
//
// Refusal is a -32603 error naming the setting that set the limit,
// never a fabricated answer: for elicitation an {action: "cancel"}
// result would claim a person decided, and nobody did. The refusal is
// also emitted on the client's warning channel. The spec assigns no
// rate-limit code; inventing one in the -320xx range is a separate
// decision (design #2333, open questions).

// rateWindow is the fixed window the per-minute limits count over.
const rateWindow = time.Minute

// RequestLimits bounds server-initiated requests, as counts per
// minute. A zero or negative limit disables that scope.
type RequestLimits struct {
	SamplingPerServer    int
	SamplingPerSession   int
	ElicitationPerServer int
}

// DefaultRequestLimits are the built-in bounds, matching the
// proto.* setting defaults. Used when Options names none.
func DefaultRequestLimits() RequestLimits {
	return RequestLimits{
		SamplingPerServer:    60,
		SamplingPerSession:   20,
		ElicitationPerServer: 30,
	}
}

// requestLimits resolves an Options value. An explicitly supplied
// struct (RequestLimitsSet) is taken as given -- so a 0 from the
// settings really does disable that scope. Otherwise the zero
// RequestLimits means "no opinion" and yields the defaults, while a
// non-zero struct is taken as the caller's intent.
func (o Options) requestLimits() RequestLimits {
	if o.RequestLimitsSet {
		return o.RequestLimits
	}
	if o.RequestLimits == (RequestLimits{}) {
		return DefaultRequestLimits()
	}
	return o.RequestLimits
}

// windowCounter is a fixed-window counter: at most limit admissions
// per window. Safe for concurrent use; admission is split into
// wouldAllow + record so a caller bounding several scopes can refuse
// without consuming any of them (see checkServerRequest).
type windowCounter struct {
	mu     sync.Mutex
	limit  int
	window time.Duration
	start  time.Time
	count  int
}

func newWindowCounter(limit int, window time.Duration) *windowCounter {
	return &windowCounter{limit: limit, window: window}
}

func (w *windowCounter) wouldAllow(now time.Time) bool {
	if w == nil || w.limit <= 0 {
		return true
	}
	w.mu.Lock()
	defer w.mu.Unlock()
	if w.start.IsZero() || now.Sub(w.start) >= w.window {
		return true
	}
	return w.count < w.limit
}

func (w *windowCounter) record(now time.Time) {
	if w == nil || w.limit <= 0 {
		return
	}
	w.mu.Lock()
	defer w.mu.Unlock()
	if w.start.IsZero() || now.Sub(w.start) >= w.window {
		w.start = now
		w.count = 0
	}
	w.count++
}

// allow records one admission attempt at now and reports whether it
// was within the limit.
func (w *windowCounter) allow(now time.Time) bool { return w.tryAllow(now) }

// tryAllow is allow as ONE transaction under the counter's lock: peek
// and record cannot be interleaved by another goroutine. Callers that
// bound a SHARED scope (the per-server counters, used by every session
// to a server at once) must admit through tryAllow -- wouldAllow +
// record as separate calls lets two sessions both pass the peek before
// either records, and the shared budget is exceeded.
func (w *windowCounter) tryAllow(now time.Time) bool {
	if w == nil || w.limit <= 0 {
		return true
	}
	w.mu.Lock()
	defer w.mu.Unlock()
	if w.start.IsZero() || now.Sub(w.start) >= w.window {
		w.start = now
		w.count = 0
	}
	if w.count >= w.limit {
		return false
	}
	w.count++
	return true
}

// ServerRequestLimiter holds one configured server's shared budgets,
// across every session mcpx holds with it. The pool owns one per
// server; see the package comment above.
type ServerRequestLimiter struct {
	sampling    *windowCounter
	elicitation *windowCounter
}

// NewServerRequestLimiter builds a server's shared budgets from lim.
func NewServerRequestLimiter(lim RequestLimits) *ServerRequestLimiter {
	return &ServerRequestLimiter{
		sampling:    newWindowCounter(lim.SamplingPerServer, rateWindow),
		elicitation: newWindowCounter(lim.ElicitationPerServer, rateWindow),
	}
}

// setupRequestLimits wires a client's rate-limit state from its
// options: the per-session counter lives on the client, the per-server
// counters are the limiter the options carry, or a fresh one when the
// client was built outside the pool.
func (c *Client) setupRequestLimits(o Options) {
	c.reqLimits = o.requestLimits()
	c.serverName = o.ServerName
	c.sessionSampling = newWindowCounter(c.reqLimits.SamplingPerSession, rateWindow)
	c.server = o.ServerLimiter
	if c.server == nil {
		c.server = NewServerRequestLimiter(c.reqLimits)
	}
}

// checkServerRequest admits one server-initiated question of the given
// method ("sampling/createMessage" or "elicitation/create"), or
// returns the refusal it fails with. Both scopes are peeked before
// either is recorded, so a refused question consumes no budget.
func (c *Client) checkServerRequest(method string) *rpcError {
	now := time.Now()
	c.reqMu.Lock()
	defer c.reqMu.Unlock()
	switch method {
	case "sampling/createMessage":
		// The session counter is this client's alone (touched only
		// here, under reqMu), so peeking it is safe. The server
		// counter is shared with every other session to this
		// server, so it is admitted atomically: tryAllow records
		// only when it admits, and a refusal here has consumed
		// neither scope (the session counter was only peeked).
		if !c.sessionSampling.wouldAllow(now) {
			return c.rateRefusalLocked(fmt.Sprintf(
				"rate limit: sampling from %s exceeds proto.samplingSessionPerMinute (%d/minute) for this session",
				c.serverDisplayName(), c.reqLimits.SamplingPerSession))
		}
		if !c.server.sampling.tryAllow(now) {
			return c.rateRefusalLocked(fmt.Sprintf(
				"rate limit: sampling from %s exceeds proto.samplingPerMinute (%d/minute)",
				c.serverDisplayName(), c.reqLimits.SamplingPerServer))
		}
		c.sessionSampling.record(now)
	case "elicitation/create":
		if !c.server.elicitation.tryAllow(now) {
			return c.rateRefusalLocked(fmt.Sprintf(
				"rate limit: elicitation from %s exceeds proto.elicitationPerMinute (%d/minute)",
				c.serverDisplayName(), c.reqLimits.ElicitationPerServer))
		}
	}
	return nil
}

// rateRefusalLocked builds the refusal error and emits it on the
// warning channel, so an operator can see which bound bit and for
// which server. Called with reqMu held; warn takes c.mu, never reqMu,
// so the two cannot deadlock.
func (c *Client) rateRefusalLocked(msg string) *rpcError {
	c.warn(Warning{Reason: msg})
	return &rpcError{Code: codeInternalError, Message: msg}
}

func (c *Client) serverDisplayName() string {
	if c.serverName != "" {
		return c.serverName
	}
	return "this server"
}
