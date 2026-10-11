# Task 05: Encryption, history, warnings, and scheme

- Status: Designed and spiked (S3). S10 (heads orderer) and S12 (welcome size) are open. The history option and the large-group gate need decisions.
- Index: [what is left](./2026-10-11T11-08-42Z_design_remaining_work.md). Depends on: [task 02](./task-02-events-and-storage.md), [task 03](./task-03-membership-and-delegation.md) (membership at a causal position).
- Design: [design §8](../2026-10-11T03-44-33Z_design_groups_events_delegation/2026-10-11T03-44-33Z_design_groups_events_delegation.md), [review §3.5](../2026-10-11T03-44-33Z_design_groups_events_delegation/2026-10-11T08-08-56Z_design_review.md). Evidence: S3 in the [spikes](../2026-10-11T03-44-33Z_design_groups_events_delegation/spikes/README.md).

## Goal

Every group is end-to-end encrypted by default, using p2panda-encryption's DataScheme. An explicit `scheme: none` opts out. History, rotation, and the size warnings follow the owner's decisions.

## Scope

1. **DGM.** Our group-membership implementation. Membership at a causal position comes from [task 03](./task-03-membership-and-delegation.md). The welcome path must insert the joiner, because the upstream `process_welcome` overwrites the joiner's membership (S3).
2. **Ordering adapter.** Name the heads, not the per-author latest. When heads would exceed 32, publish a merge update naming up to 32 of them. S3 measured 8, 40, and 1 heads in three cases. The heads orderer is untested (S10).
3. **from_join.** The welcome carries an empty bundle, built through the public `Dcgka::add` API (about 20 lines). Then an immediate `update` rotates the key. The joiner withholds pre-join application messages. Without this, the welcome fails with `UnknownGroupSecret` and the joiner's state is lost (S3 c1).
4. **History.** `full` is the default and is native: the welcome carries every epoch secret. The cost grows with each rotation, at about 146 B per secret in CBOR today. Options, pending the decision: `recent:N` for large groups, and the upstream `serde_bytes` fix (roughly 40 B per secret, an S3 estimate).
5. **Rotation and compromise.** Removal and update rotate the group secret. This heals removed members and anyone who holds only the group secret. It does not heal a member device whose own keys are compromised, which needs remove and re-add (S3 a). Document this where the design says PCS.
6. **Scheme.** The scheme and version live in the signed group policy, changed by a manage operation. Key bundles advertise supported schemes. A scheme change starts a new epoch. Readers keep a decoder for every scheme that has ever produced an epoch. A message whose scheme differs from its epoch's is rejected.
7. **`scheme: none`.** An explicit opt-out, chosen at creation and recorded in the policy. Members see it as server-readable.
8. **Warnings.** End-to-end groups only, counted in recipient devices. Above 96: warning. At 128 or more: stronger warning (128 is p2panda's figure). At 1,000 or more: strongest warning. Non-blocking, with no caps. Shown in group status (CLI, web, explorer) and whenever an action crosses a tier.
9. **Pre-keys.** The default lifetime is 84 days in the crate, and spaces configures 90. Existing sessions survive expiry. First contact with an expired bundle fails with `KeyBundlesExpired`. Decide which value we use.
10. **Upstream reports.** File the `dcgka.rs` membership overwrite (with the 8-line, 2-changed patch in `spikes/shared/s3-history/logs/patch.diff`), the missing `serde_bytes`, the orderer's per-author latest with no pruning, and `receive` discarding state on error. See [task 11](./task-11-dependencies-and-upstream.md).

## Decisions

- Joiner history default: `full` (decided). `recent:N` is added per world (decided).
- Large-group gate: end-to-end groups above 1,000 devices wait for a BeeKEM spike (recommended).
- `scheme: none` notice wording.

## Acceptance

- The S3 scenarios run against our adapter, not the test utilities: full history reads the old message, from_join does not, PCS holds for removal, and a leaked-secret case recovers after an update.
- S10: a 40-member group with concurrent updates and a merge at 32 converges for every member.
- S12: welcome size at 1,000 rotations, recorded with and without the `serde_bytes` patch.
- Warnings: the tier thresholds are tested with recipient device counts, including members with several devices.
- A downgrade test: a message with the wrong scheme is rejected.

## Gotchas

- `add` mints no secret. Only `create`, `update`, and `remove` do. "A new epoch on each membership change" is wrong for adds.
- The crate's `Ordering` names one dependency per author and never prunes. Once 33 members have each published, every later message names more than 32.
- `serde_bytes` is missing on `TwoPartyPlaintext.plaintext` and `X3dhCiphertext.ciphertext`, so every byte costs one or two.
- Adds do not rotate keys in spaces either (`EncryptionGroup::add`).
- Promote and Demote panic in p2panda-spaces 0.7.1. Do not use spaces yet.

## Files

New module, for example `pkgs/id/src/encryption/`. Group state lives with the events (task 02).

## Dependencies

p2panda-encryption (git, pinned), with the `data_scheme` feature. Do not enable `test_utils` in product code. See [task 11](./task-11-dependencies-and-upstream.md).

## 2026-10-11T12-20-38Z Update: rotations, welcome, warnings, membership bug

- Rotation triggers in the crate: create mints a secret; removal rotates (required); explicit update rotates; add does not rotate, because the welcome carries the existing secrets. Our from_join adds an update per join.
- Welcome size: about 146 B per epoch today. My estimate with serde_bytes is about 40 B; S12 measures it. Growth is linear in epochs.
- History (#7, N16): superseded by the decision in the update below. `full` is the default, and `recent:N` is an option per world, with presets 32 and 128. `full` on request through a lazy history protocol, a new item. Alternatives: `full` always; forward-only.
- Large groups (#9): no gate. Servers emit local warnings. Preferred tiers 96, 128, 1,000 (N11). BeeKEM migration (N12): preferred is new groups above 1,000 on BeeKEM, with a transition window. Pending the BeeKEM spike.
- from_join (N9): fork in developing-today, not pushed upstream (owner decision). Alternative: our DGM's `from_welcome` inserts the joiner, with no fork.
- serde_bytes (N10): local patch only. Upstream: #770 (closed; proposed for `Body`) and #1059 (closed; a `Topic` fix in PR #1058). No issue covers the two DataScheme `Vec<u8>` fields.
- Membership bug: `data_scheme/dcgka.rs` `process_welcome` calls `DGM::from_welcome` after the add. With the stock `TestDgm`, the welcome's copy overwrites the joiner's membership. The crate's tests do not catch it. Our production DGM's `from_welcome` appears to return its input unchanged. That is from reading the source, not tested. Not retested, per the owner.
- PCS, narrow form: an update or removal heals a leaked group secret. It does not heal a member device whose own keys were copied. Recovery is remove and re-add (N8).
- Pending: S10 (heads orderer), S12 (welcome sizes), BeeKEM.

## 2026-10-11T12-49-09Z Update: welcome size (S12), heads orderer (S10), BeeKEM spike

Welcome size (S12):
- Stock: 146.6 B per epoch, 147.4 KB at 1,000 epochs (mean of five runs). Encoded with `p2panda_core::cbor::encode_cbor`, which is cbor-core 0.10.1.
- serde_bytes variant: 40.0 B per epoch, 40.5 KB at 1,000 epochs. This is S12's own annotated copy. The S3 "patched" copy has no serde_bytes change, and S3's RESULTS say it was never built. Correct the earlier claim: the patch does not exist yet.
- The serde_bytes fix is wire-incompatible. A PreKey welcome fails to decode across the two builds in both directions. Ship it as a new scheme version, not a silent patch (N10 changes accordingly).
- Re-adding a member is cheaper: about 78 B per epoch, because it reuses the existing HPKE session. A new joiner costs 147 B per epoch.
- Churn traffic is quadratic: removing and re-adding members at K=1,000 sends 19.5 MB (stock) and 10.2 MB (serde_bytes) of welcomes in total.
- `recent:N` works through the public API, but only through `Dcgka::add` directly. `EncryptionGroup::add` always sends the full bundle. Sizes at K=1,000, stock / serde_bytes: N=1 747 B / 489 B; N=8 1,775 B / 771 B; N=32 5,323 B / 1,732 B. The joiner learns exactly those N secrets (60 of 60 checks).
- Stock welcome bytes vary by 120 to 170 B between runs with the same seed, because of wall-clock timestamps and `HashMap` order. Never re-encode a stored welcome to check it.
- Time at K=1,000: about 1.8 ms to build and 2.0 ms to encode. Size is the constraint, not time.
- Not tested: receiving across the group (Bob and Carol never process messages); joiner decryption of application messages; groups above three members; K above 1,000.

Heads orderer (S10):
- The per-event cap of 32 holds with the merge rule. No event named more than 32 heads.
- The frontier does not stay at 32. At 40 members a burst gives 40 heads (one per member). The merge rule is needed at a cap of 32, and not needed at 40 or more.
- Concurrent merges cannot name each other. A member's view grows by 8 per storm round (40, 48, 56). A designated merger keeps the view at 40, but skips about a quarter of storm events.
- Adversarial delivery order: caps 2 to 24 fail with HPKE `OpenError`, unless each event also names the author's own latest event. With that rule every cap converges. Cap 32 without it converged in two orders only.
- Recommended: cap 32, the merge rule, and always naming the author's own latest event. Cap 2 converges too, but costs 3,159 merges and about 1,150 waiting messages for three storms. Cap 16 costs 162 merges. Cap 32 costs the storm's own 81 merges.
- The crate's orderer keeps one dependency per author and never prunes. Messages with missing dependencies wait indefinitely; in one lost-update run a member stalled for the rest of the run.
- Not tested: membership changes, real transport, byte sizes, group sizes other than 40.

BeeKEM (beekem 0.4.0, MSRV 1.90):
- Steady-state update: 2.8 KB at 100 members, 3.9 KB at 1,000, about 5.0 KB at 5,000 (extrapolated). A remove plus the root-restoring update is about 5.3 KB at 1,000. Committer CPU is 0.6 to 0.8 ms per update.
- The first commit after a bulk build is O(n): 122 KB at 1,000 members. Per-member state is mostly operation history: 64 MB at 1,000 after a full fill, and nothing prunes it.
- Concurrency converges only with causal buffering. Dropping out-of-order operations converged in 1 of 300 trials. A concurrent commit leaves no root key until the next commit. A concurrent removal applies lazily, so membership reads are stale.
- Join: there is no welcome API. The only route is a replica copy: 0.74 MB at 1,000 right after creation, 64 MB after history. The copy carries the sender's secrets and every epoch key, so it must be stripped first. A fresh replica cannot start from the latest update.
- Forward secrecy: none. A later copy derives earlier roots. Post-compromise security after an in-group compromise holds only once the compromised member commits. A removed member cannot derive the new root.
- Bytes against DataScheme: one BeeKEM operation is 7 to 16 times DataScheme's bytes under unicast. BeeKEM wins only under fan-out, from about 11 to 17 members.
- The crate checks neither signatures nor authority, and an add accepts any key. In 11 of 300 random trials no original member was left, and the crate allowed it.
- Migration: converting DataScheme state in place is not possible. Options: a new group with re-invites; a scheme switch at a barrier inside the same group (recommended if continuity matters); a dual run; seeding the root from the old secret, rejected because the crate has no seed API.
- Recommendation: do not adopt BeeKEM now. Keep DataScheme for all groups, with the warnings from #9. Revisit when the crate has a join API and authority checks.

## 2026-10-11T16-08-10Z Decision: history, storms, merges, rotations, BeeKEM

- **Joiner history (decision 7, decided).** `full` by default. `recent:N` is an option per world, with presets 32 and 128. Lazy full-history requests are a Phase 4 item. The explainers, section 4, cover what 32 means and why not 128 or only `full`.
- **Heads and merges (N22, decided).** Cap 32; always name the author's latest event; merge when the heads would exceed the cap. Explainers, section 3.
- **Warnings (N11, N34, decided).** Thresholds 96, 128 and 1,000 members, and warnings on re-add churn as well as size. Warnings are local to the server. No group is blocked.
- **Rotations (N7, N8, decided).** No periodic rotation. Explicit updates rotate. Recovery from a compromised device is a manual remove and re-add.
- **from_join (N9, decided).** The fork stays in our developing-today fork (decision 18). Revisit moving the fix into our DGM later.
- **Scheme versions (N10, N20).** The serde_bytes encoding becomes a new scheme version, since it is wire-incompatible. Round 2 scheme-serde decides between the options; the explainers will give the full comparison.
- **BeeKEM (N12, N26, decided).** Not adopted now. The [assessment](./beekem-assessment.md) lists the difficulties and the conditions for reconsidering it.
- **DataScheme costs (new, P4.10).** Measure update cost at 100, 1,000 and 5,000 members with our adapter, for the BeeKEM comparison.
- Steps: [plan](./plan-phases.md), Phase 4.

## 2026-10-11T17-58-40Z Update: round 2 results

- The round 2 answers are in the [explainers](./explainers.md), the [index](./2026-10-11T11-08-42Z_design_remaining_work.md), and the [round 2 results](./research-round2/README.md).
- The steps this task feeds are in the [phased plan](./plan-phases.md). Each step names its blocker and its tests.
