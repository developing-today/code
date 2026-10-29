# Roadmap

This is the overall roadmap for the repository. The repo holds several projects that move on their own schedules, so each has a roadmap file in [`planning/roadmaps/`](planning/roadmaps/). This page shows where they stand next to each other. How the roadmap works, and how it connects to design records and plans, is in [`planning/README.md`](planning/README.md).

A row here records intent. It does not settle a product question and it is not a go-ahead to build.

## Projects

| Project                                        | Roadmap                                                   | Current focus                                                                                                                                             |
| ---------------------------------------------- | --------------------------------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `id`, the peer-to-peer CLI and worlds platform | [id](planning/roadmaps/id.md)                             | Phase 11 foundations: a CI signal that can be trusted, visible costs, one rule for what a client may see                                                  |
| `mcpx`, the MCP gateway and Code Mode runner   | [mcpx](planning/roadmaps/mcpx.md)                         | The only copy of the code; how downstream flakes consume it, and darwin takeover tests                                                                    |
| The NixOS flake and infra manifests            | [nixos-and-infra](planning/roadmaps/nixos-and-infra.md)   | Inputs kept current by a daily job. No open work items of its own                                                                                         |
| Repo-wide CI and automation                    | [repo-and-ci](planning/roadmaps/repo-and-ci.md)           | One Jules workflow, the Go build cache, stale PR triage — and the queue fleet program: lanes for every queue, OpenChamber session dispatch (#1233, #1234) |
| The basement home cluster (Talos, Cozystack)   | [basement-cluster](planning/roadmaps/basement-cluster.md) | Nothing built yet; research and planning done, hardware inventory and power come first                                                                    |

## Now, across the repo

1. Make CI trustworthy for `id`: the LogSync bulk test (#241).
2. Settle how other flakes consume `mcpx` from this repo, its single source (#308), and move the backlog here (#309).
3. Quiet the automation: stale PRs (#311).

## Next

1. `id` Phase 11 tasks that make the code easier to change: split `world.rs`, injected clock, typed errors, journal format tooling.
2. `id` sync protocol hardening from the Phase 3 to 5 issues (#178 to #195).
3. `mcpx` robustness: a supervisor for stdio children, session resume after a cold restart, JSON Schema validation.

## Later

1. `id` Phase 12: worlds as data, a provider registry, more guest languages, program upgrades, fair scheduling, delta frames.
2. `mcpx` policy and auth: write policy, argument rules, principals, bearer token.

## Themes that cross projects

| Theme                                      | Where it shows up                                                                                                               |
| ------------------------------------------ | ------------------------------------------------------------------------------------------------------------------------------- |
| A CI result people believe                 | `id` flakes and out-of-disk failures, Jules runs that skip, concurrency groups that drop runs                                   |
| One source of truth per component          | `mcpx` lives only here; `dezren39/nix` builds it from a pinned input, and the OpenCode and OpenChamber forks consume the result |
| Handing a running service to its successor | `mcpx` daemon takeover, `id` takeover of idle connections, the OpenCode `serve` handover                                        |

## Related repositories

| Repository                     | Relation                                                                                                                                 |
| ------------------------------ | ---------------------------------------------------------------------------------------------------------------------------------------- |
| `dezren39/nix`                 | The macOS flake. Builds `mcpx` from here via a pinned source-only input; its vendored copy was deleted on 2026-10-19 (dezren39/nix#468). |
| `developing-today/opencode`    | OpenCode fork. Calls the `mcpx` CLI and daemon.                                                                                          |
| `developing-today/openchamber` | OpenChamber fork, built against the fork of OpenCode.                                                                                    |

## Done

Completed work is recorded in GitHub — see closed issues and merged PRs.
