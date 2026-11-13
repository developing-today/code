# Roadmap: PM stack

The PM stack is how work in this repo and its related repositories is planned, picked up, and watched: Kaneo as the execution board, Pad as the planning layer, the Grist "Program roadmap" table as a mirror of these roadmap files, the aggregator in [`pm-stack/`](../../pm-stack/) as the cross-repo read view, and the queue fleet (lanes, orchestrator, stewardship) that picks work up. Design record: [`doc/2026-10-24T11-05-38Z_design_project-management-stack/`](../../doc/2026-10-24T11-05-38Z_design_project-management-stack/). How to read this file: [planning README](../README.md).

The roadmap of record is this file and its siblings in `planning/roadmaps/`. The Grist table is a mirror of them (kept in sync by the roadmap-sync job); a Grist-only row is a seed or a mirror entry, never a second source of truth.

## Now

| Title | Status | Intent | Links |
| ----- | ------ | ------ | ----- |
| PM stack coherence: single URL, fresh data, triage-able, hierarchical | in-progress | The coherence epic for the whole stack: one place to look, data that is current, issues a worker can pick up without archaeology. | #1184 |
| This roadmap and the Grist mirror reconciliation | in-progress | The PM stack listed as a roadmap project, and the Grist-only PM rows marked as mirror rows so no roadmap statement contradicts a GitHub issue state. | #1216 |
| Minimum viable label adoption for opencode, openchamber, nix | in-progress | A shared minimum label set so cross-repo views built on labels are not blind outside this repo. Measured 2026-11-13: unlabeled open issues at 1/70 (opencode), 0/82 (openchamber), 3/51 (nix); the residual labeling is queued. | #1223 |
| Kaneo execution board anchors: GitHub App installed and connected | shipped | Phase 1 of the design: Kaneo on the laptop, the developing.today GitHub App installed on this repo and on dezren39/nix, two-way sync verified. Closed as completed. | #1007, PR #752 |
| Work-picking jobs consult the orchestrator's published priorities | dropped | Closed as not planned. Recorded here so the anchor named by #1216 is represented accurately: this is not current work, and the queue fleet as built does the picking instead. | #911 |

## Next

| Title | Status | Intent | Links |
| ----- | ------ | ------ | ----- |
| Connect opencode, openchamber, nix as Kaneo projects | planned | One Kaneo project per repo (the upstream one-repo-per-project limit), the developing.today app installed per repo. Seeded as a Grist row on 2026-10-24; tracked under the coherence epic. | #1184 |
| Projector and link registry (Turso) | planned | A webhook-fed projector joining GitHub issue, Kaneo task, Pad item, and roadmap row by ID. Never a source of truth. | #1184 |
| Aggregator UI: cross-repo WIP board and roadmap view | in-progress | Read-mostly and disposable: if it rots, every layer underneath still works standalone. The aggregator service exists and reads this repo's roadmap data today; the cross-repo board is the remaining scope. | #1184 |
| Apply external Grist doc changes live | shipped | External changes to the Grist doc apply without a full server restart. Closed as completed. | #1288 |

## Later

| Title | Status | Intent | Links |
| ----- | ------ | ------ | ----- |
| Live two-way sync between Grist and the roadmap files | idea | The 4-hourly roadmap-sync job is the interim measure; a live sync replaces it when the central-schema work lands. | #1184 |
