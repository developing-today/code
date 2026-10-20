# Scheduling, Ordering & Frontier Dispatch

This document details how dependency graphs are evaluated into execution waves and how the ready frontier is computed for autonomous workers.

## 1. Graph Semantics

The workflow graph model uses typed edges from `planning/DEPENDENCIES.md` to establish execution order:

- **`requires` (Hard Blocker)**: Defines the topological sort order. A concept or step with unsatisfied `requires` edges cannot be dispatched under any circumstances.
- **`recommended-after` (Soft Sequencing)**: Recommends that source follow target to minimize rework. If workers are idle and target is still in progress, source may proceed, provided its task brief explicitly states the assumptions it is making.
- **`shares-files-with` (Mutual Exclusion)**: Two tasks that modify the same files are assigned to different waves or locked under a shared mutex so they never run concurrently in separate worktrees.

## 2. Wave Scheduling

Orchestrators group work into waves:

```text
Wave 0: Shared Contracts & Foundations
  ├── C001: Data model & type definitions
  └── C002: Core schema migration
        │
Wave 1: Parallel Feature Implementations [P]
  ├── C003: Ingest pipeline worker
  ├── C004: HTTP presentation presenter
  └── C005: Event stream subscriber
        │
Wave 2: Integration & Verification
  └── C006: End-to-end integration test & validation
```

- **Wave 0 (Foundations)**: Shared types, schema, and API contracts must be merged first.
- **Wave 1 (Parallel Builders)**: Independent features run simultaneously across parallel worktrees. Each step is marked `[P]` in implementation plans.
- **Wave 2 (Integration)**: Final verification and cross-cutting documentation.

## 3. Ready Frontier Calculation

At any given point during execution, the orchestrator computes the **Ready Frontier** using `scripts/research_workflow.py ready`:

1. **Exclude Completed Concepts**: Concepts with status in `COMPLETE` (`done`, `completed`, `merged`, `validated`) are removed.
2. **Exclude Active Concepts**: Concepts with status in `ACTIVE` (`running`, `active`, `in-progress`) are removed to prevent duplicate dispatch.
3. **Filter by Prerequisites**: A concept is ready if and only if all of its `requires` dependencies have reached `COMPLETE` status.
4. **Partition by Concurrency Fences**: If two ready concepts share a `shares-files-with` edge, only one is scheduled in the immediate wave; the other waits for the next wave.
5. **Lease Guard**: Before dispatching, the orchestrator acquires a live lease in `agent-work/<pkg>-<stage>/locks/concept-<id>.lock`. If a lease already exists on disk, dispatch is skipped.

## 4. Implementation Plan Output Format

When generating an implementation plan in Stage 7 (`feature-plan`), steps are formatted with explicit parallel markers:

```markdown
### Phase 1: Core Surface Presenters

- [ ] **1.1.1** [P] Define presenter traits in `src/presenter.rs`. (Done when: `cargo test --lib presenter` passes).
- [ ] **1.1.2** [P] Implement terminal ANSI screen renderer in `src/terminal.rs`. (Requires: 1.1.1; Done when: unit tests pass).
- [ ] **1.1.3** [P] Implement web canvas presenter in `web/src/screen.ts`. (Requires: 1.1.1; Done when: vitest passes).
- [ ] **1.1.4** Integration presenter parity check. (Requires: 1.1.2, 1.1.3; Done when: parity suite passes).
```

`[P]` explicitly tells the implementation orchestrator which tasks can be fanned out across parallel worktrees simultaneously.
