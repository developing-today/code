<!-- Copyright developing.today LLC & contributors. SPDX-License-Identifier: MIT OR Apache-2.0 -->

# From rough requests to researched designs and parallel delivery

- Status: draft, implementing the authorized documentation framework.
- Date: 2026-10-19T06:09:02Z.
- Roadmap: [repository planning](../../planning/roadmaps/repo-and-ci.md).
- Source: [the exact user request in Cairn](https://github.com/dezren39/cairn/blob/28c91a61309a5c6fd143fbc6da13ad889aaa2821/other/2026-10-19_research-design-workflow_request.txt). Cairn access is required; the private source is not copied into this public repository.
- Changes: [append-only work record](changes.md).
- Coverage: [request-to-artifact map](request-coverage.md).

## Intent

A sentence, a long document, or a noisy transcript can start a body of research without losing the original wording or prematurely narrowing the ideas. A future agent can pick up one topic, or the whole collection, from a durable reference or an issue URL. Research, design, planning, and implementation have separate completion evidence.

This change prepares that workflow, its skills, its documentation homes, and checks. It does not start research into hypothetical product features or create their issues. The owner has authorized this framework and its review through PRs.

## Documentation homes

Use the existing [three planning levels](../../planning/README.md):

| Home                                                                  | Purpose                                                                  |
| --------------------------------------------------------------------- | ------------------------------------------------------------------------ |
| Root README, AGENTS, ROADMAP                                          | Entry points, agent routing, and short portfolio priorities              |
| [Planning architecture](../../planning/DOCUMENTATION_ARCHITECTURE.md) | Placement defaults and reasons to make an exception                      |
| `doc/<UTC>_research_<slug>/`                                          | A dated request collection with raw transcript and owned concept records |
| `doc/<UTC>_<kind>_<slug>/`                                            | Other historical design and architecture records, including this one     |
| [New implementation plans](../../planning/plans/README.md)            | Granular steps, tests, dependencies, and implementation handoffs         |
| Component README/architecture/docs                                    | Living product and development references owned by that component        |
| [Portable workflow library](../../planning/workflow/README.md)        | Stage contracts, record formats, collaboration, and templates            |
| [Repository profile](../../.agents/research-workflow.json)            | Local paths and issue destination used by portable skills and tools      |

Old records and `.opencode/plans/` remain where existing links expect them. Existing historical prose stays intact. The documentation protocol now distinguishes historical evidence from maintained control documents rather than treating every Markdown file as immutable.

## Intake and the reference corpus

One intake agent saves the original UTF-8 bytes in `transcript.txt`, records its SHA-256, and maps every nonblank source line to a concept or an explicit unresolved fragment. Typos, repetitions, TTS ambiguity, unusual symbols, hinted capabilities, and concerns all survive. Clean interpretation is separate from exact excerpts. Suggested additions carry their source as inference or a research question.

For several concepts, the agent creates:

- `main.md`, the dated main reference, with introduction, owned file inventory, per-concept sections, preliminary thoughts, current state, stage-result placeholders, and inline issue/PR links plus a bottom index.
- `design.md`, the initial review order, typed dependencies, and parallel groups. It is an ordering proposal at intake, not an accepted product design.
- `changes.md`, a material change and disposition history.
- `concepts/C001-<slug>.md` and siblings, containing relevant verbatim excerpts near the top, purpose, framing, initial thoughts, and links back to the exact source and reference section.

A single concept uses `main.md` as its concept record, alongside the transcript. It has no artificial collection hierarchy. Directed notes, evidence, issue bodies, feedback, and other meta files appear only when there is actual content to put in them.

The main reference owns structured state in a delimited JSON block. This keeps integrity and scheduling data in the reference itself instead of creating a second competing manifest. The scaffold tool is the canonical creator of that block.

## Skills and stage boundaries

| Skill                                                                                  | Result and boundary                                                                                                 |
| -------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------- |
| [research-workflow](../../.agents/skills/research-workflow/SKILL.md)                   | Discover the current phase, reverify artifacts and external state, and dispatch the requested next work             |
| [request-intake](../../.agents/skills/request-intake/SKILL.md)                         | Exact input, coverage, reference and minimal owned stubs; stop at intake when that is the requested scope           |
| [research-prepare](../../.agents/skills/research-prepare/SKILL.md)                     | A first repository/code/doc map, relevant questions, and a researcher-ready brief per concept                       |
| [concept-research](../../.agents/skills/concept-research/SKILL.md)                     | Deep evidence, alternatives, feasibility, use cases, anecdotes identified as such, risks, and design findings       |
| [notes-reconcile](../../.agents/skills/notes-reconcile/SKILL.md)                       | Every directed note incorporated or explicitly acknowledged, deferred, or declined with its reason                  |
| [issue-handoff](../../.agents/skills/issue-handoff/SKILL.md)                           | A self-contained, idempotent parent, fanout, or orchestration issue for a specified next stage                      |
| [design-orchestrate](../../.agents/skills/design-orchestrate/SKILL.md)                 | Execute research/design issues through workers, reconcile, independently review/critique, and consolidate decisions |
| [feature-plan](../../.agents/skills/feature-plan/SKILL.md)                             | A feature plan and implementation issue graph with contracts, granular steps, tests, blockers, and rollout          |
| [implementation-orchestrate](../../.agents/skills/implementation-orchestrate/SKILL.md) | Execute ready implementation issues through workers and verified merge-commit PRs, then hand off the result         |

Handoffs identify the exact next stage. Issue planning does not recursively create copies of itself. Every issue includes source links, baseline, relevant excerpts or a self-contained summary, owned outputs, acceptance evidence, prerequisites, known uncertainties, and what the next agent should do first. A fresh reader tests the handoff before it is considered ready.

## Dependencies and parallel work

The [dependency vocabulary](../../planning/DEPENDENCIES.md) separates hard `requires` edges from `recommended-after` rework risks, mutually exclusive choices, shared-file conflicts, enabling outcomes, and optional future paths. Priority, criticality, horizon, readiness, and final value are separate fields.

The orchestrator dispatches the ready frontier. It does not impose a phase-wide wait when independent topics can continue. A preferred order must explain the cost of going earlier without becoming a hard blocker. The portfolio view shows the end goal, each feature's contribution, and which paths it enables.

Writer agents get their own worktree and branch from the declared integration baseline. They publish the branch before editing, then open a PR after the first meaningful commit. Read-only work and explicitly disjoint file writers can share a checkout when the exception and ownership are recorded. Ports, data stores, caches, and shared contracts are also resources to partition.

Updates and integrations use merge commits. Workers merge base updates into their branches, preserve existing work, and resolve conflicts before merge. Integrations are serialized; a conflicted branch does not stop unrelated workers. Reviews check current changes and required repository gates without making preexisting unrelated failures part of the task.

The orchestrator coordinates, monitors publication and actual PR merge state, retries or restarts incomplete workers, and delegates implementation. Conflicting designs trigger fresh reviewer, critic, or investigator prompts. Reversible defaults keep independent work moving; actual unresolved decisions with consequences are collected for human review rather than disguised as approval.

## History and reconciliation

Current status appears before the time, originating stage, explanation, evidence, and links to clarifying or superseding records. A later conclusion can be clarified, superseded, retracted, declined, deferred, or implemented without erasing the earlier finding.

Historical input and findings remain preserved. A maintained main reference or review-order document changes through a PR with an accompanying event. A worker edits its owned concept, or writes an actual directed note such as `notes/C001__to__C002__<id>.md`. It does not silently edit another worker's file. Reconciliation records a disposition and target for each note and scans for late arrivals before completion.

## Tooling and reuse

[research_workflow.py](../../scripts/research_workflow.py) scaffolds collections from an explicit outline and checks actual package integrity, source coverage, links, dependency graphs, ownership and completion claims. It provides current-state and ready-work views. The checks are tested against both valid inputs and intentional broken artifacts.

Cairn owns the reusable library. [sync_research_workflow.py](../../scripts/sync_research_workflow.py) copies reviewed generic files into this repository with source-commit and file-digest provenance, preserves the repository adapter, and refuses to overwrite local divergences. This avoids hidden dependencies on a personal home directory or an accessible private repository during future runs.

## Alternatives and decisions

1. Keep `doc/` and `planning/`, rather than migrating established paths. Reason: current records and issues already link to them.
2. Keep typed scheduling data in the main reference. Reason: source integrity and declared ownership should be checkable without a speculative metadata tree.
3. Create directed notes lazily. Reason: an all-pairs set would grow quadratically and most files would have no content.
4. Use portable skills with repository profiles. Reason: Cairn's journal remains distinct, while the same stage contracts can work inside this mixed Nix/Rust/Go repository.
5. Require a fresh independent review with evidence, without a fixed count of unchanged review rounds. Reason: review should resolve a finding or verify a changed result.
6. Use merge commits, as requested. Existing legacy skills that prescribe rebase or squash are not the authority for these lifecycle runs.

These are changeable defaults. A justified exception names the reason, effect, and replacement convention in the same PR.

## Verification and limitations

The complete raw message in Cairn was compared with the supplied body. It contains 16,594 UTF-8 bytes and its SHA-256 is `5aa4c1a83bcdff626d02568d2e3f0e4addeda940cd12594de399fedcc8453d2f`.

Validation covers fixture-based intake, integrity failures, stage routing contracts, references, typed scheduling and conflict handling. Trigger examples are a static evaluation corpus; they do not establish that every supported model/harness classified them correctly. A live multi-issue product implementation has not been executed as part of preparing this framework.

## Sources and related work

- The [exact request](https://github.com/dezren39/cairn/blob/28c91a61309a5c6fd143fbc6da13ad889aaa2821/other/2026-10-19_research-design-workflow_request.txt) and Cairn baseline `cabfbc2d29abb715be911410018aaf3231a28a60`.
- Cairn's [feature-stage specification](https://github.com/dezren39/cairn/blob/cabfbc2d29abb715be911410018aaf3231a28a60/prompts/goals/feature/SPEC.md).
- Cairn's [review of design packages](https://github.com/dezren39/cairn/blob/cabfbc2d29abb715be911410018aaf3231a28a60/2026-10-19_0013_design-spec-and-recent-designs-review.md). This is historical research consulted here, not a claim that its external sources were rechecked in this task.
- Repository baseline `7b081c30b40a85dc19cba255ba6ee9525a6bfdb6`, [documentation protocol](../DOCUMENTATION_PROTOCOL.md), and [planning](../../planning/README.md).

## Issues and PRs

Framework PRs and validation evidence are recorded in [changes.md](changes.md). No future product research issues have been created by this setup.
