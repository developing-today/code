package settings

import "github.com/dezren39/mcpx/internal/defaults"

// transportSettings govern mcpx's own MCP server on the wire: what reaches
// it, how long a quiet stream is kept alive, and how it shuts down.
func transportSettings() []Setting {
	return []Setting{
		{
			Path: "transport.allowedOrigins", Scope: ScopeDaemon, Kind: KindList,
			Default: defaults.CSV(defaults.TransportAllowedOrigins), Repeatable: true,
			Commands: []string{"daemon"},
			Name:     "Allowed origins",
			Short:    "browser Origins, beyond loopback, that may reach /mcp",
			Long: "The specification requires a server to refuse a request whose " +
				"Origin header it does not recognise, because otherwise any web page " +
				"can reach a local server through DNS rebinding. Loopback origins and " +
				"the daemon's own address are always allowed, and a request with no " +
				"Origin (every non-browser client) is never refused. List exact " +
				"origins here, scheme and port included: https://app.example.com.",
		},
		{
			Path: "transport.sessionMetaKeys", Scope: ScopeDaemon, Kind: KindList,
			Default: defaults.CSV(defaults.TransportSessionMetaKeys), Repeatable: true,
			Commands: []string{"daemon"},
			Name:     "Session meta keys",
			Short:    "_meta keys, in order, a client may name its own session with",
			Long: "A client names its own session in a request's _meta, and the " +
				"first key in this list that carries a non-empty string wins. " +
				"The default reads dev.mcpx/session (mcpx's own key) and then " +
				"ai.opencode/sessionID, the key OpenCode v2 sends on every " +
				"tools/call, so each OpenCode session gets its own scope " +
				"instead of sharing the daemon's. See docs/spec/identity.md.",
		},
		{
			Path: "transport.sseKeepAlive", Scope: ScopeDaemon, Kind: KindDuration,
			Default:  defaults.Str(defaults.TransportSSEKeepAlive),
			Commands: []string{"daemon"},
			Name:     "Event stream keep-alive",
			Short:    "how often a quiet MCP event stream carries a comment line",
			Long: "A proxy or a client idle timeout closes a stream that has said " +
				"nothing for a while, and a notification stream is quiet by nature.",
		},
		{
			Path: "transport.stdioDrain", Scope: ScopeClient, Kind: KindDuration,
			Default:  defaults.Str(defaults.TransportStdioDrain),
			Commands: []string{"serve"},
			Name:     "stdio drain",
			Short:    "how long `mcpx serve` finishes in-flight requests after its input closes",
			Long: "Closing stdin is how a host asks a server to stop, and a server " +
				"should exit promptly. Requests already running are allowed this " +
				"long to answer, then cancelled.",
		},
	}
}
