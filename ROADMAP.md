# Roadmap

This is the overall roadmap for the repository. The repo holds several projects that move on their own schedules, so each has a roadmap file in [`planning/roadmaps/`](planning/roadmaps/). This page shows where they stand next to each other. How the roadmap works, and how it connects to design records and plans, is in [`planning/README.md`](planning/README.md).

A row here records intent. It does not settle a product question and it is not a go-ahead to build.

## Projects

| Project                                                           | Roadmap                                                   | Current focus                                                                                                                                                                                             |
| ----------------------------------------------------------------- | --------------------------------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `id`, the peer-to-peer CLI and worlds platform                    | [id](planning/roadmaps/id.md)                             | Close Phase 12 (four items open) and tag the first release; one end-to-end slice of the #9106 substrate. See the [results-first plan](planning/plans/2026-11-21_results-first.md)                         |
| `mcpx`, the MCP gateway and Code Mode runner                      | [mcpx](planning/roadmaps/mcpx.md)                         | The only copy of the code; how downstream flakes consume it, and darwin takeover tests                                                                                                                    |
| The NixOS flake and infra manifests                               | [nixos-and-infra](planning/roadmaps/nixos-and-infra.md)   | Inputs kept current by a daily job. No open work items of its own                                                                                                                                         |
| Repo-wide CI and automation                                       | [repo-and-ci](planning/roadmaps/repo-and-ci.md)           | The dispatch workflows, the Go build cache, stale PR triage — and the queue fleet program: lanes for every queue, OpenChamber session dispatch (#1233, #1234), pool & routing intelligence (#1343, #1340) |
| The basement home cluster (Talos, Cozystack)                      | [basement-cluster](planning/roadmaps/basement-cluster.md) | Nothing built yet; research and planning done, hardware inventory and power come first                                                                                                                    |
| The PM stack (Kaneo, Pad, Grist, the aggregator, the queue fleet) | [pm-stack](planning/roadmaps/pm-stack.md)                 | Coherence epic #1184 in progress: one URL, fresh data, triage-able issues across the four repos                                                                                                           |
| The fleet: the agents, their tool, queue and pools                | [fleet](planning/roadmaps/fleet.md)                       | The fleet tool as the one door for coordination, mirroring and backups (#8990): the verb contract v2, the local replica and backups are in review                                                         |
| Atlas, the search and memory index                                | [atlas](planning/roadmaps/atlas.md)                       | One index over every corpus (#8983): the backup mesh (#9093), the shard consumer's serve mode and the evaluation sets                                                                                     |

## Now, across the repo

The order, the evidence behind it and the progress log are in the [results-first plan](planning/plans/2026-11-21_results-first.md) (proposed 2026-11-21).

1. Test `id` before it merges, and give the queue a way to build a slice of a feature instead of re-triaging it (#1233, #1187).
2. `id`: close Phase 12 against its exit table and tag the first release — many worlds per node (#557), fair scheduling (#1067), delta frames (#805), first providers (#1062), verification debt (#796).
3. `id`: one end-to-end slice of the signed-record substrate, two nodes and a CLI (#9106, #9112).
4. `mcpx` robustness: a supervisor for stdio children (#1384), JSON Schema validation.

## Next

1. `id` hosted worlds v1 and discovery (#6276), then the agents-in-worlds spike (#6300).
2. `id` Phase 13 shells: presentation tiers (#786), kitty keyboard (#787), native shells (#788).

## Later

1. `id` product lines held behind a named trigger — currency, jurisdictions, boards and notes, messaging, the serverless edition, importers. The triggers are in the results-first plan's Later table; nothing is dropped.
2. `mcpx` policy and auth: write policy, argument rules, principals, bearer token.

## Themes that cross projects

| Theme                                      | Where it shows up                                                                                                                      |
| ------------------------------------------ | -------------------------------------------------------------------------------------------------------------------------------------- |
| A CI result people believe                 | `id` flakes and out-of-disk failures, dispatch runs that skip, concurrency groups that drop runs                                       |
| One source of truth per component          | `mcpx` lives only here; `dezren39/darwin-nix` builds it from a pinned input, and the OpenCode and OpenChamber forks consume the result |
| Handing a running service to its successor | `mcpx` daemon takeover, `id` takeover of idle connections, the OpenCode `serve` handover                                               |

## Related repositories

| Repository                           | Relation                                                                                                                                        |
| ------------------------------------ | ----------------------------------------------------------------------------------------------------------------------------------------------- |
| `dezren39/darwin-nix`                | The macOS flake. Builds `mcpx` from here via a pinned source-only input; its vendored copy was deleted on 2026-10-19 (dezren39/darwin-nix#468). |
| `developing-today-forks/opencode`    | OpenCode fork. Calls the `mcpx` CLI and daemon.                                                                                                 |
| `developing-today-forks/openchamber` | OpenChamber fork, built against the fork of OpenCode.                                                                                           |

## Done

Completed work is recorded in GitHub — see closed issues and merged PRs.
