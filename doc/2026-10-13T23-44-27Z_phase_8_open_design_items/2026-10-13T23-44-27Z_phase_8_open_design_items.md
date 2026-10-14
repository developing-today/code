# Phase 8: open design items, retention, multi-device, and views

- Plan: [phased plan](../2026-10-11T11-08-42Z_design_remaining_work/plan-phases.md), section 12.
- Start commit: d4e99105 (origin/main, after Phase 7 and #205).
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
| P8.2 | pruning     | done     | 678e9e63, a3c82d7c | retention_pruning: 4; event_store: 92              | retention behind causal-cut snapshot                               |
| P8.3 | multidevice | done     | 862639b4, 76f5084d | multidevice: 6 (22 red compile errors before feat) | device chains & account delegations; cascade revocation & counting |
| P8.4 | views       | done     | 1d8321f1, babadcb4 | views_explorer: 6; directory_view: 21              | directory explorer waiting, voided, frozen sections                |
| P8.5 | worlds      | done     | in P2.7            | groups::worlds: 9                                  | cross-world delegation reconciliation                              |
| P8.6 | warnings    | done     | 1289cef6, 472d45d3 | cli_warnings 3; cli_groups 8; warnings 12          | size tier and churn warnings in group status                       |
| P8.7 | -           | skipped  |                    |                                                    | owner confirmed: skip BeeKEM                                       |
| P8.8 | -           | deferred |                    |                                                    | do nothing now                                                     |

## 2026-10-13T23:55:00Z Note: P8.6 warnings UI in group status and list

- Group size tier and churn warnings are wired into `id group policy` and `id group list`.
- Size tiers: tier=0 for <96 members, tier=96 for 96..127, tier=128 for 128..999, tier=1000 for >=1000 members.
- Crossing warnings from `Evaluator` are formatted via `Warning::message()`.
- Text mode includes `tier=<usize>` and optional `warning=<message>`.
- JSON mode (`--json`) includes `"tier": usize` and optional `"warning": string`.
- Red first: integration tests in `tests/cli_warnings.rs` failed initially (commit 1289cef6) and passed after wiring (commit 472d45d3).
- Targeted test suite passes: `cli_warnings` (3 tests), `cli_groups` (8 tests), `warnings` (12 tests).

## 2026-10-13T23:55:00Z Note: P8.4 views for waiting, voided, and frozen events

- Extended `DirectoryView` and `render_text` in `src/directory_view.rs` to support inspecting event state:
  - Waiting events: section `waiting:` listing event ID, missing dependency hash, and reason formatted as "waiting: missing <hash>".
  - Voided events: section `voided:` listing event ID, voiding authority/removal hash, and reason formatted as "voided: concurrent with <hash>".
  - Frozen chains: section `frozen:` listing author/event ID, fork sequence/proof hash, and reason formatted as "frozen: fork at seq <n>".
- Shared types `WaitingView`, `VoidedView`, and `FrozenView` keep HTTP JSON serialization and SSH text rendering in step.
- Red first: integration tests in `tests/views_explorer.rs` failed on missing fields and types (commit 1d8321f1). Green after implementation (commit babadcb4).
- Tests: `views_explorer`: 6 passed; `directory_view`: 21 passed.

## 2026-10-13T23:55:00Z Note: P8.3 Multi-device delegation and device counting

- Each device has its own key and chain. An account authorizes devices through signed `device.add` (`DEVICE_ADD_KIND`) and `device.revoke` (`DEVICE_REVOKE_KIND`) operations (`DeviceOp`).
- Device authorizations are structured as capability delegations from the account key: a device acts on behalf of the account for authorized groups and scopes.
- When an account is removed or revokes a device via `device.revoke`, the device's authorization is severed following cascade rules.
- Device counting helpers `active_device_count(&group, &account)` and `group_device_count(&group)` (available on `Projection` and `GroupLog`) accurately return active authorized device counts for an account and across a group.
- Red first: `cargo test --test multidevice` failed with 22 compilation errors in `862639b4`. Green after `76f5084d`: 6 of 6 integration tests pass in `tests/multidevice.rs`; all 12 `delegation` tests in `src/lib.rs` pass.

## 2026-10-13T23:59:00Z Note: P8.2 retention and pruning behind causal cut

- Implemented `EventStore::prune_behind_cut(&mut self, cut: &[Hash], protected: &[Hash]) -> Result<usize, StoreError>`.
- Preserves every event back to the causal past of open removals (needed for membership voidness check) and open frontiers.
- Active backlink walks and dependency index invariants are preserved; author chain rows are adjusted or removed on total pruning.
- Pruned events are removed from `events`, `hashes`, `groups`, `actions`, and `applied` tables.
- Invariants check (`check_invariants`) and query folds (`roster`, `tag_claims`, `documents`) produce identical results before and after pruning.
- Red first: integration tests in `tests/retention_pruning.rs` failed initially (commit 678e9e63) and passed after implementation (commit a3c82d7c).
- Targeted test suite passes: `retention_pruning` (4 tests), `event_store` (92 passed, 2 ignored).

## 2026-10-14T00:00:00Z Acceptance: durable signed revisions for web saves

- Every successful manual save or autosave checkpoint publishes the saved bytes as a blob and records an applied, node-signed `document.put` revision naming its BLAKE3 hash, byte size, and media type, plus a `filename` tag. Each save is a separate revision; historical blobs/events remain available. The event ID is returned to the caller. Legacy filename blob tags remain for existing file views.
- ProseMirror steps remain ephemeral in-memory collaboration transport; only save/autosave checkpoints become durable signed revisions.
- Save acceptance requires tests for applied signed record metadata, filename claim, blob retrieval, distinct repeated revisions, and an error response rather than success when publication fails.
