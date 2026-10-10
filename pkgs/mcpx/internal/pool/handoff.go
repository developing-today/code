package pool

import (
	"errors"
	"fmt"
	"time"

	"github.com/dezren39/mcpx/internal/mcpclient"
)

// InstanceHandoff is a live stdio instance in transit to a successor. Its
// pipes travel beside it as file descriptors, not in the JSON.
type InstanceHandoff struct {
	ID         string                  `json:"id"`
	Trace      string                  `json:"trace"`
	Key        string                  `json:"key"`
	StartedAt  time.Time               `json:"startedAt"`
	LastUsed   time.Time               `json:"lastUsed"`
	Calls      int64                   `json:"calls"`
	EraSource  string                  `json:"eraSource,omitempty"`
	LegacyLane bool                    `json:"legacyLane,omitempty"`
	Subscribed []string                `json:"subscribed,omitempty"`
	Stdio      *mcpclient.StdioHandoff `json:"stdio"`
	Session    mcpclient.Session       `json:"session"`
}

// PoolHandoff is one pool's children in transit. PoolID names the process
// definition, so a successor matches it to its own pool of the same identity.
type PoolHandoff struct {
	PoolID    string            `json:"poolId"`
	Seq       int               `json:"seq"`
	Instances []InstanceHandoff `json:"instances"`
}

// Detach takes the pool out of service so a successor can serve its stdio
// children. Only those are handed on; every other instance stays parked until
// Commit closes it or Restore puts it back.
func (p *Pool) Detach() (*PoolHandoff, error) {
	p.mu.Lock()
	if p.closed {
		p.mu.Unlock()
		return nil, errors.New("pool is closed")
	}
	p.closed = true
	parked := p.instances
	p.parked = parked
	p.instances = nil
	seq := p.seq
	metas := make([]InstanceHandoff, len(parked))
	for i, in := range parked {
		metas[i] = in.describe()
	}
	p.cond.Broadcast()
	p.mu.Unlock()

	var handed []*Instance
	var out []InstanceHandoff
	for i, in := range parked {
		if _, ok := in.transport.(*mcpclient.StdioTransport); !ok || !in.Client.Alive() {
			continue
		}
		h, err := in.Client.Detach()
		if err != nil {
			p.restore(handed)
			return nil, fmt.Errorf("server %q: %w", p.cfg.Name, err)
		}
		meta := metas[i]
		meta.Stdio = h.Stdio
		meta.Session = h.Session
		out = append(out, meta)
		handed = append(handed, in)
	}
	p.mu.Lock()
	p.handed = handed
	p.mu.Unlock()
	return &PoolHandoff{PoolID: p.cfg.PoolID(), Seq: seq, Instances: out}, nil
}

// Commit finishes a detach the successor accepted: handed children now belong
// to the successor, and everything else parked is closed.
func (p *Pool) Commit() {
	p.mu.Lock()
	parked, handed := p.parked, p.handed
	p.parked, p.handed = nil, nil
	p.mu.Unlock()
	released := make(map[*Instance]bool, len(handed))
	for _, in := range handed {
		released[in] = true
		in.Client.Release()
	}
	for _, in := range parked {
		if !released[in] {
			_ = in.Client.Close()
		}
	}
}

// Restore undoes a Detach the successor did not accept. The children never
// stopped and their pipes were only paused, so the pool carries on as before.
func (p *Pool) Restore() {
	p.mu.Lock()
	handed := p.handed
	p.handed = nil
	p.mu.Unlock()
	p.restore(handed)
}

func (p *Pool) restore(handed []*Instance) {
	for _, in := range handed {
		in.Client.Reattach()
	}
	p.mu.Lock()
	p.instances = p.parked
	p.parked = nil
	p.closed = false
	p.cond.Broadcast()
	p.mu.Unlock()
}

// describe is the instance's record without its pipes. p.mu is held.
func (in *Instance) describe() InstanceHandoff {
	in.subMu.Lock()
	subs := make([]string, 0, len(in.subscribed))
	for uri := range in.subscribed {
		subs = append(subs, uri)
	}
	in.subMu.Unlock()
	return InstanceHandoff{
		ID:         in.ID,
		Trace:      in.trace,
		Key:        in.key,
		StartedAt:  in.startedAt,
		LastUsed:   in.lastUsed,
		Calls:      in.calls.Load(),
		EraSource:  in.eraSource,
		LegacyLane: in.legacyLane,
		Subscribed: subs,
	}
}

// Adopted is a child taken from a predecessor. Nothing reads its pipes until
// Attach, so Abort can give them back untouched.
type Adopted struct {
	handoff InstanceHandoff
	tr      *mcpclient.StdioTransport
}

// Adopt takes the pipes of each handed child that is still running. A child
// that has exited is dropped; any other failure releases what was taken.
func Adopt(hs []InstanceHandoff) ([]*Adopted, error) {
	var out []*Adopted
	for _, h := range hs {
		if h.Stdio == nil {
			abortAll(out)
			return nil, fmt.Errorf("instance %s: no stdio child to adopt", h.ID)
		}
		tr, err := mcpclient.AdoptStdio(*h.Stdio)
		if errors.Is(err, mcpclient.ErrChildGone) {
			continue
		}
		if err != nil {
			abortAll(out)
			return nil, fmt.Errorf("instance %s: %w", h.ID, err)
		}
		out = append(out, &Adopted{handoff: h, tr: tr})
	}
	return out, nil
}

// Abort releases an adopted child that will not be served.
func (a *Adopted) Abort() {
	_, _ = a.tr.Detach()
}

func abortAll(ads []*Adopted) {
	for _, a := range ads {
		a.Abort()
	}
}

// Close ends an adopted child that no pool wants.
func (a *Adopted) Close() {
	_ = a.tr.Close()
}

// Attach serves adopted children from this pool, resuming each session and
// starting its read loop. Call it only after the handoff has committed.
func (p *Pool) Attach(ads []*Adopted, seq int) {
	p.mu.Lock()
	defer p.mu.Unlock()
	for _, a := range ads {
		h := a.handoff
		opts := mcpclient.Options{ClientName: "mcpx", ClientVersion: Version}
		ref := &instanceRef{p: p}
		p.installServerHooks(&opts, ref)
		cl := mcpclient.Resume(a.tr, h.Session, opts)
		p.subscribeServerEvents(cl)
		in := &Instance{
			ID:         h.ID,
			Client:     cl,
			transport:  a.tr,
			key:        h.Key,
			trace:      h.Trace,
			startedAt:  h.StartedAt,
			lastUsed:   h.LastUsed,
			eraSource:  h.EraSource,
			legacyLane: h.LegacyLane,
			subscribed: make(map[string]bool, len(h.Subscribed)),
		}
		in.calls.Store(h.Calls)
		for _, uri := range h.Subscribed {
			in.subscribed[uri] = true
		}
		ref.set(in)
		p.instances = append(p.instances, in)
		cl.Activate()
	}
	if seq > p.seq {
		p.seq = seq
	}
	p.cond.Broadcast()
}

// PoolID is the identity this pool's process definition shares with others.
func (p *Pool) PoolID() string { return p.cfg.PoolID() }
