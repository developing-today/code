# Workflow vocabulary

Every enumerated value and identifier the workflow uses, defined once. Records that carry these values are described in [records.md](records.md). `check` rejects values outside these lists; see [Tool status](records.md#tool-status) for what it validates today.

Relation kinds and priority tiers are listed here so the full set is in one place, but their meaning and use are defined in [DEPENDENCIES.md](../DEPENDENCIES.md). Horizons are the repo roadmap horizons from [planning/README.md](../README.md#roadmap).

## Stages

| Stage | Name      | Skill                                             |
| ----- | --------- | ------------------------------------------------- |
| S0    | intake    | `request-intake`                                  |
| S1    | prepare   | `research-prepare`                                |
| S2    | research  | `concept-research`                                |
| S3    | reconcile | `notes-reconcile`                                 |
| S4    | handoff   | `issue-handoff` (mode `parent`)                   |
| S5    | fanout    | `issue-handoff` (modes `fanout`, `orchestration`) |
| S6    | design    | `design-orchestrate`                              |
| S7    | plan      | `feature-plan`                                    |
| S8    | implement | `implementation-orchestrate`                      |
| any   | router    | `research-workflow` (dispatches; owns no stage)   |

A concept's `stage` is the latest stage that has started on it. The worker of that stage sets it in its first commit; the stage is finished when its exit criteria in [stages.md](stages.md) are met.

## Statement status (`status`)

What we know about a statement: a concept or an item.

- `created`: recorded from the transcript or by a stage. Not yet checked.
- `clarified`: refined by a later stage without changing the intent. The detail usually sits in a child item directly below.
- `confirmed`: checked against code, a cited source or a human answer. The trail text says which.
- `superseded`: replaced by a newer item, concept or decision. The trail text names it.
- `retracted`: withdrawn because it was wrong, misheard or is no longer wanted.

`superseded` and `retracted` are terminal. The record keeps them, but they are excluded from default reading ([immutability](../DOCUMENTATION_ARCHITECTURE.md#4-immutability-vs-maintained-documents)).

## Disposition (`disposition`)

What happens to the thing a concept describes.

- `proposed`: recorded, not yet evaluated. Never read as in force.
- `accepted`: chosen for design and planning.
- `planned`: a feature plan with steps and issues exists.
- `in-progress`: at least one implementation PR is open or merged.
- `implemented`: every plan step has merged and been validated.
- `deferred`: postponed. Requires a reason and a revisit trigger.
- `rejected`: decided against. Requires a reason.
- `superseded`: replaced by another concept or package. Names it.

Roadmap rows in `planning/roadmaps/` use their own status list. They map as follows: `idea` and `proposed` to `proposed`, `shaped` to `accepted`, `planned` to `planned`, `in-progress` to `in-progress`, `shipped` to `implemented`, `dropped` to `rejected`. A `deferred` concept keeps its roadmap row under Later or Ideas.

## Run status

Where the work for one concept at one stage stands. `claimed`, `running`, `validated`, `blocked` and `abandoned` are recorded by the orchestrator or router ([records.md](records.md#tool-status) says where today); `claimed` and `running` move to the live directory once leases exist. `pushed`, `pr-open` and `merged` are never recorded: `status --live` derives them. When several apply, the first match in this list wins.

1. `abandoned`: the concept history records that this stage's work was stopped on purpose, with a reason. The branch stays.
2. `validated`: the PR merged and the stage's exit criteria were checked afterwards, recorded in history.
3. `merged`: the PR merged and its head SHA is an ancestor of the trunk (git and GitHub).
4. `pr-open`: an open PR exists for the stage branch (GitHub).
5. `pushed`: the stage branch on origin has commits that are not in the trunk, and there is no PR (git). Treat it as an orphan if it lasts.
6. `running`: a lease is held and the worker's log has a heartbeat younger than 20 minutes (live directory).
7. `claimed`: a lease is held and there is no heartbeat yet (live directory).
8. `blocked`: a `requires` target has not finished, or the history records a blocking reason.
9. `not-started`: none of the above.

A worker's own report never sets a run status. The live directory is described in [collaboration.md](collaboration.md#live-coordination).

## Note receipt disposition

How a receiver handled a note: `incorporated`, `acknowledged`, `declined`, `deferred`, `superseded`. Each needs a target and a reason ([records.md](records.md#receipts)).

## Decision source and status

- Source: `stated`, `inferred`, `agent-preliminary`, `human-confirmed`.
- Status: `preliminary`, `confirmed`, `reversed`.

Meanings and promotion rules: [decisions.md](decisions.md#source-and-status).

## Concept type and kind

- Type: `topic` (a subject to research), `hint` (an idea only hinted at; not ready for research until a note or a human answer changes its type), `constraint` (a limit the result must respect), `directive` (an instruction about the work itself).
- Kind: `new` (the concept creates files) or `edit` (it changes existing files). `targets` lists the paths either way.

## Relation kinds

`requires`, `recommended-after`, `shares-files-with`, `conflicts-with`, `enables`, `optional-if`, `supersedes`, `contradicts`, `part-of`. Defined in [DEPENDENCIES.md](../DEPENDENCIES.md#1-dependency-relations).

## Priority

`crucial`, `recommended`, `contingent`, `unspecified`. Defined in [DEPENDENCIES.md](../DEPENDENCIES.md#2-priority-tiers).

## Horizon

`now`, `next`, `later`, `ideas`, `unspecified`. These are the roadmap horizons in [planning/README.md](../README.md#roadmap). Horizon says when; priority says how much it matters.

## Identifiers

Identifiers come from the tool, never from an agent's guess. `init`, `item add` and `note` allocate them. For a concept or decision added later, take the next free number from `ids` while holding the package's `ids` lease ([collaboration.md](collaboration.md#live-coordination)).

- Package: the folder name `<UTC>_research_<slug>`, for example `2026-10-19T06-09-02Z_research_research-intake-to-implementation`. `<UTC>` is `YYYY-MM-DDTHH-MM-SSZ`; `<slug>` is lowercase kebab case.
- Segment: `T001`, `T002`, ... in transcript order. A range is `T007-T009`.
- Concept: `C001`, `C002`, ... Allocated by `init`.
- Item: `C001.2`, `C001.2.1`, ... Allocated by `item add`, never renumbered. Items in the global bucket are `G.1`, `G.1.2`, ...
- Global bucket: `G`. Holds the process directives and constraints the user gave about how to work.
- Note: the file stem `C001__to__C002__<uuid>`, where `<uuid>` is 32 lowercase hex digits. Allocated by `note`.
- Decision: `D001`, `D002`, ... Never reused, even after a reversal.
- Branch: `wf/<slug>-c002-s2` for concept C002 at stage S2 of package `<slug>`, and `wf/<slug>-c002.3-s6` for a facet. A follow-up after a merge adds `-2`, `-3`.
- Agent name: lowercase kebab case, unique within a run. Used in leases, logs and receipts.
- Timestamp: UTC to the second, `2026-10-21T10:00:00Z`. In file and folder names, colons become hyphens: `2026-10-21T10-00-00Z`.
