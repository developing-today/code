# What is left in the design

- Date: 2026-10-11 (updated 12:20 UTC)
- Purpose: the single list of remaining design and implementation work. Each `task-NN` file in this folder is a starting point for one area. It lists the decisions, the spikes, the acceptance tests, and the files involved.
- Status terms: **Done** (in code), **Designed** (decided, with a design section), **Spiked** (evidence exists), **Open** (needs a spike or a decision first), **Deferred** (explicitly later).
- Source design: [design](../2026-10-11T03-44-33Z_design_groups_events_delegation/2026-10-11T03-44-33Z_design_groups_events_delegation.md). Where a later document differs, the later one is current: [review](../2026-10-11T03-44-33Z_design_groups_events_delegation/2026-10-11T08-08-56Z_design_review.md) and [connectivity](../2026-10-11T03-44-33Z_design_groups_events_delegation/2026-10-11T10-46-58Z_design_connectivity.md).

## Done

| Item | Evidence |
| --- | --- |
| n0 relays by default, one endpoint policy, dial deadlines | [feature doc](../2026-10-11T11-08-42Z_feature_n0_relay_default/2026-10-11T11-08-42Z_feature_n0_relay_default.md), commit `07f144c9` |
| Tuple terminator: bytes and strings end with `00 00`, a lone `00` is rejected | commit `00dccae5` on `feat/id-n0-relay-default`; regression tests in `pkgs/id/src/tuple.rs`; tags (71) and lib suite (748) pass |
| Event header, canonical CBOR, and their vectors (S1, S5) | [spikes](../2026-10-11T03-44-33Z_design_groups_events_delegation/spikes/README.md) |
| Membership, friendship, delegation semantics (S2) | spikes |
| Full history, from_join, PCS scope, ordering counts (S3) | spikes |
| Blob hash equality, gating, verified streaming (S4) | spikes |
| Storage: redb over SQLite (S6), provisional until Turso is measured (N1) | spikes |
| Connectivity: n0 default, Tunnel first for our relay, DHT and mDNS checks (C1, t1–t8); t2 and t4 re-run and pass | [connectivity design](../2026-10-11T03-44-33Z_design_groups_events_delegation/2026-10-11T10-46-58Z_design_connectivity.md), `spikes/c1-rerun` |

## Left

| Task | Area | Status | Needs first | Open decisions |
| --- | --- | --- | --- | --- |
| [01](./task-01-phase0-fixes.md) | Phase 0: tuple (done), pull page bytes, MSRV, dial deadlines in the remaining modules | Partly done | nothing | MSRV value (N15 with task 12) |
| [02](./task-02-events-and-storage.md) | ChainExt, canonical CBOR, redb store, fork freeze, bounded waiting | Designed, Spiked | 01, 12 | #6; N1–N4; N13 |
| [03](./task-03-membership-and-delegation.md) | Auth-frontier shim, admission gate, friendship, delegation modes, world capabilities | Designed, Spiked | 02 | #1–#4 |
| [04](./task-04-sync-blobs-transport.md) | Range sync over iroh, blob gating, `observe` | Designed; S7 done | 02; S7 | #5; N5, N6 |
| [05](./task-05-encryption-and-history.md) | DataScheme adapter, our DGM, heads orderer, from_join, welcome growth, warnings, BeeKEM | Designed; S10, S12, BeeKEM done (BeeKEM not adopted) | 02, 12 | #7, #8, #9; N7–N12, N18 |
| [06](./task-06-tags-documents-search.md) | Observed-remove tags, index, documents, search | Designed; S8 done | 02 | tag precedence against the content policy |
| [07](./task-07-invites-and-pull.md) | Seitan V2, invite events, links, pull policy, metadata | Designed; S9 done | 02 | #10, #11 |
| [08](./task-08-connectivity-extensions.md) | Our relay (Tunnel), QUIC flag, DHT, Worker pkarr relay, mDNS, tickets, access control | Designed; kit ready; S11 and C2 done | owner's Tunnel account (N19) | #13, #14, #15, #16 |
| [09](./task-09-time-and-witnesses.md) | Witness events, cosigning, anchors | Deferred | 02 | who runs witnesses; which anchors |
| [10](./task-10-open-design-items.md) | Retention, multi-device, fork recovery, live editing, views | Open | 02 | #17 and the rest listed in the task |
| [11](./task-11-dependencies-and-upstream.md) | Crate adoption, upstream reports (none opened) | Open; in part superseded by 12 | 01 | #18 (decided), N9, N10 |
| [12](./task-12-dependencies-and-toolchain.md) | Update all dependencies and the Rust toolchain (1.99.0; no nightly needed) | Open | 01 | decisions A–D in the task; #19 |

