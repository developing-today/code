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

| Step | Track       | Status   | Commits | Tests | Notes                                        |
| ---- | ----------- | -------- | ------- | ----- | -------------------------------------------- |
| P8.1 | -           | deferred |         |       | causal validity only (task 09)               |
| P8.2 | pruning     | todo     |         |       | retention behind causal-cut snapshot         |
| P8.3 | multidevice | todo     |         |       | device chains & account delegations          |
| P8.4 | views       | todo     |         |       | directory explorer lines                     |
| P8.5 | worlds      | todo     |         |       | cross-world delegation reconciliation        |
| P8.6 | warnings    | todo     |         |       | size tier and churn warnings in group status |
| P8.7 | -           | skipped  |         |       | owner confirmed: skip BeeKEM                 |
| P8.8 | -           | deferred |         |       | do nothing now                               |
