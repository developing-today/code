package model

import (
	"context"
	"encoding/json"
	"fmt"
	"os"
	"strings"
	"time"
)

// Config is everything resolution needs, resolved by the caller from the
// model.* settings. It is a plain struct so the package stays free of the
// settings registry: the daemon reads the registry and passes values.
type Config struct {
	// Provider: auto | sampling | plugin | openai | anthropic | google |
	// none. Empty means none.
	Provider string
	// BaseURL overrides the named provider's default endpoint. With a
	// provider of openai (or auto) it is what makes any OpenAI-compatible
	// endpoint -- LiteLLM, Ollama, LM Studio, a company gateway -- work.
	BaseURL string
	// APIKeyEnv names the environment variable holding the API key. It
	// is opt-in (#1925 open question): mcpx never scans the environment
	// for known key names on its own; the user names the variable.
	APIKeyEnv string
	// DefaultModel is model.default, in the provider/model namespace.
	// Under auto, its prefix helps choose the backend.
	DefaultModel string
	// Timeout bounds one generation call.
	Timeout time.Duration
	// Sampler is the broker sampling path, when the daemon has one.
	Sampler SampleFunc
	// LookupEnv reads the environment; nil means os.Getenv. A seam for
	// tests, which must not depend on the machine's real variables.
	LookupEnv func(string) string
	// These are the JSON object values of model.providers and model.profiles.
	// The model layer owns decoding and, importantly, only reads explicitly
	// named environment variables from provider settings.
	ProviderSettingsJSON string
	ProfilesJSON         string
}

type providerSettings struct {
	BaseURL   string `json:"baseURL"`
	APIKeyEnv string `json:"apiKeyEnv"`
}

// Profile is an ordered model chain with optional per-profile bounds.
// Timeout is a Go duration string, for example "30s".
type Profile struct {
	Models  []string `json:"models"`
	Budget  int      `json:"budget,omitempty"`
	Timeout string   `json:"timeout,omitempty"`
}

// Default base URLs for the named configured providers. anthropic and
// google are served through their OpenAI-compatible endpoints by the one
// backend of this slice; native clients are a later slice if wanted.
var defaultBaseURLs = map[string]string{
	"openai":    "https://api.openai.com/v1",
	"anthropic": "https://api.anthropic.com/v1",
	"google":    "https://generativelanguage.googleapis.com/v1beta/openai",
}

// Providers is the model.provider enum, in the order the settings
// registry presents it. Kept here so the registry and the resolver cannot
// drift apart silently; the registry test pins the registry's copy.
var Providers = []string{"auto", "sampling", "plugin", "openai", "anthropic", "google", "none"}

// Resolve builds the configured Provider.
//
// A nil Provider with a nil error is a real outcome: model.provider is
// none (the daemon default), so nothing calls a model and the caller
// stays on its deterministic path -- no behaviour change for a user who
// configures nothing. An error means a provider was named but cannot be
// built (the plugin backend is not built yet; sampling was asked for
// with no sampling path installed); callers treat it as unavailable and
// fall back the same way.
func Resolve(cfg Config) (Provider, error) {
	lookup := cfg.LookupEnv
	if lookup == nil {
		lookup = os.Getenv
	}
	var keyed map[string]providerSettings
	if cfg.ProviderSettingsJSON != "" {
		if err := json.Unmarshal([]byte(cfg.ProviderSettingsJSON), &keyed); err != nil {
			return nil, fmt.Errorf("model: invalid model.providers: %w", err)
		}
	}
	var profiles map[string]Profile
	if cfg.ProfilesJSON != "" {
		if err := json.Unmarshal([]byte(cfg.ProfilesJSON), &profiles); err != nil {
			return nil, fmt.Errorf("model: invalid model.profiles: %w", err)
		}
	}
	if len(profiles) > 0 || len(keyed) > 0 {
		return &profileRouter{cfg: cfg, keyed: keyed, profiles: profiles, lookup: lookup}, nil
	}
	switch strings.ToLower(strings.TrimSpace(cfg.Provider)) {
	case "", "none":
		return nil, nil
	case "sampling":
		if cfg.Sampler == nil {
			return nil, fmt.Errorf("model: provider sampling: %w (no sampling path installed)", ErrUnavailable)
		}
		return NewSamplingProvider(cfg.Sampler), nil
	case "plugin":
		// The opencode-plugin backend answers through the v2 plugin's
		// ctx.generate.text and depends on #47; it is a later slice of
		// #1925, named in the enum now so configurations written for it
		// fail with this sentence rather than a bare "unknown provider".
		return nil, fmt.Errorf("model: provider plugin: %w (the opencode-plugin backend is not built yet; it depends on #47)", ErrUnavailable)
	case "openai", "anthropic", "google":
		name := strings.ToLower(strings.TrimSpace(cfg.Provider))
		return configured(cfg, name, lookup), nil
	case "auto":
		return resolveAuto(cfg, lookup)
	default:
		return nil, fmt.Errorf("model: unknown provider %q (want one of %s)", cfg.Provider, strings.Join(Providers, ", "))
	}
}

