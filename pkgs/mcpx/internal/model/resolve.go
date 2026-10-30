package model

import (
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

func resolveAuto(cfg Config, lookup func(string) string) (Provider, error) {
	// auto prefers a live sampling-capable client, then a configured
	// provider, then fails cleanly (#1925's table). "Configured" here
	// means the user actually configured something -- an apiKeyEnv that
	// names a set variable, or an explicit baseURL. auto never invents
	// a key by scanning for known names.
	if cfg.Sampler != nil {
		return NewSamplingProvider(cfg.Sampler), nil
	}
	if cfg.BaseURL != "" || (cfg.APIKeyEnv != "" && lookup(cfg.APIKeyEnv) != "") {
		name, _ := SplitModelID(cfg.DefaultModel)
		if _, ok := defaultBaseURLs[name]; !ok {
			name = "openai"
		}
		return configured(cfg, name, lookup), nil
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
	return NewOpenAIProvider(name, base, key, cfg.Timeout)
}
