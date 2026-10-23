# Task 03: Membership, friendship, and delegation

- Status: Designed and spiked (S2). The dependents rule, transfer, use counts, and content on removal need owner decisions.
- Index: [what is left](./2026-10-11T11-08-42Z_design_remaining_work.md). Depends on: [task 02](./task-02-events-and-storage.md).
- Design: [design §5–§7](../2026-10-11T03-44-33Z_design_groups_events_delegation/2026-10-11T03-44-33Z_design_groups_events_delegation.md), [review §3.4](../2026-10-11T03-44-33Z_design_groups_events_delegation/2026-10-11T08-08-56Z_design_review.md). Evidence: S2 in the [spikes](../2026-10-11T03-44-33Z_design_groups_events_delegation/spikes/README.md).

## Goal

Membership, friendship, and delegation are computed from events at each event's causal position, using p2panda-auth's StrongRemove for concurrency and our own layer for everything p2panda-auth does not provide.

## Scope

1. **Auth-frontier shim.** Each auth event's `dependencies` are derived on receipt from the verified `after` closure. The derivation takes the maximal auth operations of the same group in the event's full causal past, walking through non-auth events. The stored dependencies are not trusted. S2 showed that a raw `after` stalls with `MissingDependencies`, and that a naive literal mapping either panics or wrongly voids a valid add.
2. **Admission gate.** An operation with a missing auth dependency waits. An operation for a group absent at its dependencies is rejected. Every event our layer refuses becomes a placeholder no-op in the auth graph, so its descendants do not stall (S2 P7).
3. **Friendship.** Two members, both must accept. The two-member cap is an ingest rewrite: an over-cap add becomes a no-op. In production, keep the signed event and derive the rewrite, as the spike noted. Dissolve is a projection: an effective removal of either member ends the friendship. A re-add restores membership, not friendship.
4. **Delegation.** Edges with attenuation at creation, chain search (any intact chain from the owner, the UCAN rule), and a veto on a later action with no intact chain. Three modes on removal of a delegator:
   - `cascade` (the default, decided);
   - `keep_prior`;
   - `transfer`, derived from the removal's successor field, with the successor required to hold the scope at the action's position.

   Expiry is `expires_at` in the signed delegation.

5. **Void reasons**, recorded per event: "voided: concurrent with R", "voided: author lost authority through Z", "vetoed: no intact chain", "refused: missing dependency" with the hash.
6. **World capabilities.** Replace the live `are_friends` check in `world.rs` (`effective_scopes`) with a chain check at the action's position. Bearer join tokens map to delegation events by digest. Revocation is a `revoke` event.
7. **Cross-world rules.** Reconcile with the [cross-world access](../2026-10-10T04-59-57Z_feature_cross_world_access/2026-10-10T04-59-57Z_feature_cross_world_access.md) and [world isolation](../2026-10-10T03-56-25Z_feature_world_isolation/2026-10-10T03-56-25Z_feature_world_isolation.md) docs. This task did not re-read them.

## Decisions

- Dependents of voided events: authority-based, as p2panda does (recommended). Closing over all dependents changes outcomes (S2 case 3b).
- Transfer: derived from the signed removal (recommended).
- Use-count bounds on capabilities: they do not converge across partitions. Drop them, or keep them as local enforcement by the hosting node.
- Content on removal: keep (default) or hide concurrent content (see [task 06](./task-06-tags-documents-search.md)).

## Acceptance

- The nine S2 scenarios, each in every causal order (24 orders in the spike): week-late add voided, causally prior add survives, transitive void by authority, mutual removal, three-cycle, concurrent re-add voided, and the causal re-add control.
- The S2 delegation cases: cascade vetoes the later action with only Joe's chain, and it survives through Dan's chain. `keep_prior` and `transfer` behave as the table in S2 says.
- Friendship: concurrent over-cap adds resolve the same way in every order. Dissolve on either side's removal.
- The existing world capability tests and the cross-world tests still pass.

## Gotchas

- An unknown dependency is an error, not a panic. The real panic in p2panda-auth is "group absent at dependencies", at `group/crdt/mod.rs:822`. Guard it with the admission gate.
- p2panda-auth has no member cap, no dissolve, and no cascade. Those are ours.
- The `ignore` set is cleared on every `process` call, and `apply_action` is crate-private. A cap must be an ingest rewrite.
- p2panda-auth's graph is per process. Concurrency across groups makes `heads()` rebuild globally. Measure before scaling.

## Files

New module, for example `pkgs/id/src/membership/`. `world.rs` (capabilities), `directory.rs` and `directory_view.rs` (friend lines become events).

## Dependencies

p2panda-auth (git, pinned). See [task 11](./task-11-dependencies-and-upstream.md).
