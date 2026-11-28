# Pi adapter: feasibility record

Research checked 2026-11-28 against the current Pi documentation and
mcpx `main` at `9133236c7db61a49361065130f45efa984e879ef`, where
`pkgs/mcpx/docs/adapters/` contained the Claude Code feasibility record
(`claude-code.md`, PR #15849) and the Codex decided-integration record
(`codex.md`, PR #16386). This is the Pi counterpart of those records: it
documents the integration surface; it does not add an adapter or change
the shared client.

## Recommendation

**Go for tool access through mcpx's existing MCP server; no Pi-specific
plugin is needed for the first integration.** Pi 1.0 (earendil-works/pi,
October 2026) ships a built-in MCP client supporting local stdio and
remote Streamable HTTP servers, configured in `mcp.json` and managed
with `pi mcp` commands. mcpx already exposes its server over stdio
(`mcpx serve --transport stdio`), and the daemon serves HTTP itself, so
both of Pi's transports have an existing mcpx counterpart.

**No-go for Pi-session attribution until a probe proves what identity
reaches mcpx.** Pi keeps a session identity of its own (sessions are
JSONL trees under `~/.pi/agent/sessions/`), and its TypeScript extension
API can observe session state, but the Pi MCP documentation checked for
this record does not specify a Pi session ID forwarded in an MCP
request's `_meta`. A stdio mcpx server therefore uses its own
per-process session identity, as with Claude Code and Codex. Unlike
those hosts, Pi has a real extension API, so an extension-side bridge is
a plausible later design — it is named as the probe below, not assumed.

## Capability map

| Capability needed by an adapter | Pi surface                                                                                                                                                              | mcpx implication                                                                                                                                                          |
| ------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Tool access                     | Built-in MCP client: stdio and Streamable HTTP servers (legacy SSE rejected), configured in `~/.pi/agent/mcp.json` or project `.pi/mcp.json` (project file is trust-gated). | The existing stdio server is sufficient; no harness-specific tool wrapper is needed.                                                                                      |
| Tool exposure                   | Per-server `exposure`: `codemode` (default — tools callable from Pi's JavaScript sandbox scripts, not declared to the model), `deferred`, `direct`, or `hidden`; per-tool overrides via `toolExposure`. Tools are named `mcp__<server>__<tool>`. | mcpx tools remain callable under every exposure mode; `direct` matches the other harnesses' default presentation. Codemode composition is Pi-side and needs no mcpx change. |
| Plugin packaging                | Pi extensions are TypeScript modules with a deep API (custom tools, commands, event hooks, UI). No extension is required for MCP tool access.                          | Packaging an extension bundle is a later option, not a prerequisite; the settings guard (`TestThePluginReadsNoUndeclaredSetting`) already covers `plugin/**/*.ts(x)` if one is ever added. |
| Working directory               | A stdio entry accepts `cwd`; a relative value resolves against the session directory.                                                                                  | Stronger than the Codex record's assumption: the mcpx stdio server's process cwd can be pinned per server entry instead of merely inherited.                              |
| Session identity                | Sessions are JSONL trees in `~/.pi/agent/sessions/`; extensions can observe session state. No documented forwarding of a session ID in MCP request `_meta`.             | Stdio is isolated by mcpx's process identity, not Pi's session ID. Do not claim Pi-session attribution until the probe below observes what arrives.                        |
| Environment / call wrapping     | Stdio entries accept `env`; values support `${VAR}` expansion and `!command` substitution. Extensions can hook tool calls.                                              | Startup environment is available at the server boundary; per-call injection into an MCP server is not part of the documented MCP surface.                                 |
| Lifecycle                       | Stopping a stdio server closes stdin, then sends SIGTERM, then SIGKILL to its process group.                                                                              | The adapter must preserve normal process signal handling — the same constraint the Claude Code and Codex records name for their stdio paths.                              |

## Minimal configuration

For a local installation with `mcpx` on `PATH`, register the server with
the CLI:

```sh
pi mcp add mcpx -- mcpx serve --transport stdio
```

or declare it in `~/.pi/agent/mcp.json`:

```json
{
  "mcpServers": {
    "mcpx": {
      "command": "mcpx",
      "args": ["serve", "--transport", "stdio"]
    }
  }
}
```

Add `"exposure": "direct"` to the entry if mcpx tools should be declared
to the model like built-in tools instead of reached through codemode
scripts. `pi mcp list` connects to every enabled server and reports
state, tools, and errors.

## Runtime compatibility

Pi is a Node.js program and its extensions are TypeScript, but the MCP
server boundary is runtime-neutral: Pi launches the configured stdio
command, and mcpx's server side does not depend on the plugin core's Bun
Unix-socket client. That separation is why this adapter needs no
harness-specific code for tool access. The Bun-socket question only
returns if a future Pi extension wants the direct `/v1` daemon client
instead of the MCP server; that is the shared-core extraction's concern,
not this record's.

## Next implementation slice

Run a small Pi integration probe with one stdio mcpx server: discover
and call a tool under both `codemode` and `direct` exposure, verify a
`session`-scoped upstream, and capture the request identity mcpx
actually sees. Repeat with two concurrent Pi sessions. The expected
baseline is separate per-process scopes when Pi starts one stdio server
per session. Because Pi extensions can observe session state, the probe
should also test whether an extension can stamp a stable Pi session ID
onto outbound MCP calls at all; if it cannot, record that gap instead of
designing the bridge. The probe should also exercise an MCP elicitation
form if question routing is part of the first user-facing slice.

## Evidence and sources

Pi primary and reference sources (checked 2026-11-28):

- Pi project site and documentation (pi.dev) and the earendil-works/pi
  repository — MCP configuration, `pi mcp` commands, exposure modes,
  extension API, and session storage, as of Pi 1.0.
- Earendil, "You said no MCP!" (2026) — the reversal that made MCP a
  built-in: stdio and Streamable HTTP transports, `mcp.json`
  configuration, and the codemode default exposure.
- Independent 1.0-era walkthroughs of the same documented surface
  (Flavio Copes, "A deep dive into Pi", checked against Pi 1.0,
  October 2026; Mervin Praison's MCP/codemode summary of the Pi docs).

Current mcpx contracts:

- [`docs/plugins.md`](../plugins.md) — MCP-only hosts, per-process identity, and adapter compatibility matrix (the `pi` row's unknowns are what this record answers).
- [`docs/spec/identity.md`](../spec/identity.md) — stdio and HTTP identity rules and accepted `_meta` session keys.
- [`internal/cli/serve.go`](../../internal/cli/serve.go) — stdio server setup and caller context; HTTP MCP is served by the daemon, not by `mcpx serve`.
- [`plugin/core/daemon.ts`](../../plugin/core/daemon.ts) — shared client transport, including the Bun Unix-socket option.

The conclusions about a Pi session ID in MCP `_meta` and about
extension-side identity stamping are intentionally marked as
undocumented until the integration probe observes them. With this
record, the manus adapter remains #1861's open record work.
