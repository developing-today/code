# Ordering

How relations between concepts and steps become waves of parallel work, and how a plan writes them down. Who may dispatch, and how many at once, is collaboration rule 8 ([collaboration.md](collaboration.md#the-rules)).

## Edges

Relation kinds and priority tiers are defined in [DEPENDENCIES.md](../DEPENDENCIES.md#1-dependency-relations). Scheduling uses them like this:

- `requires`: a hard blocker. The source is not ready until the target has merged, or reached the `stage` the relation names. Cycles are rejected.
- `recommended-after`: a soft edge. The source goes one wave after the target, but may start early when writers are idle. Its `rework_if_early` text says what may need redoing, and the worker's brief states the assumption it runs on.
- `shares-files-with`: never in the same wave.
- `conflicts-with`: settled by the decision protocol ([decisions.md](decisions.md#disagreement)) before either side is planned.
- `optional-if`: scheduled only once its `trigger` holds.
- The other kinds do not change the order.

## Waves

`ready` turns the edges into waves:

1. Drop units that are finished at their stage, and units that are claimed or running ([run status](vocabulary.md#run-status)).
2. Wave 1 is every remaining unit whose `requires` targets have merged on `origin/<trunk>`. Later waves follow `requires` in topological order.
3. Two units in one wave that share files are split; the later one moves to the next wave. A `recommended-after` source moves one wave past its target. The `waves` view shows each such cost under the wave that pays it ([records.md](records.md#generated-views)).
4. The router dispatches wave 1 up to the cap, taking each unit's lease first and skipping any unit whose lease is held.

Shared names, types and interfaces are pinned before wave 1 starts, in the plan or by a first step that every parallel step requires, so parallel workers build against one contract. Read-only reviewers are not scheduled; they run whenever needed. Today `ready` reads the working tree and knows no leases ([Tool status](records.md#tool-status)).

## Plans

A plan ([template](../templates/plan.md)) groups its steps by wave. Each step is one PR and one `Implement:` issue, and carries these fields:

```markdown
### Wave 2

3. **[P] Web canvas presenter.** Implement the presenter in `web/src/screen.ts`.
   - Tier: recommended. Goal: the web view shows what the terminal shows. Enables: 5, C006.
   - Requires: 1. After: 2, rework if early: the shared escape table may change.
   - Files: `web/src/screen.ts`. Cites: D002, C004.3.
   - Done when: `vitest run screen` passes.
```

- `Requires:` hard blockers, by step number. The step's issue has exactly these as blocked-by, and a step is never in the same wave as one it requires.
- `After:` soft edges, each with what may need redoing if the step starts before its target finishes.
- `[P]`: the step may run in parallel with the other `[P]` steps in its wave; they share no files and none requires another. A step without `[P]` runs alone in its wave, such as an integration or migration step.
- `Tier:` `crucial`, `recommended` or `contingent`. Crucial steps are dispatched first in a wave. A contingent step names the trigger that schedules it and does not start before the trigger holds.
- `Goal:` what is true after the step that is not true before. `Enables:` the later steps and concepts it unlocks, so a reviewer can see why it exists and what stalls if it slips.
- `Files:` every path the step writes. Two steps in one wave never share one.
- `Cites:` the decisions and items the step implements ([decisions.md](decisions.md#coverage)).
- `Done when:` a check that a test or CI job runs.
