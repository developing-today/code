package model

import (
	"context"
	"fmt"
)

// SampleFunc raises one generation request on the caller's own model, via
// MCP sampling through the elicitation broker -- the path mcpx had before
// this layer existed. The daemon supplies it; the package cannot import
// the broker without inverting the dependency direction.
type SampleFunc func(ctx context.Context, req Request) (Response, error)

// SamplingProvider exposes sampling as a Provider backend, so the
// pre-existing path survives as one backend among several (#1925
// approach C) instead of being the only path -- the position it was in
// when the 2026-07-28 MCP revision deprecated sampling (SEP-2577) and it
// turned out no opencode declares the capability.
type SamplingProvider struct {
	fn SampleFunc
}

// NewSamplingProvider wraps fn. A nil fn is a provider that is never
// available, which is the honest state of a daemon with no broker.
func NewSamplingProvider(fn SampleFunc) *SamplingProvider {
	return &SamplingProvider{fn: fn}
}

func (p *SamplingProvider) Name() string { return "sampling" }

func (p *SamplingProvider) Generate(ctx context.Context, req Request) (Response, error) {
	if p.fn == nil {
		return Response{}, fmt.Errorf("model: sampling: %w (no broker sampling path installed)", ErrUnavailable)
	}
	resp, err := p.fn(ctx, req)
	if err != nil {
		return Response{}, err
	}
	if resp.Provider == "" {
		resp.Provider = p.Name()
	}
	return resp, nil
}
