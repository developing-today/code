# Roadmap: mcpx

`mcpx` is the Go MCP gateway and Code Mode runner in `pkgs/mcpx`. It pools MCP servers behind one daemon, runs TypeScript scripts against them, and hands a running daemon to its successor during upgrades.

This repo is the only source for `mcpx`. The macOS flake in `dezren39/nix` deleted its vendored copy on 2026-10-19 (dezren39/nix#468) and now builds mcpx from a pinned source-only input of this repo, and the OpenCode and OpenChamber forks consume the result. Sources: [`pkgs/mcpx/TODO.md`](../../pkgs/mcpx/TODO.md), [`pkgs/mcpx/docs/`](../../pkgs/mcpx/docs/), and the large mcpx backlog in `dezren39/nix`, which issue #309 moves here. Backlog numbers below are written as `nix#N`. How to read this file: [planning README](../README.md).

## Now

| Title                                           | Status      | Intent                                                                                                                                                                                     | Links      |
| ----------------------------------------------- | ----------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ | ---------- |
| How downstream flakes consume mcpx              | in-progress | This repo is the single source (the `dezren39/nix` copy was deleted on 2026-10-19). What remains is the consumption decision and re-homing the backlog that still lives in `dezren39/nix`. | #308, #309 |
| Takeover tests on darwin                        | proposed    | The package builds and tests on aarch64-darwin without a skip list kept in another repo.                                                                                                   | #237       |
| Unreliable orphan-session test in the Nix build | proposed    | `TestTerminateOrphanedSessionEndsTheUpstreamSession` sees 0 DELETEs in the Nix build and hides why.                                                                                        | #290       |

## Next

| Title                                           | Status   | Intent                                                                                                                                                                            | Links                           |
| ----------------------------------------------- | -------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ------------------------------- |
| A supervisor that owns each stdio child's pipes | idea     | Today's takeover handoff is a stopgap that passes pipes over a unix socket. A supervisor that outlives the daemon makes a daemon upgrade a plain restart of the front end.        | [TODO](../../pkgs/mcpx/TODO.md) |
| Resume upstream sessions after a cold restart   | idea     | The upstream `Mcp-Session-Id` is kept only in memory and in the takeover handoff, so every HTTP upstream is initialised again after a restart.                                    | [TODO](../../pkgs/mcpx/TODO.md) |
| Decide what a stdio server's identity is        | proposed | `ExecIdentity` is the bytes of the executable, which is wrong for a script that finds files relative to its location. Include the directory, or always restart a re-pointed link. | [TODO](../../pkgs/mcpx/TODO.md) |
| JSON Schema validation of tool input and output | proposed | Seven conformance rows depend on it.                                                                                                                                              | nix#254                         |
| Legacy client robustness                        | proposed | Session 404, GET stream, SSE retry, string ids, stream timeout and the version header.                                                                                            | nix#203, nix#200, nix#201       |
| Tasks, which are advertised and not implemented | proposed | The 2026 tasks extension is advertised, and core tasks live in two stores.                                                                                                        | nix#209                         |
| Script runtime follow-ups                       | proposed | Capability probe and clearer parse-failure diagnostics for `--runtime node`, and recipes with `@param` holes.                                                                     | nix#108, nix#367                |

## Later

| Title                               | Status   | Intent                                                                                                                                 | Links                     |
| ----------------------------------- | -------- | -------------------------------------------------------------------------------------------------------------------------------------- | ------------------------- |
| A write policy                      | proposed | Ask, allow or deny per call, with the exact call shown when asking. Argument rules and a stop after a write of unknown outcome follow. | nix#340, nix#355, nix#339 |
| A bearer token for `/v1` and `/mcp` | proposed | Optional authentication on the HTTP surfaces.                                                                                          | nix#338                   |
| Principals and permissions          | idea     | A principal per surface and a permission per operation, declared permissively. The permissive default currently authorizes everyone.   | nix#298, nix#300          |
| An OAuth authorization server       | idea     | Research item.                                                                                                                         | nix#345                   |
| Front codex-connectors-mcp          | idea     | OAuth-authenticated apps without mcpx doing OAuth.                                                                                     | nix#342, nix#346          |
| Packaging test and Nix modules      | proposed | Install the built artifact and check the MCP handshake. Package, service and config modules for NixOS, nix-darwin and home-manager.    | nix#341, nix#343          |
| Choose between MCP spec revisions   | idea     | A precedence list, `--mcp-spec` and version predicates where revisions conflict.                                                       | nix#307                   |
| Tool views and composed tool sets   | idea     | Rename, pin and hide tools, cross-server namespaces, script-backed tools.                                                              | nix#114                   |
| Hooks                               | idea     | Lifecycle hooks on exec and config-declared hooks over the event bus.                                                                  | nix#81, nix#112           |

## Ideas

| Title                                  | Status | Intent                                                                       | Links   |
| -------------------------------------- | ------ | ---------------------------------------------------------------------------- | ------- |
| WebAssembly as a sandboxed tool source | idea   | wazero, and the component model as the tool boundary.                        | nix#299 |
| Roc as a script language               | idea   | Whether mcpx should be a Roc platform.                                       | nix#296 |
| Reach a daemon by identity             | idea   | Find a daemon by its Iroh identity, and the Go-binding question that raises. | nix#297 |
| A sentence in, a reviewed script out   | idea   | Natural language to script.                                                  | nix#294 |
| Embedding mcpx                         | idea   | A public Go API, a C ABI and WASM.                                           | nix#84  |
| A web frontend served by the daemon    | idea   | No second build system.                                                      | nix#86  |
| Adapter import                         | idea   | A declaration generated from completions, man pages and `--help`.            | nix#295 |

## Done

Completed work is recorded in GitHub — see closed issues and merged PRs.
