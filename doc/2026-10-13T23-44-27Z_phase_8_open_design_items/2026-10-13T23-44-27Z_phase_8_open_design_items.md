# Phase 8: open design items, retention, multi-device, and views

- Plan: [phased plan](../2026-10-11T11-08-42Z_design_remaining_work/plan-phases.md), section 12.
- Start commit: e6a5123b (origin/main, after Phase 7 and #205).
- Base branch `phase-8`. Tracks:
  - `p8-warnings`: P8.6 warnings UI (show group size tier and churn warnings in group status/list views).
  - `p8-multidevice`: P8.3 multi-device (device keys authorized by account keys via `device.add` and `device.revoke`).
  - `p8-pruning`: P8.2 retention and pruning (retention behind causal-cut snapshot, respecting open removals and frontiers).
  - `p8-views`: P8.4 views (directory explorer lines in shared grammar).
  - P8.5 cross-world rules: reconciled with delegation (checked against P2.7 `src/groups/worlds.rs`).
- Explicitly skipped/deferred:
  - P8.1 time and witnesses (task 09, deferred; causal validity only).
  - P8.7 BeeKEM (owner confirmed: skip).
  - P8.8 clippy/lint sweeps (review later; do nothing now).

## Steps

| Step | Track       | Status   | Commits            | Tests                                              | Notes                                                              |
| ---- | ----------- | -------- | ------------------ | -------------------------------------------------- | ------------------------------------------------------------------ |
| P8.1 | -           | deferred |                    |                                                    | causal validity only (task 09)                                     |
| P8.2 | pruning     | todo     |                    |                                                    | retention behind causal-cut snapshot                               |
| P8.3 | multidevice | done     | 862639b4, 76f5084d | multidevice: 6 (22 red compile errors before feat) | device chains & account delegations; cascade revocation & counting |
| P8.4 | views       | todo     |                    |                                                    | directory explorer lines                                           |
| P8.5 | worlds      | todo     |                    |                                                    | cross-world delegation reconciliation                              |
| P8.6 | warnings    | todo     |                    |                                                    | size tier and churn warnings in group status                       |
| P8.7 | -           | skipped  |                    |                                                    | owner confirmed: skip BeeKEM                                       |
| P8.8 | -           | deferred |                    |                                                    | do nothing now                                                     |

## 2026-10-13T23:55:00Z Note: P8.3 Multi-device delegation and device counting

- Each device has its own key and chain. An account authorizes devices through signed `device.add` (`DEVICE_ADD_KIND`) and `device.revoke` (`DEVICE_REVOKE_KIND`) operations (`DeviceOp`).
- Device authorizations are structured as capability delegations from the account key: a device acts on behalf of the account for authorized groups and scopes.
- When an account is removed or revokes a device via `device.revoke`, the device's authorization is severed following cascade rules.
- Device counting helpers `active_device_count(&group, &account)` and `group_device_count(&group)` (available on `Projection` and `GroupLog`) accurately return active authorized device counts for an account and across a group.
- Red first: `cargo test --test multidevice` failed with 22 compilation errors in `862639b4`. Green after `76f5084d`: 6 of 6 integration tests pass in `tests/multidevice.rs`; all 12 `delegation` tests in `src/lib.rs` pass.