## Spikes

| Spike | Question | Status | Result |
| --- | --- | --- | --- |
| S1–S6, C1 | Earlier spikes (see above) | Done | [spikes](../2026-10-11T03-44-33Z_design_groups_events_delegation/spikes/README.md) |
| C1 t2, t4 | Connectivity re-run on current infrastructure | Done, both pass | `spikes/c1-rerun` |
| S7 | Range sync on real stores over iroh, 1,000 and 10,000 chains with real events | Done: range works on real stores; K=16 and T=16 kept | `spikes/s7-range-real` |
| S8 | Observed-remove tags: property tests over random orders | Done: tags hold; the clock must cover the causal past; compaction needs a snapshot | `spikes/s8-tags` |
| S9 | Seitan V2 against vectors, freshness check, admin path | Done: derivations match Keybase's vectors; freshness window proposed; a ticket cannot carry the token | `spikes/s9-seitan` |
| S10 | Heads orderer in a 40-member group, merge at 32 | Done: cap 32 needs the merge rule and the author's own latest event named every time | `spikes/s10-heads` |
| S11 | Relay operation: restart, reconnect, key and certificate rotation | Done: restart and reconnect measured; systemd `KillSignal` and metrics exposure need fixing | `spikes/s11-relay` |
| S12 | Welcome size at 1, 10, 100, 1,000 epochs, with and without serde_bytes | Done: 146.6 B per epoch stock, 40.0 B with serde_bytes (a new scheme version) | `spikes/s12-welcome` |
| C2 | Holepunching between separate networks, using namespaces | Done: full cone and port-restricted holepunch succeed (3 of 3 each); symmetric against symmetric falls back; QAD is required for a direct path | `spikes/c2-holepunch` |
| BeeKEM | Costs at 100, 1,000, 5,000; concurrency; join; migration | Done: not adoptable now (no welcome API, no authority checks); DataScheme stays | `spikes/beekem` |

## Owner decisions (the original 19, with this session's answers)

