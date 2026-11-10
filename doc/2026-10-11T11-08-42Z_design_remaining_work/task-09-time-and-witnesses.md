# Task 09: Time and witnesses

- Status: Deferred, as the owner decided. The design keeps causal order as the only order used for validity.
- Index: [what is left](./2026-10-11T11-08-42Z_design_remaining_work.md). Depends on: [task 02](./task-02-events-and-storage.md).
- Design: [design §12](../2026-10-11T03-44-33Z_design_groups_events_delegation/2026-10-11T03-44-33Z_design_groups_events_delegation.md). Research: [§10](../2026-10-11T03-44-33Z_design_groups_events_delegation/2026-10-11T04-20-17Z_research_findings.md).

## Goal

Later, give events a provable time, without making time decide validity. Claimed time stays display-only until this task lands.

## Correction to the design

A witness proves that an event existed at or before the signed time. The design said "at or after", which is wrong. This is recorded in the design's appended discovery section.

## Options (from research §10)

1. **In-band witness event.** A signed event whose payload is a Merkle root of a batch of event hashes, with the heads in `after`. Each event in the batch needs an inclusion path, which the 32-hash cap makes necessary.
2. **Sidecar cosignature.** C2SP-style: a witness cosigns `(chain, seq, root)` after checking consistency with what it cosigned before. Witnesses refuse a head that conflicts with one they already cosigned, so a forked author cannot get a quorum for both branches. The C2SP spec sets no quorum. Sigsum's split-view bound needs k > 2n/3 (for example 3-of-3, 4-of-5, 5-of-7).
3. **External anchors.** OpenTimestamps (Bitcoin-anchored Merkle roots, hours of latency, no operator clock trusted), RFC 3161 (one TSA is a trust point), or Roughtime (several servers, RFC 10049).
4. **Display ordering.** Hybrid logical clocks, for display only. `uhlc` 0.9.0 provides the receive rule. p2panda-core's timestamp module does not.

## Open questions

- Who runs witnesses. Home servers suit this, since C2SP witnesses serve many rarely active logs. With three witnesses the quorum is three, so one outage stalls cosigning.
- Which anchors. OpenTimestamps is the one that trusts no operator clock.
- Whether the witness is a separate event type or a checkpoint (task 05's merge update is a different thing).

## Acceptance (once started)

- A witnessed batch proves existence at or before the time, in a test.
- Validity never depends on the witnessed time. A test shuffles the time and checks that the state is unchanged.
- An anchor verifies without contacting the anchor operator, for option 3.

## Gotchas

- Our per-author chains are not Merkle trees, so consistency proofs need a tree or a chain-segment scheme.
- Witness timestamps are self-asserted. Verifiers may reject future ones.
- Each checkpoint reveals the origin, the size, and the root.
