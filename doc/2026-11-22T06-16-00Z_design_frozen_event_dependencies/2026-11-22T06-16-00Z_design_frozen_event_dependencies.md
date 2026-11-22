# Frozen-chain event dependencies

## Intent

Issue [#12050](https://github.com/developing-today/code/issues/12050) reports that the disposition of a dependent event changes with arrival order when its `after` hash names a valid event above a later fork point. An event received before the fork is stored and later voided; the same event received after the fork is currently refused as frozen and is absent. The contract nevertheless says a stored voided event satisfies `after`.

The chosen invariant is to preserve Phase 1 dependency presence: a verified post-fork event at an unoccupied author/sequence position is retained as voided evidence, never applied. This makes the `after` target present in either arrival order without making the event authoritative. The store has one event row per author/sequence position, so a rival for an occupied post-fork position is refused as frozen. A conflict at the fork sequence that does not alter the recorded proof remains a frozen refusal.

## Rationale and trade-off

`event_store/ingest.rs` defines stored voided events as present for dependencies and retained for conflict evidence. Excluding voided events from `after` would change that existing contract and its focused test. Retaining a post-fork arrival changes its outcome from refusal to stored/voided, which is the narrower compatibility change and matches how the same signed event is treated when it arrives before the fork, provided no distinct event already occupies that sequence. It may retain additional authenticated event bytes from a frozen author; the implementation must not add those events to the waiting pool or apply their actions.

The final applied state is already defined by stored events and fork proofs and is checked across arrival orders; this change targets the arrival-dependent ingest/dependency result and the stranded waiting row.

## Initial implementation plan

1. Add an EventStore-level regression that sends the same signed event set in both fork/event orders and checks that a unique post-fork event is stored as voided and the other author's `after` dependency resolves in both.
2. Change the `decide_in` path so a verified event above an existing fork point at an unoccupied position is stored directly as voided evidence, without entering the waiting pool or applying. Preserve frozen refusal for occupied post-fork positions and an extra event at the fork point that cannot change the selected proof.
3. Update module/API comments and the existing released-event regression, then run focused `id-events` tests, formatting, and diff checks.

## References

- [Issue #12050](https://github.com/developing-today/code/issues/12050)
- `pkgs/id/crates/id-events/src/event_store/ingest.rs`, especially the Phase 1 presence contract and `decide_in`.
