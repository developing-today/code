# Workflow stages

What each stage takes in, produces and must prove before the next one starts. Stage names, skills and every status value are in [vocabulary.md](vocabulary.md); file formats in [records.md](records.md); the rules every writer follows in [collaboration.md](collaboration.md). Exit criteria name the command that checks them. A command not built yet is listed in [Tool status](records.md#tool-status); until it exists, the stage owner checks the criterion by hand and says so in the PR.

```text
S0 intake → S1 prepare → S2 research → S3 reconcile → S4 handoff → S5 fanout → S6 design → S7 plan → S8 implement
            └──────── one agent per concept, all concepts in parallel ────────┘   └ orchestrator + workers ┘
```

## When to use the workflow

Use it when a request covers several topics, is long or garbled, or will lead to many issues. A request that is one clear change goes straight to an issue and a PR. One design question gets a design record under [doc/DOCUMENTATION_PROTOCOL.md](../../doc/DOCUMENTATION_PROTOCOL.md). If a stage's input would not fit one fresh session, split it into more concepts or facets before starting ([collaboration.md](collaboration.md#the-rules), rule 8).

## Every stage

- The owner works in its own worktree and branch, pushes the empty branch first and opens a PR after its first commit ([collaboration.md](collaboration.md#the-rules)).
- In its first commit the owner sets the concept's `stage` and appends a history entry ([records.md](records.md#concept-file)).
- "Merged" in an exit criterion means the PR merged and its head SHA is an ancestor of the trunk.
- Each stage keeps everything earlier stages wrote. It refines items with `item set-status` and `item add`; it never deletes text.
- The worker ends with the turn report: branch, PR URL, last pushed SHA, what is verified, what is not.

## S0 intake

- Skill: `request-intake`. Parallelism: one agent. Read-only helpers may propose outline parts; only the intake agent writes.
- Input: the request bytes as received, with their source, an expected SHA-256 when one is known, and whether they may be public.
- Output: the package (`transcript.txt`, `package.json`, `main.md`, `design.md`, `changes.md`, one concept file per concept) with every concept at `S0`, status `created`, disposition `proposed`. `main.md` has the introduction, preliminary thoughts per concept, the garbled-input register and the views. Ideas the request only hints at become `hint` concepts or items labelled inference; nothing is dropped at this stage.
- Owner: the intake agent owns the whole package folder until its PR merges. Afterwards each concept file belongs to the owner of its next stage, and `main.md` and `design.md` are wide files.
- Exit criteria:
  - `check --package <dir>` passes: the transcript hash matches `package.json` (and `expected_sha256`), every segment is covered by a concept or `G`, every excerpt is exact, every value is in the vocabulary, and the views are current.
  - The intake PR is merged.
- Next: the router starts S1 for every concept `ready` returns. `hint` concepts wait until a note or a human answer changes their type.

## S1 prepare

- Skill: `research-prepare`. Parallelism: one agent per concept, all ready concepts at once up to the cap.
- Input: the concept file, its section in `main.md`, the transcript segments it cites, and the repo.
- Output, in the concept file only: "Current repository state" (the code, docs and designs the concept touches, as links), "Research questions" (numbered, each answerable), more excerpts if the concept needs other segments, and readings of garbled tokens it relies on. Outside the file: the issue `Research: <concept> (<slug>·C001)` with its marker, self-contained so a zero-context agent can run S2 from its URL. Changes it wants in another concept go out as notes.
- Exit criteria:
  - The two sections are filled; `check` finds no placeholder text in them.
  - The research issue exists, its marker is found by a search, and it is in the concept's `issues` with stage `S1`.
  - The PR is merged.
- Next: the merged PR and the open research issue. The router dispatches S2, or anyone can pick up the issue.

## S2 research

- Skill: `concept-research`. Parallelism: one agent per concept.
- Input: the research issue URL.
- Output, in the concept file: "Research" (the request restated, feasibility, options with a steelman each, prior art, example projects, user reports, risks with mitigations, an answer or an open reason for every research question), "Sources" (every link used, as returned by the tool that fetched it), and the concept's "Change log". Items are refined with `item add` and `item set-status` through a PR to `main.md`. Notes go to other concepts as soon as they are known, in note-only PRs.
- Exit criteria:
  - Every research question has an answer or an open reason; "Research" and "Sources" are not empty.
  - Every note this concept sent is on the trunk or in an open PR.
  - The PR is merged.
- Next: S3 for this concept. The same agent may run it in the same session when it fits.

## S3 reconcile

- Skill: `notes-reconcile`. Parallelism: one agent per concept.
- Input: every note addressed to the concept, on the trunk and on every `origin/*` ref (`check --remote-notes`).
- Output: one receipt per note, the concept file updated where notes were incorporated, and the "Notes" section listing each note and its disposition. The scanned refs and SHAs are recorded ([records.md](records.md#receipts)).
- Exit criteria:
  - `check --package <dir> --require-resolved` passes.
  - `check --remote-notes` reports nothing pending for this concept.
  - The PR is merged.
- Next: S4. A note that arrives later reopens S3 for its receiver; if S4 or later already ran, the reconcile owner updates the affected issues.

## S4 handoff

- Skill: `issue-handoff`, mode `parent`. Parallelism: one agent per concept.
- Input: the reconciled concept file.
- Output: the issue `Handoff: <concept> (<slug>·C001)`. It asks for the concept to be investigated and planned, in part or whole, lists the facets to research with a short description each, and links everything a zero-context agent needs. The concept's `issues` gains it.
- Exit criteria: the issue exists with its marker and every body section [records.md](records.md#issues) requires; the PR that records it is merged.
- Next: the open parent issue. An agent with no other context runs S5 from its URL.

## S5 fanout

- Skill: `issue-handoff`, mode `fanout`, then mode `orchestration`. Parallelism: one agent per parent issue.
- Input: the parent issue URL.
- Output: one `Design:` child issue per facet, each a full handoff with blocked-by and parallel-safe-with, and each facet recorded as an item (`C001.3`) so the issue can name it. Then the orchestration issue `Design: orchestrate <concept> (<slug>·C001)`, listing the children in priority order and in waves ([ordering.md](ordering.md)). The design's "initial research stages" in `design.md` list every issue for the concept with a short description and its order.
- Exit criteria: every facet in the parent issue links to its child issue; every child and the orchestration issue have markers; the concept's `issues` lists them with stage `S5`; the PR is merged.
- Next: the open orchestration issue starts S6.

## S6 design

- Skill: `design-orchestrate`. Parallelism: one orchestrator, plus one worker per child issue up to the cap, plus read-only reviewer, critic and investigator subagents.
- Input: the orchestration issue URL.
- Output: a design record folder `doc/<UTC>_design_<slug>/` per concept (each facet worker owns one file in it; one consolidation worker writes the main document); decision records in the package; `human-review.md`; the issue `Plan: <concept> (<slug>·C001)`. The concept's disposition becomes `accepted`, `deferred` or `rejected`.
- Owner: workers write; the orchestrator only merges, files the decision records the reviewers return, and keeps issues and state current ([collaboration.md](collaboration.md#the-rules), rule 7).
- Exit criteria:
  - Every child issue is closed by a merged PR.
  - Every `conflicts-with` pair in scope has a decision record, and the challenge pass has run on every decision ([decisions.md](decisions.md)).
  - `human-review.md` is current and the plan issue exists.
- Next: the open plan issue starts S7. Preliminary decisions do not block it; a human reversal later triggers scoped rework.

## S7 plan

- Skill: `feature-plan`. Parallelism: one agent per design.
- Input: the plan issue URL, the design record and its decisions.
- Output: `planning/plans/<slug>.md` from the [plan template](../templates/plan.md), with granular steps in waves ([ordering.md](ordering.md#plans)); one `Implement:` issue per step whose blocked-by matches the plan; the orchestration issue `Implement: orchestrate <concept> (<slug>·C001)`; a row in the plan catalog. Disposition becomes `planned`.
- Exit criteria:
  - Every step has an issue with a marker, and each issue's blocked-by equals the step's `requires`.
  - No two steps in one wave share a file; shared names and interfaces are pinned in the plan before wave 1.
  - Every `stated` or `human-confirmed` decision and every accepted item of the concept is cited by at least one step ([decisions.md](decisions.md#coverage)).
  - The PR is merged.
- Next: the open implementation orchestration issue starts S8.

## S8 implement

- Skill: `implementation-orchestrate`. Parallelism: one orchestrator plus one worker per ready step, up to the cap.
- Input: the orchestration issue URL and the plan.
- Output: merged PRs, the plan's progress kept current, the concept's disposition `implemented`, and its roadmap row moved to Done.
- Exit criteria:
  - Every step issue is closed by a merged PR.
  - The profile's `validation_commands` pass on the trunk after the last merge.
  - One fresh-context review ran after integration; its findings are fixed or filed as issues.
- Next: none. A package is finished when every concept is `implemented`, `deferred`, `rejected` or `superseded`.

## Router

- Skill: `research-workflow`. Parallelism: one.
- Input: a package path, or nothing (it finds packages under `requests_root`).
- Output: dispatch decisions only. It runs `status --live` and `ready`, starts the stage skill for each ready concept up to the cap, restarts stalled workers, and reports what is blocked and why. It writes no content.
- Exit criteria: every ready concept has a running worker, or nothing is ready and the report names each blocker.
- Next: run it again when a PR merges, a worker ends, or a heartbeat lapses.

## A single concept

The layout does not change: one concept file, the same views. The path is shorter in sessions, not in records:

- S1, S2 and S3 may run in one session by one agent. With no sibling concepts, S3 is the remote-notes scan, recorded in the S3 history entry.
- S4 and S5 may run in one session.
- `design.md` has one wave and needs no review order.
- S6 to S8 do not shrink: their size follows the number of facets and steps, not concepts.