| # | Decision | Status | Preferred | Alternatives | Task |
| --- | --- | --- | --- | --- | --- |
| 1 | Dependents of voided events | Open | Authority-based, as p2panda's StrongRemove does | Closure over all dependents (more voids) | 03 |
| 2 | Transfer on removal | Open | Derived from the signed removal | A separate signed transfer event | 03 |
| 3 | Use-count bounds on world capabilities | Open | Drop them; keep local enforcement only | Keep as signed bounds | 03 |
| 4 | Content on removal | Open | Keep content; removal stops new content | Hide concurrent content; local purge on request (not a guarantee) | 03, 06 |
| 5 | `observe` gating on blobs | Open; you asked who can see it, per server, per object, or per uploader | Gate each request: serve only to peers who could fetch the blob under the world's rules. Observe off by default | Disable observe, take size from the referencing event; accept the leak with a notice | 04 |
| 6 | Waiting caps | Open | 256 pending per author, 7 days | Tighter for public worlds (64, 24 h); no time cap | 02 |
| 7 | Joiner history | Partly answered: you prefer `recent:N` over `full`, and want `full` on request later | Default `recent:N`; `full` by request through a lazy history protocol (new item). N from S12 | `full` always (grows with rotations); forward-only (no history) | 05 |
| 8 | `scheme: none` notice | Open | Explicit notice in the UI and in group metadata | Refuse unencrypted groups except in public worlds | 05 |
| 9 | Large-group gate | **Answered:** no gate. Warnings only. Servers warn locally when client-made groups approach or exceed the thresholds. BeeKEM gets a migration path with a transition window | Confirm the thresholds 96, 128, 1,000 (N11) | none | 05 |
| 10 | Invite style | Open | Bearer link (Seitan V2) with expiry and single use; typed key optional with owner approval | Typed key only; bearer with approval always on | 07 |
| 11 | Pull default | Open | Open pull, as decided, with an authorized option | Authorized-only by default | 07 |
| 12 | Our relay | **Answered:** Tunnel first | Tunnel now; VPS later if needed | VPS first | 08 |
| 13 | Relay access control | **Answered for now:** no shared token (public service). Revisit at implementation | None now; rate limits at the relay | A non-secret token embedded in the binary; per-world tokens | 08 |
| 14 | mDNS default | Open | Off in the library; on for `serve` only when LAN discovery is wanted | On everywhere (current `serve` default) | 08 |
| 15 | DHT default | Open | Off; opt in per world | On for public worlds | 08 |
| 16 | Ticket policy | **Answered:** tickets yes, with relays, not direct addresses only | Relay list and no IPs by default; direct addresses opt-in | Include direct addresses (LAN setups) | 08 |
| 17 | Fork recovery | Open | Removal and re-enrolment | Automatic quarantine with owner approval | 10 |
| 18 | Upstream patch policy for p2panda-encryption | **Answered:** the from_join fix stays in our fork in developing-today; nothing goes upstream; no upstream issues or PRs; serde_bytes goes in as a new scheme version, not a silent patch (N10) | Keep the fork; later move from_join into our DGM if that proves simpler (N9) | Push upstream (rejected) | 11 |
| 19 | ed25519-dalek version | Open | 2.x, to match p2panda-core, one copy in the tree | 3.0 with a shim at the boundary | 12 |

Other answers this session, recorded here so they are not lost:
- Connectivity: n0 stays the default. Bare node-ID lookup stays on. Tickets are yes. No shared token now.
- Groups above 1,000: no gate. Local warnings only.
- from_join fix: fork in developing-today, not pushed.
- serde_bytes: local only. No upstream issue or PR. Existing issues checked: #770 (proposed for `Body`, closed) and #1059 with PR #1058 (a `Topic` fix). No issue covers the two DataScheme `Vec<u8>` fields.

## New open questions

Each has a preferred answer and alternatives. These are my recommendations, not decisions.

