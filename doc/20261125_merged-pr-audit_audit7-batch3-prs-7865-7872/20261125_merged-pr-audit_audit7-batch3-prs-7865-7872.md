# AUDIT-7 merged-PR audit — batch 3 continuation (#7865–#7872)

## Brief

Continue the AUDIT-7 Lane 4 batch 3/14 audit for `developing-today/code`: read each selected PR's body and complete diff, compare it with the spawning issue and comments, check later merged work for any remainder, apply one Lane 4 outcome label, and record the ledger on #13879. The five payload entries at this boundary are #7865, #7866, #7867, #7868 and #7872.

## Method and evidence

- Verified all five payload entries resolve to merged PRs. Read each full PR body, changed-file response and comments; counts match metadata: #7865 (1/2), #7866 (2/4), #7867 (9/7), #7868 (9/10), #7872 (1/1).
- Read source issue bodies and full comment sets, confirming #6164 (30), #3869 (23), #7084 (51), #3975 (28), and #5824 (31).
- #7866's empty-day watchdog check and test address acceptance items 1 and 3 of #3869. The completion note #6943183434 says target resolution remains hardcoded and does not show the written criterion's actual-container-list failure evidence. The issue is closed on that rationale; the Lane 2 closure pointer is #13871 comment 7158982068, and the PR cross-reference is comment 7158987720. Lane 4 did not reopen the issue.
- #7867's #7084 source issue records stable IDs/links, idempotent fixed-high-water ingestion, outage/lag visibility, and bounded hot-list coverage; its completion note verifies those behaviors and tracks an unrelated generated-schedule test flake separately.
- Applied and read back `audit:partial` on #7866 and `audit:complete` on #7865, #7867, #7868 and #7872.

## Per-entry ledger

| Payload number | Spawning issue | Verdict          | Finding                                                                                                                                                                                                                                                                                                                                           |
| -------------: | -------------: | ---------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
|          #7865 |          #6164 | `audit:complete` | Amends the board v1 contracts: creator-chosen validated ID affirmed, SHA-256 plus exact size for board payload classes, and portable view receipt deferred with privacy rationale and local reproducibility inputs.                                                                                                                               |
|          #7866 |          #3869 | `audit:partial`  | Adds an alarm for an empty newest backup day and a simulated drill. The written target-resolution alternative requires label/project resolution or failure evidence with the actual container list; the completion note instead relies on corrected hardcoded names plus nonzero exit/watchdog alert. Closure pointer: #13871 comment 7158982068. |
|          #7867 |          #7084 | `audit:complete` | Consumes the GitHub hot list into the common activity stream with stable source IDs/links, durable high-water checkpoints, duplicate/restart handling, and visible lag/gap coverage boundaries. The issue completion note rechecks all four acceptance items.                                                                                     |
|          #7868 |          #3975 | `audit:complete` | Detects dependency cycles as named SCCs, distinguishes malformed/unavailable edges from valid external refs, preserves rank/count behavior, and routes eligible inline actions through the audited `/api/actions` path.                                                                                                                           |
|          #7872 |          #5824 | `audit:complete` | Records Redis as hot operational state with the regular store as system of record, journal-first writes, RPO-0 recovery/reconciliation, and invariants, while explicitly recording the #5823 dependency without inventing its port contract.                                                                                                      |

## Slice state

Five payload entries were processed: five actual merged PRs audited and labelled, with no invalid references. Four are complete; #7866 is partial with a closed-issue acceptance question passed to Lane 2. Across all slices to this point, **126 valid PRs are audited from 130 processed payload identifiers**; #7667, #7668, #7693 and #7694 are issues rather than PRs. There are 120 payload identifiers remaining; next is #7874. Batch 3 remains partial.

Tracking progress comment: [#13879 comment 7159080781](https://github.com/developing-today/code/issues/13879#issuecomment-7159080781); duplicate count post #7159129751 was also corrected.

## Raw supplements

`raw/` contains the five PR bodies, full changed-file responses, comments and label readbacks; five source issue bodies/comment pages; #3869's closure evidence; and the #7084 later-PR search plus #7285 overlap context.
