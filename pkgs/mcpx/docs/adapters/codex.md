# Codex adapter: decided integration

Decision recorded on #1861 on 2026-11-27; this record checked 2026-11-28
against mcpx `main` at `b055c0bdb5c31e8ddc673c00ca55bb36012cec5c`, where
`pkgs/mcpx/docs/adapters/` contained only the Claude Code feasibility record
(`claude-code.md`, PR #15849). This is the Codex counterpart of that record:
it documents the decided integration; it does not add an adapter or change
the shared client.

## Recommendation

**Go for tool access through mcpx's existing MCP server; no Codex-specific
plugin layer is needed, and the shared core does not move for this adapter.**
OpenAI's Codex supports local STDIO and remote Streamable HTTP MCP servers
in the shared `config.toml`, and the CLI exposes `codex mcp add`. The MCP
STDIO specification is a child process speaking JSON-RPC over stdin/stdout.
mcpx already exposes its server over STDIO/HTTP, while the direct plugin
core uses Bun's Unix-socket fetch; the Codex adapter stays at the MCP
process boundary:

```sh
mcpx serve --transport stdio
```

**No-go for Codex-session attribution until a test proves an identity
reaches mcpx.** Codex's public MCP documentation and configuration do not
specify a stable Codex thread/session ID forwarded in request `_meta`, so
session attribution remains absent unless a future test proves one is
available. A stdio mcpx server uses its own per-process session identity;
do not claim Codex-session attribution on the basis of the configuration
alone.

## Capability map

| Capability needed by an adapter | Codex surface                                                                                                                             | mcpx implication                                                                                                                                                                    |
| ------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Tool access                     | MCP client supports local STDIO and remote Streamable HTTP servers, configured in the shared `config.toml` or added with `codex mcp add`. | The existing stdio server is sufficient; no harness-specific tool wrapper is needed.                                                                                                |
| Plugin packaging                | No Codex-specific mcpx plugin layer is added (decided).                                                                                   | Packaging, if ever wanted, is a later decision; it is not a prerequisite for tool access and does not require a new mcpx daemon plugin API.                                         |
| Working directory               | The stdio server is a child process of the host. The decision record does not establish a per-request cwd field.                          | `mcpx serve` scopes from its process cwd. Treat cwd inheritance from a Codex session as an integration assumption; verify it before relying on `cwd`, `repo`, or `worktree` scopes. |
| Session identity                | Public MCP docs/config do not specify a stable Codex thread/session ID forwarded in request `_meta`.                                      | Stdio is isolated by mcpx's process identity, not Codex's session ID. On a shared HTTP daemon, unnamed requests get no session continuity.                                          |
| Environment / call wrapping     | MCP server configuration can set the server process environment.                                                                          | Startup environment is available; there is no documented per-call environment injection into an MCP server.                                                                         |

## Minimal configuration

For a local installation with `mcpx` on `PATH`, register the server with
the CLI:

```sh
codex mcp add mcpx -- mcpx serve --transport stdio
```

or declare it in the shared `config.toml`:

```toml
[mcp_servers.mcpx]
command = "mcpx"
args = ["serve", "--transport", "stdio"]
```

The host owns the child process lifecycle for a stdio server, so the
adapter must preserve normal process signal handling — the same constraint
the Claude Code record names for its stdio path.

## Runtime compatibility

The MCP server boundary is runtime-neutral: Codex launches the configured
stdio command, and mcpx's server side does not depend on the plugin core's
Bun Unix-socket client. That separation is why this adapter needs no
harness-specific code: anything Codex requires beyond the generic MCP
server surface is, by the decision's own terms, new evidence — not a
reason to pre-build a plugin layer.

## Undo condition

If a tested Codex MCP call requires a host lifecycle capability the
generic server cannot supply, add a harness-specific adapter and document
the missing capability **before** moving shared core code. Until such a
test exists, the four-harness shape of #1861 is served for Codex by this
configuration alone; the pi and manus adapters, and the Claude Code
two-session stdio integration probe named in `claude-code.md`, remain the
issue's open work.