// profileRouter routes a call after seeing the selected model reference. A
// profile may be selected by model.default or by an operation override.
type profileRouter struct {
	cfg      Config
	keyed    map[string]providerSettings
	profiles map[string]Profile
	lookup   func(string) string
}

func (r *profileRouter) Name() string { return "profiles" }

func (r *profileRouter) Generate(ctx context.Context, req Request) (Response, error) {
	selection := strings.TrimSpace(req.Model)
	if selection == "" {
		selection = strings.TrimSpace(r.cfg.DefaultModel)
	}
	if profile, ok := r.profile(selection); ok {
		return r.generateProfile(ctx, selection, profile, req)
	}
	provider, modelID := SplitModelID(selection)
	if provider == "" {
		provider = strings.ToLower(strings.TrimSpace(r.cfg.Provider))
	}
	p, err := r.provider(provider)
	if err != nil {
		return Response{}, err
	}
	req.Model = modelID
	resp, err := p.Generate(ctx, req)
	if err == nil && strings.TrimSpace(resp.Text) != "" {
		if resp.Model == "" {
			resp.Model = modelID
		}
		if resp.Provider == "" {
			resp.Provider = p.Name()
		}
	}
	return resp, err
}

func (r *profileRouter) profile(name string) (Profile, bool) {
	if p, ok := r.profiles[name]; ok {
		return p, true
	}
	for key, p := range r.profiles {
		if strings.EqualFold(key, name) {
			return p, true
		}
	}
	return Profile{}, false
}

func (r *profileRouter) generateProfile(ctx context.Context, name string, profile Profile, req Request) (Response, error) {
	if len(profile.Models) == 0 {
		return Response{}, fmt.Errorf("model: profile %q has no models: %w", name, ErrUnavailable)
	}
	if profile.Budget < 0 {
		return Response{}, fmt.Errorf("model: profile %q has negative token budget %d", name, profile.Budget)
	}
	var timeout time.Duration
	if profile.Timeout != "" {
		var err error
		timeout, err = time.ParseDuration(profile.Timeout)
		if err != nil || timeout <= 0 {
			return Response{}, fmt.Errorf("model: profile %q has invalid timeout %q", name, profile.Timeout)
		}
	}
	var failures []string
	for _, entry := range profile.Models {
		provider, modelID := SplitModelID(strings.TrimSpace(entry))
		if provider == "" || modelID == "" {
			failures = append(failures, fmt.Sprintf("invalid model id %q", entry))
			continue
		}
		p, err := r.provider(provider)
		if err != nil {
			failures = append(failures, err.Error())
			continue
		}
		callCtx := ctx
		cancel := func() {}
		if timeout > 0 {
			callCtx, cancel = context.WithTimeout(ctx, timeout)
		}
		call := req
		call.Model = modelID
		if profile.Budget > 0 && (call.MaxTokens <= 0 || call.MaxTokens > profile.Budget) {
			call.MaxTokens = profile.Budget
		}
		resp, callErr := p.Generate(callCtx, call)
		cancel()
		if callErr == nil && strings.TrimSpace(resp.Text) != "" {
			if resp.Model == "" {
				resp.Model = modelID
			}
			if resp.Provider == "" {
				resp.Provider = p.Name()
			}
			return resp, nil
		}
		if callErr == nil {
			callErr = fmt.Errorf("provider returned empty output")
		}
		failures = append(failures, fmt.Sprintf("%s: %v", entry, callErr))
	}
	return Response{}, fmt.Errorf("model: profile %q exhausted (%s): %w", name, strings.Join(failures, "; "), ErrUnavailable)
}

