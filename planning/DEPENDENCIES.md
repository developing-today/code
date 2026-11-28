# Dependency Kinds & Planning Taxonomy

This document specifies the dependency relations, priority tiers, roadmap horizons, and lifecycle statuses used across research collections and implementation plans.

## 1. Dependency Relations

Edges between concepts, tasks, or features carry semantic kinds that dictate scheduling and concurrency:

| Kind                | Type                   | Meaning                                                                                          | Scheduling Behavior                                                                                                           |
| ------------------- | ---------------------- | ------------------------------------------------------------------------------------------------ | ----------------------------------------------------------------------------------------------------------------------------- |
| `requires`          | Hard Blocker           | Source cannot begin until target is completed and verified.                                      | Strictly serialized. Hard dependency cycles are rejected during validation.                                                   |
| `recommended-after` | Soft Edge              | Source _can_ technically run in parallel or earlier, but doing so incurs a known risk of rework. | Placed in a later wave by default. Allowed to run early if resources are idle, provided the `rework_if_early` cost is logged. |
| `shares-files-with` | Concurrency Constraint | Source and target modify the same files, directories, or worktree paths.                         | Must never run in the same parallel execution wave. Dispatched sequentially to avoid git conflicts.                           |
| `conflicts-with`    | Alternative Fork       | Source and target represent mutually exclusive approaches or incompatible designs.               | Dispatched to the decision protocol (reviewer/critic subagents) to choose a direction or define a hybrid.                     |
| `enables`           | Downstream Unlock      | Completing source unlocks or unblocks future capabilities or follow-up features.                 | Informational and portfolio tracking; does not block current execution.                                                       |
| `optional-if`       | Contingent Path        | Source is only needed if a specific condition, feature flag, or external decision is triggered.  | Requires an explicit `trigger` condition before being scheduled into `now`.                                                   |
| `supersedes`        | Historical Replacement | Source updates, refines, or replaces target.                                                     | Target status transitions to `superseded` with a backlink to source.                                                          |
| `contradicts`       | Logical Conflict       | Source contradicts target statements or requirements.                                            | Flagged during intake or research; requires human confirmation or explicit clarification.                                     |

## 2. Priority Tiers

Every concept and feature specifies its priority tier:

- **`crucial`**: Core capability or foundational invariant. The system or release is incomplete without it.
- **`recommended`**: Standard best practice, hardening, performance optimization, or quality-of-life feature. Highly desirable, but does not block minimal viable validation.
- **`contingent`**: Built "in case we one day need that path". Low immediate urgency; explored only when prerequisites are idle or an external trigger demands it.

## 3. Roadmap Horizons

Alinged with the repository's planning framework in `planning/README.md`:

- **`now`**: Active work or immediate next step. Backed by open issues or active PRs.
- **`next`**: Direction decided and shape mostly understood. Scheduled immediately after `now`.
- **`later`**: Direction chosen, but detailed shape intentionally deferred to avoid premature commitment.
- **`ideas`**: Recorded to preserve intent and prevent loss; no commitment to implement.

## 4. Status Vocabulary

### A. Statement Status (What We Know)

- `created`: Recorded from source material or initial prompt.
- `clarified`: Refined with additional nuance or precision without changing intent.
- `confirmed`: Explicitly validated by user confirmation or codebase verification.
- `superseded`: Replaced by a more comprehensive or updated item.
- `retracted`: Withdrawn as inaccurate, misheard, or no longer applicable.

### B. Disposition (What We Do With It)

- `proposed`: Awaiting evaluation or review.
- `accepted`: Approved for planning and implementation.
- `planned`: Detailed in an implementation plan with numbered steps and test contracts.
- `in-progress`: Actively being built in an open PR.
- `implemented`: Code complete and merged.
- `validated`: Verified against all automated and manual criteria.
- `deferred`: Postponed with a documented reason and revisit trigger.
- `rejected`: Decided against with a documented rationale and rejected alternatives.
