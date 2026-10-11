# What is left in the design

- Date: 2026-10-11 (UTC 11:08)
- Purpose: the single list of remaining design and implementation work. Each `task-NN` file in this folder is a starting point for one area. It lists the decisions, the spikes, the acceptance tests, and the files involved.
- Status terms: **Done** (in code), **Designed** (decided, with a design section), **Spiked** (evidence exists), **Open** (needs a spike or a decision first), **Deferred** (explicitly later).
- Source design: [design](../2026-10-11T03-44-33Z_design_groups_events_delegation/2026-10-11T03-44-33Z_design_groups_events_delegation.md). Where a later document differs, the later one is current: [review](../2026-10-11T03-44-33Z_design_groups_events_delegation/2026-10-11T08-08-56Z_design_review.md) and [connectivity](../2026-10-11T03-44-33Z_design_groups_events_delegation/2026-10-11T10-46-58Z_design_connectivity.md).

## Done

| Item                                                                    | Evidence                                                                                                                                     |
| ----------------------------------------------------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------- |
| n0 relays by default, one endpoint policy, dial deadlines               | [feature doc](../2026-10-11T11-08-42Z_feature_n0_relay_default/2026-10-11T11-08-42Z_feature_n0_relay_default.md), commit `07f144c9`          |
| Event header, canonical CBOR, and their vectors (S1, S5)                | [spikes](../2026-10-11T03-44-33Z_design_groups_events_delegation/spikes/README.md)                                                           |
| Membership, friendship, delegation semantics (S2)                       | spikes                                                                                                                                       |
| Full history, from_join, PCS scope, ordering counts (S3)                | spikes                                                                                                                                       |
| Blob hash equality, gating, verified streaming (S4)                     | spikes                                                                                                                                       |
| Storage choice: redb over SQLite (S6)                                   | spikes                                                                                                                                       |
| Connectivity: n0, our relay via Tunnel, DHT and mDNS checks (C1, t1–t8) | [connectivity design](../2026-10-11T03-44-33Z_design_groups_events_delegation/2026-10-11T10-46-58Z_design_connectivity.md) and the relay kit |

## Left

| Task                                         | Area                                                                                         | Status                             | Needs first    | Open decisions                                                     |
| -------------------------------------------- | -------------------------------------------------------------------------------------------- | ---------------------------------- | -------------- | ------------------------------------------------------------------ |
| [01](./task-01-phase0-fixes.md)              | Phase 0: tuple prefix bug, pull page bytes, MSRV, dial deadlines in the remaining modules    | Open                               | nothing        | none                                                               |
| [02](./task-02-events-and-storage.md)        | Event header, canonical CBOR, redb store, fork proof, bounded waiting                        | Designed, Spiked                   | 01             | waiting caps (proposed 256 events per author, 7 days)              |
| [03](./task-03-membership-and-delegation.md) | Auth-frontier shim, admission gate, friendship, delegation modes, world capabilities         | Designed, Spiked                   | 02             | dependents rule; transfer; use-count bounds; content on removal    |
| [04](./task-04-sync-blobs-transport.md)      | Range sync over iroh, blob gating, `observe`                                                 | Designed; S7 open                  | 02; S7         | `observe` gating default                                           |
| [05](./task-05-encryption-and-history.md)    | DataScheme adapter, our DGM, heads orderer, from_join, welcome growth, warnings, scheme      | Designed, Spiked; S10 and S12 open | 02             | `recent:N` option; large-group gate (BeeKEM)                       |
| [06](./task-06-tags-documents-search.md)     | Observed-remove tags, index, documents, search                                               | Designed; S8 open                  | 02             | tag precedence against the content policy                          |
| [07](./task-07-invites-and-pull.md)          | Seitan V2, invite events, links, pull policy, metadata documentation                         | Designed; S9 open                  | 02             | bearer or typed; approval; pull default                            |
| [08](./task-08-connectivity-extensions.md)   | Our relay, QUIC flag per relay, DHT, Worker pkarr relay, mDNS, ticket policy, access control | Designed; kit ready                | owner accounts | relay option; access control; DHT and mDNS defaults; ticket policy |
| [09](./task-09-time-and-witnesses.md)        | Witness events, cosigning, anchors                                                           | Deferred                           | 02             | who runs witnesses; which anchors                                  |
| [10](./task-10-open-design-items.md)         | Retention, multi-device, fork recovery, live editing, views                                  | Open                               | 02             | several (listed in the task)                                       |
| [11](./task-11-dependencies-and-upstream.md) | Crate adoption, pins, upstream bug reports                                                   | Open                               | 01             | ed25519 version alignment; iroh-docs feature change                |