func (r *profileRouter) provider(name string) (Provider, error) {
	name = strings.ToLower(strings.TrimSpace(name))
	if name == "sampling" {
		if r.cfg.Sampler == nil {
			return nil, fmt.Errorf("model: provider sampling: %w", ErrUnavailable)
		}
		return NewSamplingProvider(r.cfg.Sampler), nil
	}
	if name == "plugin" {
		return nil, fmt.Errorf("model: provider plugin: %w", ErrUnavailable)
	}
	var settings providerSettings
	configured := false
	for key, value := range r.keyed {
		if strings.EqualFold(key, name) {
			settings, configured = value, true
			break
		}
	}
	if !configured && name == strings.ToLower(strings.TrimSpace(r.cfg.Provider)) {
		settings = providerSettings{BaseURL: r.cfg.BaseURL, APIKeyEnv: r.cfg.APIKeyEnv}
		configured = true
	}
	if !configured {
		return nil, fmt.Errorf("model: provider %s is not configured: %w", name, ErrUnavailable)
	}
	base := settings.BaseURL
	if base == "" {
		base = defaultBaseURLs[name]
	}
	if base == "" {
		return nil, fmt.Errorf("model: provider %s has no baseURL: %w", name, ErrUnavailable)
	}
	key := ""
	if settings.APIKeyEnv != "" {
		key = r.lookup(settings.APIKeyEnv)
		if key == "" {
			return nil, fmt.Errorf("model: provider %s API key variable %q is unset: %w", name, settings.APIKeyEnv, ErrUnavailable)
		}
	}
	return NewOpenAIProvider(name, base, key, r.cfg.Timeout), nil
}

func resolveAuto(cfg Config, lookup func(string) string) (Provider, error) {
	// auto prefers a configured provider, then sampling, then fails
	// cleanly (#1925's table). "Configured" here means the user actually
	// configured something -- an apiKeyEnv that names a set variable, or
	// an explicit baseURL. auto never invents a key by scanning for
	// known names.
	//
	// Sampling is NOT first here, whatever #1925's table hoped: the
	// daemon cannot tell a live sampling path from a merely installed
	// broker (the broker exists whether or not any sampling-capable
	// client or answerer is attached), so preferring sampling would
	// park every auto request on a question nobody may ever answer,
	// until expiry, ahead of an explicitly configured endpoint.
	// Sampling is therefore the fallback for auto, and the first choice
	// only when the user names model.provider=sampling outright.
	if cfg.BaseURL != "" || (cfg.APIKeyEnv != "" && lookup(cfg.APIKeyEnv) != "") {
		name, _ := SplitModelID(cfg.DefaultModel)
		if _, ok := defaultBaseURLs[name]; !ok {
			name = "openai"
		}
		return configured(cfg, name, lookup), nil
	}
	if cfg.Sampler != nil {
		return NewSamplingProvider(cfg.Sampler), nil
	}
	return nil, nil
}

// configured builds the OpenAI-compatible backend for a named provider,
// honouring model.baseURL and model.apiKeyEnv.
func configured(cfg Config, name string, lookup func(string) string) Provider {
	base := cfg.BaseURL
	if base == "" {
		base = defaultBaseURLs[name]
	}
	key := ""
	if cfg.APIKeyEnv != "" {
		key = lookup(cfg.APIKeyEnv)
	}
	p := NewOpenAIProvider(name, base, key, cfg.Timeout)
	p.defaultModel = ModelID(cfg.DefaultModel)
	return p
}
