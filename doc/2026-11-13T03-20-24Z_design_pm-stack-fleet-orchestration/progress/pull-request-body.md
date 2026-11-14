## Summary

```text
PM/Fleet source packet (#9196, task 15381)
  → source statement inventory
  → pinned census + requirement/command crosswalk
  → prior art + versioned domain designs
  → proposed FDRs + granular, dependency-linked issues
  → independent coverage/architecture review
```

Preserve both tracks; extend existing Fleet/PM foundations, never replace them.
This PR is the documentation batch for the active mill. Related: #9196, #9198–#9209.
It does **not** close the program or authorize production implementation.

## Evidence

- **Before:** preserved sources and domain starters, without an atomic crosswalk or component build-out.
- **After:** inspect the committed census, validation receipts, and coverage audit in
  `doc/2026-11-13T03-20-24Z_design_pm-stack-fleet-orchestration/` as the batch lands.
- Census pin: `a038f084dbeb525494d7bb928ad91f597db2ac51`.
- Worker preflight passed; all source reads are from this worktree. Validation and
  reviewer results will be recorded before merge; no passing checks are claimed yet.

## Merge Danger

**Door:** two-way

Design/research/traceability files only. No production queue, dispatch, schema, binary,
database, or resolver changes. New implementation remains separately queued work.

**Blast Radius:** documentation
