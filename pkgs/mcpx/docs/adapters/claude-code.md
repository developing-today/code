# Claude Code adapter feasibility

Research checked 2026-11-27 against the current Claude Code documentation and
mcpx `main` at `0c8fdf0bf58fe6322e14427c324cda3956c6beb8`. The referenced mcpx
contracts were inspected at `45e834d4896dd2426d653c4a0a0c2e8190ef66d7` and
confirmed unchanged through this `main` tip. This is a feasibility record; it
does not add an adapter or change the shared client.

## Recommendation

**Go for tool access through mcpx's existing MCP server; no Claude-specific
client plugin is needed for the first integration.** Claude Code accepts local
stdio MCP servers, and mcpx already exposes the daemon through
`mcpx serve --transport stdio`. Claude Code plugins can package MCP server
configuration alongside hooks, skills, and agents, so a later installable
bundle remains possible without moving the shared core.

**No-go for a session-attributing Claude adapter until an end-to-end probe
shows what identity reaches mcpx.** Claude Code supplies `session_id` to hook
handlers, but its public MCP client documentation does not specify a Claude
session ID in each MCP request's `_meta`. A stdio mcpx server therefore uses
its own per-process session identity (`mcp-<pid>`); a shared HTTP daemon gets no
Claude identity unless the client sends one. Do not claim Claude-session
attribution or share session-scoped upstream state through one HTTP daemon on
the basis of hook input alone.

## Capability map

| Capability needed by an adapter | Claude Code surface                                                                                                                                                                                | mcpx implication                                                                                                                                                                                              |
| ------------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Tool access                     | MCP client supports local stdio and remote HTTP (Streamable HTTP); SSE remains documented but is deprecated. WebSocket is also listed.                                                             | Existing stdio server is sufficient. A project `.mcp.json` can start `mcpx serve --transport stdio`; no harness-specific tool wrapper is needed.                                                              |
| Plugin packaging                | A Claude Code plugin can include `.mcp.json` plus hooks, skills, agents, and other components.                                                                                                     | Optional packaging can install the MCP server and any later hook scripts together. It does not require a new mcpx daemon plugin API.                                                                          |
| Working directory               | Hook input includes `cwd`. The MCP server configuration has a command and environment, but the MCP guide does not promise a per-request cwd field.                                                 | `mcpx serve` scopes from its process cwd. Treat cwd inheritance from a Claude session as an integration assumption; verify it before relying on `cwd`, `repo`, or `worktree` scopes.                          |
| Session identity                | Hook input includes a `session_id`. The MCP guide does not document that ID being forwarded in request `_meta`.                                                                                    | Stdio is isolated by mcpx's process identity, not Claude's stable session ID. On a shared HTTP daemon, unnamed requests get no session continuity. A hook's ID is not automatically visible to MCP calls.     |
| Environment / call wrapping     | MCP server config can set process environment. Hooks can match MCP tools as `mcp__<server>__<tool>` and run before, after, or on failure.                                                          | Startup environment is available; there is no documented per-call environment injection into an MCP server. Hooks can observe or gate tool calls, but are not a replacement for the existing MCP server path. |
| After-tool timing               | `PostToolUse` runs per tool; `PostToolBatch` runs once after a parallel batch.                                                                                                                     | Optional timing could be collected by a hook, but it is a separate Claude adapter feature. The first integration can rely on mcpx's own call logs.                                                            |
| Show a line of text             | Hooks can return context/system messages, and plugins can provide skills or agents. The docs do not establish a general persistent plugin UI/toast API equivalent to OpenCode's TUI notifications. | Do not add a display adapter for the basic MCP path. If a concrete Claude UX requires it, choose a supported hook output and test it on each target surface.                                                  |
| Ask the user                    | Claude Code advertises MCP elicitation support, including form and URL modes on its current MCP connections.                                                                                       | The generic MCP route is the candidate for upstream elicitation. Verify form and URL responses in an integration run before promising parity with OpenCode's prompt broker.                                   |
| Run a model                     | Claude Code's prompt/agent hooks can use the host model. The MCP client guide does not document Claude Code as an MCP sampling client.                                                             | Treat hook-based model evaluation and MCP sampling as different capabilities. Sampling-backed mcpx features remain unverified for Claude Code.                                                                |

## Minimal configuration

For a local installation with `mcpx` on `PATH`, the current MCP configuration
shape is:

```json
{
  "mcpServers": {
    "mcpx": {
      "type": "stdio",
      "command": "mcpx",
      "args": ["serve", "--transport", "stdio"]
    }
  }
}
```

Claude Code also accepts the same server through `claude mcp add` or a plugin's
MCP component. Project-scoped configuration may require workspace trust and
server approval. The host signals a stdio server when it disconnects and waits
briefly for shutdown, so the adapter must preserve normal process signal
handling.

## Runtime compatibility

The MCP server boundary is runtime-neutral: Claude Code launches the configured
stdio command, and mcpx's server is Go. The shared TypeScript plugin client is
not the path used by this adapter. `plugin/core/daemon.ts` uses Bun's
`fetch(url, { unix })` for Unix sockets; Node's built-in `fetch` ignores that
option. If a later Claude hook or plugin component needs the daemon API, use a
URL endpoint or add a separately tested Node `http` socket transport rather
than assuming the OpenCode/Bun client runs unchanged.

## Next implementation slice

Before adding code, run a small Claude Code integration probe with one stdio
mcpx server: discover and call a tool, verify a `session`-scoped upstream, and
capture the request identity mcpx actually sees. Repeat with two concurrent
Claude sessions. The expected baseline is separate `mcp-<pid>` scopes when
Claude starts one stdio server per session; if the host reuses a process or a
stable Claude session ID is required for audit continuity, record the observed
gap and design a narrow hook-to-client bridge before writing a Claude-specific
adapter. The probe should also exercise an MCP elicitation form if question
routing is part of the first user-facing slice.

## Evidence and sources

Claude Code primary sources:

- [MCP server transports, configuration, lifecycle, and elicitation](https://code.claude.com/docs/en/mcp)
- [Plugin overview and component model](https://code.claude.com/docs/en/plugins/overview)
- [Plugin manifest and MCP component reference](https://code.claude.com/docs/en/plugins/manifest-reference)
- [Hook events, common fields, MCP tool matching, and hook input/output](https://code.claude.com/docs/en/hooks)

Current mcpx contracts:

- [`docs/plugins.md`](../plugins.md) — MCP-only hosts, per-process identity, and adapter compatibility matrix.
- [`docs/spec/identity.md`](../spec/identity.md) — stdio and HTTP identity rules and accepted `_meta` session keys.
- [`internal/cli/serve.go`](../../internal/cli/serve.go) — stdio server setup and caller context.
- [`plugin/core/daemon.ts`](../../plugin/core/daemon.ts) — shared client transport, including the Bun Unix-socket option.

The conclusions about cwd inheritance, per-session stdio process reuse, and a
Claude-session `_meta` value are intentionally marked as assumptions or
undocumented until the integration probe observes them.
