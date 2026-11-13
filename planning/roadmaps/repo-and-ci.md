# Roadmap: repo-wide CI and automation

This file covers `.github/workflows`, the pm-dispatch automation (formerly Jules) and the way pull requests are triaged. Each project's own checks are in its roadmap. How to read this file: [planning README](../README.md).

## Now

| Title                                     | Status   | Intent                                                                                                                                                                  | Links               |
| ----------------------------------------- | -------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ------------------- |
| Queue fleet reliability                   | proposed | Lane-style shifts for every repo queue and shift completion certified from queue state, so the fleet runs a week without operator repairs.                              | #1233, #1130, #1131 |
| OpenChamber session dispatch + babysitter | proposed | Larger code tasks run as OpenChamber sessions on plan pools, dispatched with admission control and watched by one babysitter — the token-cheap executor for fleet work. | #1234, #1185, #1187 |
| Pool & routing intelligence               | proposed | Lanes everywhere plus session dispatch and the babysitter live now; the steward, meta-reviewer, and pool intelligence run oversight on top of them.                     | #1343, #1340        |

## Next

| Title                                         | Status      | Intent                                                                                                                                                               | Links                           |
| --------------------------------------------- | ----------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ------------------------------- |
| Adopt the planning folder                     | in-progress | New plans and roadmap changes go through `planning/`.                                                                                                                | [planning README](../README.md) |
| OpenClaw babysitting-by-comment               | proposed    | The monitor's routine issue comment becomes the control channel: OpenClaw reports its session state and obeys instructions in-thread. Pilot first, then mirror mode. | #1234, #1190                    |

## Ideas

| Title                         | Status | Intent                                                              | Links |
| ----------------------------- | ------ | ------------------------------------------------------------------- | ----- |
| Jules does not repeat a theme | idea   | The Bolt and Palette jobs skip a theme that already has an open PR. | #310  |

## Done

Completed work is recorded in GitHub — see closed issues and merged PRs.
