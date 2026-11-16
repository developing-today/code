package model

import (
	"context"
	"fmt"
)

// PluginProvider is the opencode-plugin backend: a Provider whose
// generations are answered by the plugin answerer through the same
// elicitation broker the sampling backend uses (#1925). Triage on
// #1925 (task 13246) verified the broker already carries
// daemon-initiated model requests to an answerer -- brokerSampler
// opens an elicit.SampleAsk question, publishes sample.opened before
// awaiting, and maps a non-accept answer to ErrUnavailable -- so the
// plugin backend is a different sampler binding (its questions are
// labelled model.provider=plugin and its answers are stamped with
// this provider's name), not a new transport. The daemon installs
// the binding as Config.PluginSampler; the v2 plugin work it was
// once said to depend on ("#47") was a stale reference -- #47 in
// this repo is an unrelated Dependabot PR, and the plugin-port docs
// it meant landed via #1855.
type PluginProvider struct {
	fn SampleFunc
}

// NewPluginProvider wraps fn, the daemon's plugin sampler binding. A
// nil fn is a provider that is never available, which is the honest
// state of a daemon with no plugin answerer path installed.
func NewPluginProvider(fn SampleFunc) *PluginProvider {
	return &PluginProvider{fn: fn}
}

func (p *PluginProvider) Name() string { return "plugin" }

func (p *PluginProvider) Generate(ctx context.Context, req Request) (Response, error) {
	if p.fn == nil {
		return Response{}, fmt.Errorf("model: plugin: %w (no plugin answerer path installed)", ErrUnavailable)
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
