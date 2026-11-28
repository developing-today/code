# Manus adapter: feasibility record

Research checked 2026-11-28 against the current Manus documentation and
mcpx `main` at `9133236c7db61a49361065130f45efa984e879ef`, where
`pkgs/mcpx/docs/adapters/` contained the Claude Code feasibility record
(`claude-code.md`, PR #15849), the Codex decided-integration record
(`codex.md`, PR #16386), and the Pi feasibility record (`pi.md`). This is
the Manus counterpart of those records: it documents the integration
surface; it does not add an adapter or change the shared client.

Manus differs from the other three harnesses in kind, not degree: it is
a hosted agent product. Tasks run in Manus's own cloud environment, not
in a process on the user's machine, and there is no third-party plugin
or extension API analogous to opencode's plugins, Pi's extensions, or
Claude Code's plugin bundles. The only integration surface Manus offers
a tool provider is its Connectors system.

## Recommendation

**Go, narrowly, for tool access against a deliberately exposed remote
mcpx endpoint; no Manus-specific adapter code exists to write.** Manus
Connectors can add a custom MCP server by URL (a remote Streamable HTTP
server), or through Custom MCP with a command or an imported JSON
configuration. mcpx's daemon already serves MCP over HTTP, so a hosted
mcpx endpoint can be registered as a Manus connector with configuration
alone.

**No-go for local-daemon access.** A Manus task does not run on the
user's machine, so no connector configuration reaches the user's local
mcpx daemon, its Unix socket, or a stdio server spawned from the user's
`PATH`. The command form of Custom MCP is entered in Manus's product
UI; the documentation checked for this record does not establish that
such a command executes on the user's machine, and this record treats
it as Manus-side until Manus documents otherwise. Any Manus use of mcpx
therefore means exposing an mcpx endpoint Manus's cloud can reach —
which makes transport authentication a prerequisite design item, not a
detail (security/auth is deferred on #1861's tracker; nothing here
pre-answers it).

**No-go for Manus-session attribution.** The Manus connector
documentation checked for this record does not specify a Manus task or
session ID forwarded in an MCP request's `_meta`. Requests arriving at a
shared remote mcpx endpoint carry no documented Manus identity, so
session-scoped upstream state must not be keyed to a Manus task on the
basis of the connector configuration alone.

## Capability map

| Capability needed by an adapter | Manus surface                                                                                                                                        | mcpx implication                                                                                                                                                     |
| ------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Tool access                     | Connectors: add a custom MCP server by URL (remote), or Custom MCP by command or imported JSON configuration.                                          | Only the by-URL route has a clear mcpx counterpart: the daemon's HTTP MCP endpoint, deliberately exposed. The command route's execution environment is undocumented (see Recommendation). |
| Plugin packaging                | No third-party plugin or extension API; connectors are account-level configuration in the Manus product.                                              | There is nothing to package. The adapter is a documented connector configuration, full stop.                                                                            |
| Working directory               | No user-machine working directory exists for a hosted task.                                                                                           | `cwd`, `repo`, and `worktree` scopes cannot be supplied by Manus; an exposed endpoint must not silently inherit the endpoint host's cwd as if it were the user's.        |
| Session identity                | Manus tasks have product-side identity, but no documented forwarding of a task/session ID in MCP request `_meta`.                                      | No session continuity or attribution on a shared endpoint. Per-process stdio identity — the fallback the other three records rely on — does not exist here.              |
| Environment / call wrapping     | API-key connectors store a credential with a usage note; MCP connector setup is sign-in or key based, per the server. No per-call environment surface. | Startup credential only, held by Manus. No per-call injection, no tool-call hooks, no post-call timing hook — capabilities 3–5 of the #1861 adapter contract degrade to absent. |
| Display / questions             | No documented channel for an MCP server to show lines in, or elicit answers through, the Manus task UI beyond ordinary tool results.                 | Ambiguity and warning display (capability 5) and question routing (capability 6) are absent; elicitation-dependent flows must degrade, as the contract allows.            |

## Minimal configuration

In Manus: open Connectors, choose Create, then **Add MCP by URL**, and
register the deliberately exposed mcpx daemon endpoint:

```text
Name:     mcpx
Server URL: https://<exposed-mcpx-host>/mcp
```

(the daemon's MCP path on current main; see
[`internal/cli/serve.go`](../../internal/cli/serve.go) — HTTP MCP is
served by the daemon, while `mcpx serve` is stdio-only and therefore
unusable from Manus). Authentication for that endpoint is the open
design item named in the Recommendation; do not expose an
unauthenticated daemon endpoint to satisfy this configuration.

There is no local configuration file counterpart: unlike Codex's
`config.toml` or Pi's `mcp.json`, the connector lives in the Manus
account, so nothing ships in this repository for Manus beyond this
record.

## Runtime compatibility

The runtime question that shapes the other adapters — the plugin core's
Bun Unix-socket client versus the harness's runtime — does not arise:
Manus runs no mcpx code. The entire compatibility surface is the MCP
HTTP protocol between Manus's cloud and the exposed endpoint, plus the
identity and scope absences in the capability map.

## Next implementation slice

Before any endpoint is exposed: a two-part design-and-probe slice.
First, the authentication design for a remotely reachable mcpx daemon
endpoint, recorded against #1861's deferred security/auth tracker.
Second, with that design in place, a single Manus task probe against a
test endpoint: discover and call one tool, and capture exactly what
identity, headers, and `_meta` the endpoint receives, so the
attribution no-go above is either confirmed with evidence or replaced
by what the probe actually observes. No connector recommendation for
real use precedes that probe.

## Evidence and sources

Manus primary sources (checked 2026-11-28):

- [Manus Connectors feature page](https://manus.im/features/connectors) — the three connection routes (app connectors, custom MCP by URL / command / JSON import, saved API keys).
- [Manus Connectors documentation](https://manus.im/docs/integrations/connectors) — connector types (App / MCP / API key), account sign-in or key per server, and the custom-MCP pointer for services outside the catalog.

Current mcpx contracts:

- [`docs/plugins.md`](../plugins.md) — MCP-only hosts, per-process identity, and adapter compatibility matrix (the `manus` row's unknowns are what this record answers).
- [`docs/spec/identity.md`](../spec/identity.md) — stdio and HTTP identity rules and accepted `_meta` session keys.
- [`internal/cli/serve.go`](../../internal/cli/serve.go) — stdio server setup and caller context; HTTP MCP is served by the daemon, not by `mcpx serve`.

The conclusions about where a Custom MCP command executes and about
Manus task identity in MCP `_meta` are intentionally marked as
undocumented until the probe named above observes them. With this
record, all four harnesses named in #1861 have adapter records; the
issue's remaining work is the probes and any adapter code those probes
justify.