## Spikes still to run

- **S7**: range sync over iroh on real stores, 1,000 chains with real events. Needed before task 04.
- **S8**: observed-remove tags, with property tests over random orders. Needed before task 06.
- **S9**: Seitan V2 implementation against the test vector. Needed before task 07.
- **S10**: heads orderer in a 40-member group, with concurrency and the merge at 32. Needed before task 05.
- **S11**: relay operation on one home server (restart, reconnect, relay key rotation). Needed before task 08.
- **S12**: welcome size at 1,000 rotations, with and without the `serde_bytes` patch. Needed before task 05's history decision.
- **C2**: holepunching between two peers on separate networks, both homed on our relay, with n0's relays in their maps. Not possible in this sandbox. Needed before relying on direct paths (task 08).
- **Re-run t2 and t4** (connectivity) on current infrastructure. They passed earlier, but were not re-run after the QUIC-flag change.
- **BeeKEM**: a spike before end-to-end groups above 1,000 devices (task 05's gate).

## Owner decisions still open

1. Dependents of voided events: authority-based (recommended) or closure over all dependents (task 03).
2. Transfer on removal: derived from the signed removal (recommended) or a separate signed event (task 03).
3. Use-count bounds on world capabilities: drop them or keep them as local enforcement (task 03).
4. Content on removal: keep (default) or hide concurrent content (task 03, task 06).
5. `observe` gating on blobs: gate it (recommended) or accept the metadata leak (task 04).
6. Waiting caps: 256 pending events per author and 7 days, as proposed (task 02).
7. Joiner history: `full` by default, and whether to add `recent:N` for large groups (task 05).
8. `scheme: none` notice and wording (task 05).
9. Large-group gate above 1,000 devices (task 05).
10. Invite style: bearer link, typed key, or both; approval on or off; default expiry (task 07).
11. Pull default: open (decided), with an authorized option (task 07).
12. Our relay: option A (Tunnel, recommended) or B (VPS), and the accounts to provide (task 08).
13. Relay access control: shared token (recommended) or open (task 08).
14. mDNS default: off (design) or on (current `serve` default) (task 08).
15. DHT default: off, opt-in (task 08).
16. Ticket policy: relay list only, no IPs (recommended) (task 08).
17. Fork recovery: removal and re-enrolment (task 10).
18. Upstream patch policy for p2panda-encryption (task 11).
19. ed25519-dalek version: align with p2panda-core's 2.2 or keep our 3.0 (task 11).

## Known discrepancies in the current code

- `serve` enables mDNS by default. The connectivity design says off (task 08, decision 14).
- Pull pages are capped by item count, not bytes (task 01).
- `pkgs/id` declares Rust 1.91.0. The p2panda crates need 1.96 (task 01).
- `pkgs/id/src/tuple.rs` has the NUL prefix trap, which `tags.rs` uses (task 01).
- Artifacts sign serde_json (task 02 replaces this).
- The Roc test and the world-module hang are pre-existing, and the three `world_compile` tests need `host.wasm`. See the feature doc.

## Suggested order

1. Task 01 (small, unblocks the rest).
2. Task 02 (everything else depends on the event store).
3. Tasks 03, 04, 05, 06, and 07 in parallel, after their spikes run. Each has a spike gate.
4. Task 08 whenever the owner has chosen a relay setup. Its code parts can start earlier.
5. Tasks 09, 10, and 11 as needed.

## Evidence

- Spike code and results: [spikes](../2026-10-11T03-44-33Z_design_groups_events_delegation/spikes/README.md).
- Research: [findings](../2026-10-11T03-44-33Z_design_groups_events_delegation/2026-10-11T04-20-17Z_research_findings.md).
- Connectivity kit: [relay configs](../2026-10-11T03-44-33Z_design_groups_events_delegation/connectivity/relay/).
