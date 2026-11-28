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

## 2026-10-11T12-53-32Z Update: preferred answers for each item

Each item below gives my recommendation first, then the alternatives. The owner decides; the design note and test are still required before implementation.

1. **Retention.** Preferred: keep every event back to the causal past of each open removal and each open frontier. Prune only behind a snapshot at a causal cut (S8). Test: a replica that joins after pruning reaches the same state. Alternative: no pruning, simplest and growing. Alternative: time-based retention with a snapshot.
2. **Multi-device.** Preferred: each device has its own chain. The account authorises devices with signed `device.add` and `device.revoke` delegations, and the warnings count devices. Alternative: one device per account at launch. Note that p2panda-spaces is not production-ready, so the group-encryption plan must not depend on it (task 05).
3. **Fork recovery.** Preferred: removal and re-enrolment under a new device key (decision 17). Alternative: automatic quarantine, with owner approval. Test: after a fork, the frozen chain is no longer accepted, and the re-enrolled device reaches the same state.
4. **Content on removal.** Preferred: keep content (decision 4). Hide concurrent content only with an explicit flag. Alternative: hide concurrent content for a window. Test: as in task 03.
5. **Use-count bounds.** Preferred: drop them from the protocol and keep local enforcement at the hosting node. Counts do not converge across partitions, so they cannot be authoritative. Alternative: keep signed bounds. Test: a partition test shows no divergence in state.
6. **Live editing.** Preferred: the collab server stays the live authority. Write a document event on explicit save and on idle close. Alternative: timer only. Alternative: every change, which is large. Decide before task 06's document work.
7. **Views.** Preferred: CLI first, then web. Keep the rejection reasons: "voided", "waiting", "frozen". Alternative: web first.
8. **Cross-world rules.** Not changed here. Still to be reconciled with delegation (task 03, item 7).
9. **Warnings UI.** Preferred: show the size tier in group status in both the CLI and the web view. Alternative: CLI only at first.

## 2026-10-11T16-08-10Z Decision: fork recovery (decision 17), with the alternative elaborated

**Preferred, accepted: removal and re-enrolment.** When a chain forks, the chain is frozen at the fork point, and the fork proof is stored and shared. The managers remove the author. The person re-enrols under a new device key. Events from the forked chain after the fork point are not applied.

**Alternative: quarantine with approval.** Same first step, the freeze and the proof. Then, instead of an immediate removal, the post-fork events of each branch are held in quarantine. They are stored, not applied, and not voided. A manager then chooses one of three paths:
1. Approve one branch. Its events are applied, and the other branch stays frozen.
2. Approve neither. The author is removed and re-enrols, which is the preferred path.
3. Approve a merged view, only if the two branches can be reconciled by hand.

Why this could be better: a fork often comes from a restored backup or a copied key. The post-fork events on the other branch may be real work, and quarantine keeps them for review, where the preferred path would not apply them.

Costs: more state held for a while, an approval interface for managers, a rule for who may approve, and a timeout. The timeout is needed, because otherwise the quarantine is open forever. After the timeout the preferred path applies automatically.

Security: while a branch is quarantined, its events are not applied, so the forked key cannot act through it.

Decision: the preferred path is accepted. Quarantine is recorded as a later option, not a step in Phase 6.

## 2026-10-11T17-58-40Z Update: round 2 results

- The round 2 answers are in the [explainers](./explainers.md), the [index](./2026-10-11T11-08-42Z_design_remaining_work.md), and the [round 2 results](./research-round2/README.md).
- The steps this task feeds are in the [phased plan](./plan-phases.md). Each step names its blocker and its tests.
