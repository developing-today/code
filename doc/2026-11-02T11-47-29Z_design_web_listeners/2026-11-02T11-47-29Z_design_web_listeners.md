# Web listener bindings for `id serve`

## Request and intent

Allow one `id serve` process to expose the Explorer web UI on multiple
addresses. Preserve the existing single-listener behavior for callers that do
not opt in, report every effective listener at startup, and fail startup rather
than partially serving when any requested listener cannot bind. The design was
specified in the triage for issue #2145 and implemented on queue task 5191.

## Effective listener set

`serve` resolves bindings once, in this precedence order:

1. One or more repeatable `--web-listen IP:PORT` values define the entire set.
2. Otherwise, values from the `[web_bindings]` section in embedded
   `defaults.conf` define the set.
3. Otherwise, the existing `--bind` and `--port` options synthesize one
   listener, preserving current defaults and behavior.

The config format uses one socket address per line. Invalid entries fail
startup with the offending value identified. When `--web-listen` is supplied,
an explicitly supplied legacy `--bind` or `--port` is an error; implicit Clap
defaults do not conflict. This avoids silently ignoring caller intent.

Every listener serves the same Explorer router. The global `--web-token`
applies to all listeners; each listener receives security policy derived from
its own IP address. Non-loopback listeners without a token retain the existing
warning behavior, emitted per listener. Distinct per-listener tokens and
listener-specific program content are out of scope (issue #2146).

## Binding, reporting, and compatibility

All requested sockets are bound before the serve lock is written. A failure on
any address aborts startup and names that address; no partial listener set is
started. Startup prints a listener-count summary and one URL per listener, in
configured order, using the actual port selected by the OS when port zero is
requested.

`ServeInfo.web_port` remains the first listener's port for existing readers.
The additive `web_bindings` array records every address and defaults when
deserializing older lock files.

The resolved binding must retain the complete `SocketAddr`, not only its IP and
port: IPv6 scope IDs select link-local interfaces and are part of the requested
address. Derive the IP from that socket address only when applying per-listener
security policy.

When `--web` is disabled, serve must not construct web routers or their
`AppState`; this preserves the prior behavior where web persistence stores are
not opened or created by a non-web server.

## Verification and follow-up

The implementation and review follow-up are on PR #4118. Executable-level
tests cover repeated listener flags and explicit legacy-flag conflicts; a
no-web regression test protects startup from web-state initialization; unit
coverage verifies scoped IPv6 socket addresses survive resolution. Existing
tests continue to cover config parsing, precedence, lock compatibility, and
listener startup behavior.
