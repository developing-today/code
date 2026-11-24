# The roadmap hierarchy: initiatives, projects, epics, aggregators

## The four types as the product conceives them

The programme frames the roadmap as four types: major initiatives, projects, epics, and aggregators. That frame is the product model. It names the grains Drew plans at: a major initiative sets direction, a project carries a tracked body of work under it, an epic groups related items inside a project, and an aggregator rolls the levels below it into one view.

## What the code implements

The code implements a different ladder. This chapter follows the code where the two differ (D10).

- **Kind is `Task | Epic` only.** The GitHub Project field `Kind` on Project #4 takes exactly two values, `Task` and `Epic` (`aggregator/src/model.ts`). No `Initiative` type and no `Project` type exist in that field. The word "initiative" as a code-level type is UNVERIFIED: no file in the survey names an `Initiative` type in `aggregator/src/model.ts`.
- **The initiative and project level rides on programs.** Programs P1–P9 are a document scheme defined in `pm-stack/design-reprioritization-2026-11-12.md`: P1 ID, P2 Atlas (rolling), P3 Fleet, P4 SaaS for ID/Atlas/fleet, P5 PM stack, P6 Turso+Cloudflare, P7 GitHub+dynamic compute, P8 Storage, P9 Lab. Each program carries a queue band. In live data a program surfaces as a project in the aggregator's project registry and on the `/program/\<id\>` surface (`aggregator/src/program.ts`, `project-config.ts`, `projects.ts`), and as an `Area` value in the GitHub Project.
- **Projects exist twice.** The portfolio project is one GitHub Project: org Project "Developing Today" (#4). Its items carry single-select fields Status, Area, Horizon, and Kind, read verbatim by the aggregator (`aggregator/src/projects.ts`). The registry project is a second notion: a tracked programme with its own status and roadmap, derived from the registry, its tracked issues, and the Project pool. The `/program` page is the hub for the second notion; Project #4 is the record for the first.
- **Epics are `Kind = Epic` items.** Project `Kind=Epic` is canonical. The aggregator reads the field and resolves `epicRef` through native sub-issue parents; it does not infer epic membership from labels. The `epic` label is a maintained mirror: every pool item with `Kind=Epic` carries it, and no other pool item does. The `meta/epic` label is retired and has no consumer.
- **"Aggregator" names a service, not a node type.** The aggregator is the dashboard service. Its own program page states the role: "This dashboard is the aggregator of the PM stack: the one screen Drew lives in while the fleet works in GitHub, Kaneo, and Pad underneath" (`aggregator/src/program.ts`). No roll-up node type named aggregator exists in the PM schema (D10). The staging aggregator in Atlas (`pkgs/atlas-search/crates/atlas-core/src/aggregate.rs`) is a separate component and takes no part in this hierarchy.

The reconciled ladder, top to bottom: program (document scheme, surfaced as registry project and as Project `Area`) → project (registry entry plus Project #4) → epic (`Kind=Epic` item plus sub-issue parent) → task or item (`Kind=Task` pool item, GitHub issue or PR, Kaneo card, Grist or repo roadmap row).

## How items join and roll up

Items join by refs, not by a shared table. A ref names its source: `gh:…`, `kaneo:\<slug\>/\<id\>`, `grist:\<doc\>/Roadmap/\<id\>`, `session:…`, `orch:…`. A bare issue or PR reference takes the form `owner/repo#N`.

Two edge kinds stay distinct. Hierarchy is the native GitHub sub-issue parent relation; it carries epic membership. Dependencies are the native `blockedBy` and `blocking` relations (`aggregator/src/orchestration-dependency.ts`, `epic-graph.ts`, `graph.ts`). Prose substitutes for neither edge kind.

Epic rollup groups children by `epicRef` and counts done against total (`aggregator/src/epics.ts`). A child counts as done by its state and status. Progress is weight, not counts (`aggregator/src/weights.ts`): `effort/xs` = 1, `effort/s` = 4, `effort/m` = 12, `effort/l` = 40, and `effort/l` is a floor flagged open-ended. An unsized item contributes no weight and is counted visibly as unsized; it is never zero-weighted. Summaries also split open weight by `mode/*`: physical and hybrid count as hands weight, alongside online and unmoded.

An epic closure gate guards the top of the rollup (`epics.ts` `evaluateEpicClosureGate`, `scripts/epic-closure-gate.ts`): an epic does not close while its complete subtree has open children or unmet conditions, and the gate enumerates blockers by condition. `fleet-monitor/epic_close_check.py` runs the gate report-only. Reparenting moves sub-issue parents (`aggregator/src/epic-reparent.ts`, `scripts/epic-reparent.ts`); `scripts/check-epic-conformance.ts` checks conformance.

## The roadmap surfaces

The `/roadmap` page and its twin `/api/roadmap.json` merge rows from three sources: the Grist `Roadmap` table (`sources/live.ts` `gristRoadmap()`), repo files (`planning/roadmaps/*.md` in developing-today/code and `ROADMAP.md` in dezren39/darwin-nix, read by `sources/repo-roadmap.ts`), and Project-pool and program data. The merge rule is fixed: a title present in both a repo file and Grist collapses to the repo-file row, the Grist duplicate is noted in provenance, and its links are unioned in. Rows carry Title, Status, Horizon, Area, Intent-or-Notes, and Links. One filter function serves the page and its twin (`filterRoadmapForView`).

The `/program` page rolls up per project: distributions over Status, Area, Horizon, and Kind, epic rollups, tracked-issue state, and orchestration freshness. A Project Status that conflicts with derived state is surfaced, not hidden; `/conflicts` lists every live source disagreement.

## Two horizon vocabularies, two roadmaps

"Horizon" has three live meanings, and this book does not collapse them (D11):

- Repo-file and Grist roadmap rows use the buckets `Now | Next | Later | Ideas | Ongoing` (`sources/repo-roadmap.ts`, validated by `roadmap-horizon.ts`).
- Aggregator tier items use `Horizon = today | tomorrow | next-3-days | longer | incoming` (`aggregator/src/model.ts`).
- Project #4 has its own `Horizon` single-select. The aggregator reads its values verbatim from the Project; the Project defines them.

The console carries a second roadmap implementation (D12). `console/src/plan.ts` derives phases `doing | now | next | later | built` from labels, parses capabilities from a plan doc, and parses the plan doc's markdown table into roadmap rows. It is not a client of the aggregator's `/api/roadmap.json`. Order inside each surface is decided by the rules in [Prioritisation: how order is decided and recorded](prioritisation.md).

Sources. Survey `roadmap.md` §§1–2; survey `disagreements.md` D10, D11, D12; code at laptop clone `~/git/code` at `4d937b3` as cited inline.
