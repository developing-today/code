# Plan outline: sections, steps, edges, waves, tiers, deferred decisions

Read in steps 4 to 10. The [plan template](../../../../planning/templates/plan.md) owns the base sections. [ordering.md](/planning/workflow/ordering.md#plans) owns the step fields and what they mean, and [DEPENDENCIES.md](../../../../planning/DEPENDENCIES.md) owns the relation kinds and tiers. This file says how to fill them in for one sub-feature of a settled design.

## 1. File, header, goal and scope (step 4)

- **Path.** `planning/plans/<slug>.md`, where `<slug>` is the sub-feature's short name, or `<UTC>_<slug>.md` when order matters ([plans README](../../../../planning/plans/README.md)). One plan per plan issue.
- **Header.** The template's four lines, plus two:
  - `Status: draft`. The S8 orchestrator moves it to `in-progress` and `done`.
  - `Roadmap item:` the row in `planning/roadmaps/<area>.md`, or `none: <why>`.
  - `Design:` the consolidated design record, then the decision ids the plan builds on.
  - `Issues:` the plan issue now; the orchestration issue and the implement issues by step once they exist.
  - `Package: <PKG> · <ITEM>` and `Planned against: origin/<trunk> at <sha>`.
- **Goal.** One paragraph: what is true when the plan is done that is not true now. Start from the sub-feature's goal and "enables" in the consolidated record's "Sub-features" section.
- **Scope.** `In:` what the plan changes. `Out:` what it leaves alone, each item with where it is tracked: another plan issue, a contingent item, a roadmap row or an issue. An `Out` item with no link is a dropped requirement.

## 2. Section order

The template lacks three sections. The full order is: header, Goal, Scope, **Interfaces**, Steps (one `### Wave <n>` sub-section per wave, then the critical path), **Deferred decisions**, Tests and evidence, Rollout and rollback, Risks, Progress. S8 adds `## Retro` at the end; never write it yourself.

## 3. Interfaces (step 5)

An interface is anything two or more steps must agree on: a type, a schema, a file format or path, a CLI flag or subcommand, a config key, an error string a test matches, a module boundary. List each one:

```markdown
## Interfaces

- **Lock directory layout** `agent-work/<package>-<stage>/locks/<resource>.lock/owner`. Owner: step 1. Contract test: `test_lock_layout`. Used by: 3, 4, 6.
- **Branch name** `research/<slug>/s8-<item>-step-<n>`. Owner: plan (no code). Used by: 5, 7.
```

- One owner each: a step, or `plan` when the text above is the whole contract.
- Every step that uses an interface owned by a step `Requires:` that step.
- No other step defines or changes it. A worker who finds the interface wrong stops and reports; the S8 orchestrator adds a step.
- Shared names the design already pinned in its "Shared names" section are copied with a link, never renamed.

## 4. The step format (step 6)

Use the [ordering.md](/planning/workflow/ordering.md#plans) format with two more fields, `Test first:` and `Size:`:

```markdown
### Wave 2

3. **[P] Lease list command.** Add `lease list`, which prints every live lease of a package and skips expired ones.
   - Tier: crucial. Goal: an orchestrator sees every live lease with one command. Enables: 5, 6, C004.
   - Requires: 1 (the lock layout). After: 2, rework if early: the owner line may gain a field.
   - Files: `scripts/research_workflow_leases.py`, `scripts/tests/test_leases.py`. Cites: D003, C001.4.2.
   - Test first: `test_lease_list_skips_expired` fails at `<sha>` with `invalid choice: 'lease'`.
   - Done when: `python3 -m pytest scripts/tests/test_leases.py -q` passes.
   - Size: S.
```

- **Numbering.** One sequence across the whole plan, in dependency order, so a step requires only lower numbers. Once issues exist, numbers never change; a step added later takes the next free number. Implement issues link the step's wave heading (`#wave-2`) and name the number.
- **Granular.** One PR, one fresh session, one concern, a diff a reviewer reads in one sitting. `Size:` is `S` (under about 300 changed lines) or `M` (under about 800). Split anything bigger, and anything that mixes a refactor with a behaviour change.
- **Goal** is a state, not an activity: "the web view shows what the terminal shows", not "work on the presenter".
- **Enables** names later step numbers and the concept or item ids the step unlocks. A step that enables nothing and is not a deliverable of the goal is a candidate to cut.
- **Files** lists every path the step writes, tests included, as exact paths (`new` is fine). A directory only when the step creates it.
- **Cites** the decisions and items the step implements. Together the steps cite every `stated` and `human-confirmed` decision and every accepted item of `$ITEM` ([decisions.md](/planning/workflow/decisions.md#coverage)).
- **Test first** names the test the worker writes before the change and how it fails at the recorded SHA. A step with no runtime behaviour (docs, config) names a command that fails first instead, such as `rw check` on a missing link or `grep -q`. Never `n/a`.
- **Done when** is one command or CI job. Never "reviewed" or "looks right".

## 5. Edges (step 7)

| Edge                        | Use it when                                                                                                     | Write it as                                    |
| --------------------------- | --------------------------------------------------------------------------------------------------------------- | ---------------------------------------------- |
| `Requires:` (hard)          | The target's output is a real input: an interface, a generated file, a migration, behaviour the test exercises. | `Requires: 1 (the lock layout)`                |
| `After:` (soft)             | Starting early is possible but may cost rework.                                                                 | `After: 2, rework if early: <what is redone>`  |
| File collision              | Two steps must write the same file and the file cannot be reassigned to one of them.                            | `Requires: 4 (shares scripts/x.py)`            |
| `conflicts-with` (not here) | Two steps implement incompatible choices.                                                                       | A design gap: section 9, never a plan edge     |
| `optional-if` (contingent)  | The step runs only if something happens.                                                                        | `Tier: contingent. Trigger: <observable fact>` |

- Prefer the arrangement with the fewest hard edges. For each `Requires:`, ask whether the step could build against a pinned interface instead; if so, drop the edge or make it `After:`.
- Resolve a file collision by reassigning first: move the shared change into the step that owns the file, or into an interface step both require. Only then serialize with a hard edge, whose reason names the file. The S8 orchestrator relies on this: a collision is always a `blocked-by` link, never a wave convention.
- A cycle is rejected. Break it by extracting the shared part into an interface step.

## 6. Waves, `[P]` and the critical path (step 7)

- A step's wave is one more than the highest wave of its `Requires:` targets; an `After:` source goes one wave past its target ([ordering.md](/planning/workflow/ordering.md#waves), rule 3). Steps with no edges are in wave 1, after any interface steps they require.
- Mark `[P]` on a step when another step in its wave exists and the two share no file and no edge. A step that must run alone (a migration, a repo-wide rename, a lockfile or integration step) has no `[P]` and gets its own wave.
- Under the last wave, write `Critical path: 1 → 3 → 6 → 9 (4 sessions)` (the longest `Requires:` chain), `Peak parallelism: 4 of max_parallel_writers 5`, and `Estimate: <n> sessions over <w> waves`. A wave wider than the cap runs in batches, crucial first.

## 7. Tiers and value order (step 8)

- `crucial`, `recommended` and `contingent` mean what [DEPENDENCIES.md](/planning/DEPENDENCIES.md#2-priority-tiers) says, measured against this plan's goal. Start from the sub-feature's tier in the consolidated record.
- No crucial step requires a non-crucial one. Raise the blocker's tier, or drop the edge by pinning an interface.
- A contingent step names its trigger as an observable fact and who checks it, for example `Trigger: the benchmark in step 7 exceeds 200 ms (S8 orchestrator, after step 7 merges)`. It gets no issue until the trigger holds.
- Within a wave: crucial steps first, then the step that enables the most later steps, then the smaller one.

## 8. Tests and evidence (step 9)

Fill the template's section with what the steps do not already say: the integration or end-to-end checks, which step runs them (usually the last wave, without `[P]`), what each proves, and the profile's `validation_commands`. Name every test that must fail before the change.

## 9. Deferred decisions (step 10)

```markdown
## Deferred decisions

- **Q1. Owner line: one line or JSON?** Default: one line, as in collaboration.md. Settled by: step 1's worker, from what the parser needs. Before: step 3 starts. If it differs: steps 3 and 5 change their parsers; their briefs state the default.

Human-review defaults this plan builds on:

- **D007** (default: leases in the git common directory). Steps 1, 3 and 6 redo work if a human reverses it.
```

- Defer only what depends on implementation details: a format, a library, a threshold, a name a worker will see first. Each entry has all five parts: question, default, the step whose worker settles it, the step it must be settled before, and what changes if the answer differs.
- A product or architecture question, or a `conflicts-with` the design left open, is not deferred here. Comment it on the design orchestration issue with the default you plan on, and keep planning.
- You never write decision records. The S8 orchestrator records each settled answer as a dated line under its entry.

## 10. Rollout, rollback, risks and progress (step 11)

- **Rollout and rollback.** How it ships (default on or behind a flag), what a user or operator must do, and how to undo it: revert the merge commits in reverse wave order, and how any migrated data goes back.
- **Risks.** Each with its likelihood, its effect, the mitigation and the step that owns the mitigation.
- **Progress.** One dated line per event, starting with `- <UTC>: plan drafted against <sha>, PR #<n>`. S8 keeps it current.
