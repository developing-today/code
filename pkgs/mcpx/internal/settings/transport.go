package settings

import "github.com/developing-today/code/pkgs/mcpx/internal/defaults"

// transportSettings govern mcpx's own MCP server on the wire: what reaches
// it, how long a quiet stream is kept alive, and how it shuts down.
func transportSettings() []Setting {
	return []Setting{
		{
			Path: "transport.bearerToken", Scope: ScopeDaemon, Kind: KindString, Default: "",
			Commands: []string{"daemon"},
			Name:     "Bearer token",
			Short:    "when set, every /v1 and /mcp request must carry this token as a Bearer credential",
			Long: "Optional shared-secret auth for the daemon's HTTP surfaces. Unset (the " +
				"default), the daemon behaves exactly as it always has: the socket's file " +
				"permissions are the access control. Set, every /v1 and /mcp request -- on the " +
				"socket and the loopback listeners alike -- must carry 'Authorization: Bearer " +
				"<token>' or is refused 401; the comparison is constant-time. First-party " +
				"callers (the CLI, and through it scripts and the plugin) read the same setting " +
				"and attach the header. Source it from the environment " +
				"(MCPX_TRANSPORT_BEARER_TOKEN) so the token need not live in a configuration " +
				"file. A token shorter than 32 characters is refused at daemon startup. The " +
				"value is never logged or echoed in an error. This is a static shared token by " +
				"design -- not OAuth (#1950) and not per-server upstream auth (#1926).",
		},
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
			Default: defaults.CSV(defaults.TransportSessionMetaKeys), Repeatable: true, Hot: true,
			Commands: []string{"daemon", "serve"},
			Name:     "Session _meta keys",
			Short:    "the _meta keys a client may name its session with, first present wins",
			Long: "Every request mcpx serves gets an identity, and a client that wants " +
				"continuity across its calls names itself in `_meta`. This is the " +
				"precedence list mcpx reads: the first key present on a request wins, " +
				"and its value is used verbatim as the session id. mcpx's own " +
				"dev.mcpx/session comes first so an explicit name beats the one a host " +
				"attaches automatically; ai.opencode/sessionID is what opencode v2 puts " +
				"on every tools/call, including calls from inside its code mode. A " +
				"request that sends none keeps the fallback: its connection's identity " +
				"where the transport gives one, otherwise its own call scope. " +
				"See docs/spec/identity.md.",
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
