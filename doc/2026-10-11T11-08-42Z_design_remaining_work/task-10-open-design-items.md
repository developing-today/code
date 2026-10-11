# Task 10: Open design items

- Status: Open. These have no design section yet, or need one. Each is a question to answer, then a design note, before implementation.
- Index: [what is left](./2026-10-11T11-08-42Z_design_remaining_work.md). Depends on: [task 02](./task-02-events-and-storage.md).
- Design: [design §6, §9.5, §14, §15](../2026-10-11T03-44-33Z_design_groups_events_delegation/2026-10-11T03-44-33Z_design_groups_events_delegation.md).

## 1. Retention and pruning

The concurrency checks need the events back to the removal's causal past (design §6.4). Decide what must survive, and how pruning respects it. `PruneFlag` in p2panda-core turns off the backlink check, so it is not a substitute.

## 2. Multi-device

- Each device has its own key and its own chain. An account authorises devices with signed `device.add` and `device.revoke` events, which are delegations from the account key.
- Removing an account removes its devices, and its delegations follow the group's delegation policy.
- p2panda-spaces handles multi-device group encryption. It is not production-ready (see task 05), so the design needs a plan without it.
- The warnings count recipient devices, so device counts matter for the size tiers.

## 3. Fork recovery

A forked chain is frozen (task 02). Recovery: managers remove the author, then re-enrolment under a new device key (recommended). Decide the UI and the steps.

## 4. Content on removal

Keep (default) or hide concurrent content. Documents and tags concurrent with a removal follow this policy, not membership voidness (task 03, task 06).

## 5. Use-count bounds

Counts do not converge across partitions. Drop them, or keep them as local enforcement by the hosting node. Decide, and document the choice.

## 6. Live editing

The ProseMirror collab server stays the live authority. A document event comes from a snapshot. The trigger is open: on idle close, on a timer, or on explicit save. Decide before task 06's document work.

## 7. Views

Design §14 lists the CLI, explorer, and web views. None is built. In brief:

- CLI: `group create|policy|join|add|remove|role|list`, `delegate`, `revoke`, `publish document|tag`, `events`, `frontier`, `history`.
- Explorer: new lines in the shared grammar in `directory_view.rs`, so SSH and HTML stay in step.
- Web: a websocket push when events are applied or voided. Sections for waiting events (with the missing dependency), voided events (with the removal that voided them), and fork proofs. A verification mark per event.
- Every rejection gives a reason: "voided: concurrent with R", "waiting: missing X", "frozen: fork at seq n".
- "As of" means a frontier, not a clock time.

## 8. Cross-world rules

The cross-world access and world isolation rules (see [task 03](./task-03-membership-and-delegation.md), item 7) must be reconciled with delegation. Not re-read for this task.

## 9. Warnings UI

The size tiers (task 05) need a UI in every surface that shows group status.

## Acceptance

Each item gets a short design note, linked from the design, before any implementation starts. Each note records the decision and the test that would show it is right.
