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

| Step | Track       | Status   | Commits            | Tests                                     | Notes                                        |
| ---- | ----------- | -------- | ------------------ | ----------------------------------------- | -------------------------------------------- |
| P8.1 | -           | deferred |                    |                                           | causal validity only (task 09)               |
| P8.2 | pruning     | todo     |                    |                                           | retention behind causal-cut snapshot         |
| P8.3 | multidevice | todo     |                    |                                           | device chains & account delegations          |
| P8.4 | views       | done     | 1d8321f1, babadcb4 | views_explorer: 6; directory_view: 21 | directory explorer waiting, voided, frozen sections |
| P8.5 | worlds      | todo     |                    |                                           | cross-world delegation reconciliation        |
| P8.6 | warnings    | done     | 1289cef6, 472d45d3 | cli_warnings 3; cli_groups 8; warnings 12 | size tier and churn warnings in group status |
| P8.7 | -           | skipped  |                    |                                           | owner confirmed: skip BeeKEM                 |
| P8.8 | -           | deferred |                    |                                           | do nothing now                               |

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
