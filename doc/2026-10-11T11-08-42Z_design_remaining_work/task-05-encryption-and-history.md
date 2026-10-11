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

- Joiner history default: `full` (decided). Whether to add `recent:N` for large groups.
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
