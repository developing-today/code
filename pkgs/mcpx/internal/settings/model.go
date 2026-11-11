package settings

import (
	"github.com/developing-today/code/pkgs/mcpx/internal/defaults"
)

// modelSettings are the knobs for mcpx's own model layer (#1925).
//
// Everything model-shaped goes through this registry -- a config key, an
// MCPX_* variable, a CLI flag, /v1/settings and an MCP tool -- and never
// through a new endpoint, exactly as prompt.* did before it. The default
// is none: a user who configures nothing gets the behaviour mcpx has
// always had, and no operation calls a model until they say so. That
// matches prompt.sample's never, for the same reason: asking a model is
// the one thing here that spends somebody else's money and time.
//
// model.budget's default of 0 means no layer-imposed ceiling: the
// per-call bounds that already exist (prompt.maxTokens and friends) keep
// applying. A positive budget caps every request through the layer.
func modelSettings() []Setting {
	return []Setting{
		{
			Path: "model.provider", Scope: ScopeDaemon, Kind: KindEnum, Default: "none",
			Enum: []string{"auto", "sampling", "plugin", "openai", "anthropic", "google", "none"},
			Name: "Model provider",
			Short: "where mcpx's own generation gets a model: a configured provider, " +
				"sampling through the caller, or none",
			Long: "The provider layer is internal/model (#1925): one Generate " +
				"function, several backends, chosen here. none -- the default -- " +
				"means no operation calls a model, which is the behaviour mcpx " +
				"has always had. sampling asks the caller's model through the " +
				"broker (deprecated by the 2026-07-28 MCP revision, SEP-2577, " +
				"and unanswered by opencode, but correct where a client does " +
				"declare it). openai, anthropic and google are configured " +
				"providers reached through one OpenAI-compatible client, so " +
				"model.baseURL can point them at a gateway, LiteLLM, Ollama or " +
				"LM Studio instead. plugin is the plugin backend: a sampler " +
				"binding over the same broker, labelled for the plugin " +
				"answerer; naming it resolves to unavailable when the daemon " +
				"has no plugin answerer path installed. auto prefers a live sampling " +
				"path, then a configured provider, then fails cleanly.",
		},
		{
			Path: "model.baseURL", Scope: ScopeDaemon, Kind: KindString, Default: "",
			Name:  "Model base URL",
			Short: "the OpenAI-compatible endpoint the configured provider calls",
			Long: "Empty means the named provider's own endpoint. Set it to " +
				"point the one configured-provider backend at any " +
				"OpenAI-compatible service -- a company gateway, LiteLLM, " +
				"Ollama (http://localhost:11434/v1), LM Studio -- which is " +
				"the proxy case from #1925: a base URL, not a second " +
				"implementation.",
		},
		{
			Path: "model.apiKeyEnv", Scope: ScopeDaemon, Kind: KindString, Default: "",
			Name:  "Model API key variable",
			Short: "the environment variable holding the configured provider's API key",
			Long: "Opt-in by design: mcpx never scans the environment for " +
				"known key names on its own, because a key a user exported " +
				"for another tool is not consent to spend it. Name the " +
				"variable -- OPENAI_API_KEY, say -- and only that variable " +
				"is read, and only its value is sent, as a bearer token to " +
				"model.baseURL's endpoint. The key itself is never a setting " +
				"value: settings are printable, keys are not.",
		},
		{
			Path: "model.providers", Scope: ScopeDaemon, Kind: KindString, Default: "{}",
			Name:  "Provider-keyed model settings",
			Short: "JSON object mapping provider names to baseURL and apiKeyEnv",
			Long: "Optional provider-keyed OpenAI-compatible endpoints and " +
				"explicit environment-variable names, for example {\"openai\": " +
				"{\"apiKeyEnv\":\"OPENAI_API_KEY\"}, \"ollama\": " +
				"{\"baseURL\":\"http://localhost:11434/v1\"}}. A key is read " +
				"only from the variable named for that provider. Empty baseURL uses " +
				"the provider's built-in endpoint when one exists. The singular " +
				"model.baseURL/model.apiKeyEnv remain the fallback for the " +
				"provider named by model.provider.",
		},
		{
			Path: "model.profiles", Scope: ScopeDaemon, Kind: KindString, Default: "{}",
			Name:  "Model profiles",
			Short: "JSON object of named ordered provider/model fallback chains",
			Long: "Each profile maps a name to {\"models\":[\"openai/gpt-5\", " +
				"\"anthropic/claude-sonnet\"], \"budget\":2048, " +
				"\"timeout\":\"30s\"}. Models are tried in order; an " +
				"unconfigured or failed provider is skipped. An operation's model " +
				"selection and model.default may name either a profile or a " +
				"provider/model id. A profile budget caps the per-call token ask; " +
				"timeout bounds each provider attempt.",
		},
		{
			Path: "model.default", Scope: ScopeDaemon, Kind: KindString, Default: "",
			Name:  "Default model",
			Short: "the model id used when an operation names none, as provider/model",
			Long: "Model ids live in the provider/model namespace (#1925): " +
				"openai/gpt-5, ollama/qwen3:8b. The prefix says which " +
				"provider the id belongs to and is stripped before the id " +
				"reaches that provider's API. Empty means no default: " +
				"an API backend then requires the operation itself to " +
				"name a model (an unnamed request to it is unavailable " +
				"rather than sent under an invented id), and sampling " +
				"asks with no model hint.",
		},
		{
			Path: "model.budget", Scope: ScopeDaemon, Kind: KindInt, Default: "0",
			Name:  "Model token budget",
			Short: "hard ceiling on the tokens one generation through the layer may ask for; 0 is no ceiling",
			Long: "A bound on the layer itself, beside the per-feature bounds " +
				"(prompt.maxTokens for intent generation): every request " +
				"through internal/model is capped at this many maxTokens " +
				"when it is positive. 0 -- the default -- imposes no ceiling " +
				"of its own, so existing per-feature bounds are unchanged.",
		},
		{
			Path: "model.timeout", Scope: ScopeDaemon, Kind: KindDuration, Default: defaults.ModelTimeout.String(),
			Name:  "Model call deadline",
			Short: "how long one generation through the layer may take",
			Long: "The deadline on a single Generate call, in the shape of " +
				"prompt.sampleTimeout: when it expires the provider is " +
				"treated as unavailable and the caller falls back to its " +
				"deterministic result, with a note that the model did not " +
				"answer.",
		},
		{
			Path: "sampling.answerLocally", Scope: ScopeDaemon, Kind: KindBool, Default: "false",
			Name:  "Answer sampling locally",
			Short: "answer upstream sampling requests with mcpx's configured model provider instead of relaying them to the caller",
			Long: "Slice 3 of #1925. Upstream servers send sampling/" +
				"createMessage requests to mcpx, and mcpx has always " +
				"relayed them through the broker to whatever drives it -- " +
				"but the primary consumer never declares the sampling " +
				"capability, so the relay goes unanswered. With this set, " +
				"a sampling request is instead answered by the configured " +
				"model layer (model.provider / model.profiles through " +
				"internal/model Resolve), with model.budget and " +
				"model.timeout as its bounds. The model.* settings stay " +
				"the source of truth: enabling this with no provider " +
				"configured fails the request cleanly rather than hanging " +
				"or silently succeeding. Default false -- the relay " +
				"behaviour mcpx has always had is unchanged.",
		},
	}
}
