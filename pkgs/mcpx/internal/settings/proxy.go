package settings

import "github.com/developing-today/code/pkgs/mcpx/internal/defaults"

// proxySettings govern requests the daemon forwards to an LLM provider.
func proxySettings() []Setting {
	return []Setting{
		{
			Path: "proxy.timeout", Scope: ScopeDaemon, Kind: KindDuration,
			Default:  defaults.Builtin().Proxy.Timeout,
			Commands: []string{"daemon"},
			Name:     "Proxy upstream timeout",
			Short:    "how long the daemon waits for a chat-completions upstream",
			Long: "The deadline covers the upstream response, including reading its body. " +
				"A timeout before response headers arrive is returned to the caller as " +
				"a 502. A stream that exceeds the deadline after headers have been sent " +
				"is cut off, because an HTTP response status cannot be changed mid-stream.",
		},
	}
}
