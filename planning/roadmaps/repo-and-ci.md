# Roadmap: repo-wide CI and automation

This file covers `.github/workflows`, the Jules automation and the way pull requests are triaged. Each project's own checks are in its roadmap. How to read this file: [planning README](../README.md).

## Now

| Title                                     | Status   | Intent                                                                                                                                                                  | Links               |
| ----------------------------------------- | -------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ------------------- |
| One Jules workflow                        | proposed | Three Jules workflow files become one.                                                                                                                                  | #287                |
| Cache the Go build and reuse artifacts    | proposed | Build cache by `go.sum` hash.                                                                                                                                           | #288                |
| Triage stale PRs                          | proposed | Overlapping Jules drafts, old Renovate security PRs and one PR with conflicts.                                                                                          | #311               |
| Queue fleet reliability                   | proposed | Lane-style shifts for every repo queue and shift completion certified from queue state, so the fleet runs a week without operator repairs.                              | #1233, #1130, #1131 |
| OpenChamber session dispatch + babysitter | proposed | Larger code tasks run as OpenChamber sessions on plan pools, dispatched with admission control and watched by one babysitter — the token-cheap executor for fleet work. | #1234, #1185, #1187 |

## Next

| Title                                         | Status      | Intent                                                                                                                                                               | Links                           |
| --------------------------------------------- | ----------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ------------------------------- |
| The `id` workflow gate stays under 15 minutes | planned     | The gate once took about two hours. It is fast now. Keep it that way and measure it.                                                                                 | #292, F11.4                     |
| Adopt the planning folder                     | in-progress | New plans and roadmap changes go through `planning/`.                                                                                                                | [planning README](../README.md) |
| OpenClaw babysitting-by-comment               | proposed    | The monitor's routine issue comment becomes the control channel: OpenClaw reports its session state and obeys instructions in-thread. Pilot first, then mirror mode. | #1234, #1190                    |

## Ideas

| Title                         | Status | Intent                                                              | Links |
| ----------------------------- | ------ | ------------------------------------------------------------------- | ----- |
| Jules does not repeat a theme | idea   | The Bolt and Palette jobs skip a theme that already has an open PR. | #310  |

## Done

Completed work is recorded in GitHub — see closed issues and merged PRs.