| # | Question | Preferred | Alternatives | Task |
| --- | --- | --- | --- | --- |
| N1 | Storage engine for the event store | redb: writes 93 µs vs 187 µs, lookups 24 µs vs 57 µs, but files 2.8× larger | Turso (not measured; the S6 harness can test it); libsql (C fork) | 02 |
| N2 | Identity database (web, libsql) | Keep libsql until N1 is settled | Move to redb with one engine (rewrite the identity tables; data can be discarded) | 02, 12 |
| N3 | p2panda-store | Implement its traits over redb if they fit our schema (S7 tests it) | Our own store API (no p2panda-sync); fork p2panda-store (rejected: keeps SQLite) | 02 |
| N4 | Chain identity in headers | ChainExt on every event from day one | One log per group (fewer logs; order moves into the body) | 02 |
| N5 | Sync method per session | Range sync above about 40 chains; LogSync below (confirm the crossover in S7) | Range sync always; LogSync always | 04 |
| N6 | Range parameters on real stores | K=16, T=16 as simulated in S1 | Values chosen from S7 | 04 |
| N7 | Periodic key rotation | Off; rotate on removal and on explicit update | Rotate every N events, or weekly | 05 |
| N8 | Recovery after a device key compromise | Manual remove and re-add, as a CLI command | Automatic on a detected key change | 05 |
| N9 | from_join membership fix | Keep the fork (decided); revisit moving the fix into our DGM later | Our DGM's `from_welcome` inserts the joiner, no fork | 05, 11 |
| N10 | serde_bytes on the two DataScheme fields | A new scheme version, since it is wire-incompatible (S12): 40.0 B vs 146.6 B per epoch | Stay on stock encoding until a migration window exists | 05, 11 |
| N11 | Warning thresholds | 96, 128, 1,000 | Other values, set from S12 costs | 05 |
| N12 | BeeKEM scope and migration | New groups above 1,000 use BeeKEM; existing groups stay on DataScheme until migrated; both supported in a transition window | All new groups on BeeKEM; in-place conversion, if the BeeKEM spike finds one | 05 |
| N13 | Power-loss testing for storage | Run LazyFS or a VM power cut before choosing the production engine | SIGKILL only (what S6 did) | 02 |
| N14 | p2panda source pin | Stay on the git rev | crates.io 0.7.x with patches | 12 |
| N15 | Toolchain step | 1.99.0 in one step (nixpkgs has it) | Stay at 1.97.0 until the next dependency pass | 12 |
| N16 | `welcome` `recent:N` default N | 32 epochs (about 4.7 KB today; confirm in S12) | 8; 1 | 05 |
| N17 | `store.rs` doc comment | Fix it: FsStore is redb, not SQLite | none | code |
| N18 | Observe mode default | Covered by #5 | none | 04 |
| N19 | Accounts for our Tunnel relay | You create and provide them; nothing is created on your behalf | Use n0's relays only until then | 08 |
| N20 | Scheme version for serde_bytes | A new DataScheme scheme version with negotiation, since the change is wire-incompatible | Stay on stock encoding and accept 3.7 times larger welcomes | 05 |
| N21 | `recent:N` implementation path | Call `Dcgka::add` directly from our adapter (`EncryptionGroup::add` always sends the full bundle) | Patch the high-level add to take a history bound | 05 |
| N22 | Heads orderer rule | Cap 32, always name the author's own latest event, merge rule on overflow | Cap 16 (162 merges per storm) or cap 2 (3,159 merges); a designated merger skips about a quarter of storm events | 05 |
| N23 | Seitan freshness window | `created − 5 min ≤ ctime ≤ min(now + 5 min, expires)` | A wall-clock window only; or accept any `ctime` up to expiry | 07 |
| N24 | Trust in tickets | Tickets are unsigned address hints. Identity comes from signed events and the invite event | Sign tickets in a new format | 07, 08 |
| N25 | Relay operation defaults | `KillSignal=SIGINT`; metrics bound to localhost; fail closed when the config file is missing | Keep SIGTERM (a hard kill) and rely on restart policy; not recommended | 08 |
| N26 | BeeKEM for groups above 1,000 | Do not adopt now. DataScheme for all groups, with warnings. Revisit when the crate has a join API and authority checks | Fix and extend BeeKEM ourselves (large); a scheme switch at a barrier later if continuity matters | 05 |
| N27 | Range sync parameters for bulk catch-up | K=16, T=16 normally; K=4, T=8 for bulk catch-up (saves 28 to 33% of control at 200 new events) | K=16 for everything | 04 |
| N28 | Migrating existing DataScheme groups to a new scheme | A scheme switch at a barrier inside the same group | A new group with re-invites | 05 |
| N29 | Source of QAD for direct paths | Keep n0 in the default map for QAD, and verify direct paths on real networks (C2 could not reach n0). Add our own QAD relay (option B) later if capacity or privacy needs it | Tunnel only, which gives no QAD and so no direct paths beyond n0; a VPS with QAD first | 08 |
| N30 | Address privacy from connected peers | Document that a connected peer learns our addresses (C2). Tickets stay address-free for lookups | Authorize peers before connecting (invite first), so only trusted peers connect | 07, 08 |
| N31 | Relay capacity for fallback pairs | Plan relay capacity for symmetric-to-symmetric and symmetric-to-restricted pairs, which fall back (C2) | Accept lower direct-path rates and tune relay limits | 08 |
| N32 | Startup wait on relays | Wait only for the home relay, with a deadline. Do not wait on every map entry (dead entries cost about 3 s) | Wait for every entry | 08 |

