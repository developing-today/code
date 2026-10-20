# Workflow Lifecycle Stages

The workflow breaks down work into nine distinct lifecycle stages plus a top-level router. Each stage has defined entry criteria, parallel execution rules, and measurable deliverables.

```text
S0: Intake ──> S1: Prepare ──> S2: Research ──> S3: Reconcile
                                                     │
S7: Plan <── S6: Design <── S5: Fanout <── S4: Handoff
   │
   └──> S8: Implement
```

## Stage Catalog

| Stage      | Name          | Skill                          | Concurrency                | Deliverable                                                                                                                                                                                                      |
| ---------- | ------------- | ------------------------------ | -------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| **S0**     | **Intake**    | `request-intake`               | Single agent               | Exact sealed transcript (`transcript.txt`), manifest (`package.json`), main reference (`main.md`), initial review order (`design.md`), change log (`changes.md`), and concept stubs (`concepts/C001-<slug>.md`). |
| **S1**     | **Prepare**   | `research-prepare`             | 1 agent per concept        | Codebase, doc, and architecture mapping for the concept, research questions, and trigger issue drafting.                                                                                                         |
| **S2**     | **Research**  | `concept-research`             | 1 agent per concept        | Deep factual research, alternative steelmanning, use-case evaluation, risks with mitigations and fallbacks.                                                                                                      |
| **S3**     | **Reconcile** | `notes-reconcile`              | 1 agent per concept        | All incoming directed notes processed, receipts recorded, and concept documents synced.                                                                                                                          |
| **S4**     | **Handoff**   | `issue-handoff` (mode: parent) | 1 agent per concept        | Self-contained parent GitHub issue defining the scope, goals, and acceptance criteria for investigating and designing the concept.                                                                               |
| **S5**     | **Fanout**    | `issue-handoff` (mode: fanout) | 1 agent per parent issue   | Granular facet child issues, initial research stages section populated in `design.md`, and an orchestration issue for dispatch.                                                                                  |
| **S6**     | **Design**    | `design-orchestrate`           | 1 orchestrator + N workers | Detailed design records (`doc/<UTC>_design_<slug>/`), decision records (`decisions/D001-<slug>.md`), minimal human-review list, and plan issues.                                                                 |
| **S7**     | **Plan**      | `feature-plan`                 | 1 agent per design         | Implementation plan in `planning/plans/<slug>.md` with numbered steps, automated test criteria, stack map, and task issues.                                                                                      |
| **S8**     | **Implement** | `implementation-orchestrate`   | 1 orchestrator + N workers | Autonomous execution of ready issues across git worktrees, verified merge-commit PRs, final review, and handoff.                                                                                                 |
| **Router** | **Workflow**  | `research-workflow`            | Single agent               | Automatic phase discovery from machine state, readiness inspection, and worker dispatch.                                                                                                                         |

## Orchestrator vs. Worker Separation

Orchestrator agents (`research-workflow`, `design-orchestrate`, `implementation-orchestrate`) are strictly coordinators:

- They inspect git and issue state to compute the unblocked ready frontier.
- They boot worker subagents into dedicated worktrees.
- They monitor publication, verify reported SHAs, and execute merge commits.
- They never implement features, write code, or draft research prose directly.