## Phases (draft; I will revise them after the spikes report)

- **Phase 0. Fixes and dependencies.** Tuple fix (done). Task 01's remaining items. Task 12 (toolchain and dependencies), before storage work so the storage code starts on current crates.
- **Phase 1. Events and storage.** ChainExt, canonical CBOR, the redb event store (N1–N4), fork freeze and proof, waiting caps (#6). Gate: S7's correctness checks pass on the redb store.
- **Phase 2. Membership and delegation.** Auth-frontier shim, admission gate, friendship, delegation modes, world capabilities (#1–#4). Gate: S2's scenarios pass on the real store.
- **Phase 3. Sync and blobs.** Range sync over iroh (N5, N6; S7 done, K=16 and T=16 kept), blob gating and observe (#5). Gate: S4 gating tests, still to write.
- **Phase 4. Group encryption and history.** DataScheme adapter, our DGM, from_join (N9), serde_bytes as a new scheme version (N20), `recent:N` through `Dcgka::add` (N21) and lazy `full` (#7), the heads rule (N22), warnings (N11), rotation policy (N7, N8). BeeKEM is not adopted now (N26). Gates: S10 and S12 are done; their rules are in task 05.
- **Phase 5. Tags, documents, search.** Task 06. Gate: S8.
- **Phase 6. Invites and pull.** Task 07. Gate: S9.
- **Phase 7. Connectivity extensions.** Our Tunnel relay (N19), tickets, access control at implementation (#13), mDNS and DHT defaults (#14, #15), QAD source (N29), startup waits (N32). Gates: S11 runbook (done); C2 (done in namespaces). Still needed: a real-network test with two separate networks and n0's QAD.
- **Phase 8. Later.** Time and witnesses (task 09), retention, multi-device, fork recovery (task 10).

## Known discrepancies in the current code

- `serve` enables mDNS by default. The connectivity design says off (decision 14).
- Pull pages are capped by item count, not bytes (task 01).
- `pkgs/id` declares Rust 1.91. p2panda and the spikes need 1.96 (task 01, task 12).
- `src/store.rs` says FsStore is SQLite. It is redb (N17).
- The connectivity design says peers re-home in 20–22 s. Three runs measured 20.5, 21.6 and 22.5 s, so it should read 20–23 s.
- `spikes/shared/c1-connect/RESULTS.md` lines 141 and 144 are stale. t2 and t4 were re-run; see `spikes/c1-rerun`.
- The Roc test, the world-module hang, and three `world_compile` tests are pre-existing failures. The last three need `host.wasm`.
- Clippy: a target dir must use one rustc. Mixing the Nix rustc with the rustup clippy driver gave E0514 errors on 2026-10-11. Run clippy through the 1.97.0 toolchain with `RUSTC` pinned. With `-D warnings` it reports 141 pre-existing lints crate-wide. None fall on lines changed for the tuple fix.
- The S3 "patched" copy has no serde_bytes change. The serde_bytes patch does not exist yet (N10).
- The kit's systemd unit uses `KillSignal=SIGTERM`, which hard-kills the relay. Use `SIGINT` (N25).
- A missing relay config file makes the relay silently start on `[::]:80`, with metrics on `[::]:9090` (N25).

## Evidence

- Spike code and results: [spikes](../2026-10-11T03-44-33Z_design_groups_events_delegation/spikes/README.md). New spikes are copied into the same folder when they report.
- Research notes: [research](./research/README.md).
- Connectivity kit: [relay configs](../2026-10-11T03-44-33Z_design_groups_events_delegation/connectivity/relay/).
