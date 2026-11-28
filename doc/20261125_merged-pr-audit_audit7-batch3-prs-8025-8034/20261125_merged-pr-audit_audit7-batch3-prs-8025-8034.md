# AUDIT-7 merged-PR audit — batch 3 continuation (#8025–#8034)

## Brief

Continue the AUDIT-7 Lane 4 batch 3/14 audit for `developing-today/code`: read each selected PR's body and complete diff, compare it with the spawning issue and comments, check later merged work for any remainder, apply one Lane 4 outcome label, and record the ledger on #13879. The five payload entries at this boundary are #8025, #8031, #8032, #8033 and #8034.

## Method and evidence

- Verified all five payload entries resolve to merged PRs. Read full PR bodies, changed-file responses, PR comments, inline review comments and reviews; counts are #8025 (2 files/5 comments, 12 inline review comments and 10 review records), #8031 (2/5), #8032 (4/5), #8033 (6/4), and #8034 (5/4).
- Read source issue bodies and full comment sets: #6534 (54 comments), #8182 (12), #8183 (13), #1066 (18), #6378 (24), #6218 (22), #5544 (25), and the #835 phase design record. Read later PRs #8188 (merged into the #8025 branch before its merge) and #8494 (phase 3 of #5544).
- #8025 fixes the dead-inhibitor child behavior for #6534. The Major/Minor review findings were addressed through stacked PR #8188 and shepherd commit `718b96d02e`; #8025 merged after the fixes, and #6534 closes with the final head.
- #8031 implements F12.9 versioned program snapshots, optional migration and probe-first rollback for #1066; the issue closes with the feature.
- #8032 adopts nextest but leaves several #6378 goals unmeasured or unfinished. A new issue #15079 and live queue task #23801 now track remote measurement of the remaining targets; the source closure pointer is #13871 comment 7162067831.
- #8033 implements probe-based UDS/TCP selection across the #6218 fleet client and broker paths, with tests for present, absent and stale sockets; the issue closes with the implementation.
- #8034 is phase 2 of #5544. Later PR #8494 implements phase 3 joined-session frames and closes the source issue.
- Applied and read back `audit:complete` on #8025, #8031 and #8033, `audit:partial` on #8032, and `audit:completed-later` on #8034. Posted the partial and later-completion notes.

## Per-entry ledger

| Payload number | Spawning issue | Verdict                 | Finding                                                                                                                                                                                                                            |
| -------------: | -------------: | ----------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
|          #8025 |          #6534 | `audit:complete`        | Detects dead `systemd-inhibit` children, re-engages loudly and reaps on release. Review follow-ups #8182/#8183 were resolved by PR #8188 plus shepherd fixes before #8025 merged and #6534 closed.                                 |
|          #8031 |          #1066 | `audit:complete`        | Adds versioned snapshots and an optional migration export; probe-first upgrades commit only after a successful state round-trip and preserve the old program on failure.                                                           |
|          #8032 |          #6378 | `audit:partial`         | Adopts nextest but does not measure or close all original unit/integration/E2E/VM targets. Follow-up #15079 and task #23801 cover remote measurement and routing; Lane 2 pointer: #13871 comment 7162067831. PR note: #7162274904. |
|          #8033 |          #6218 | `audit:complete`        | Selects UDS or TCP per broker role by configured preference and live socket probe, with same-call TCP fallback and coverage for socket-present/absent/stale cases.                                                                 |
|          #8034 |          #5544 | `audit:completed-later` | Implements phase 2 actor-authorized install/revoke. Later PR #8494 wires the joined-session frames (phase 3) and #5544 closes. PR note: #7162276253.                                                                               |

## Slice state

Five payload entries were processed: five actual merged PRs audited and labelled, with no invalid references. Three are complete, one partial, and one was completed by later merged work. Across all slices to this point, **216 valid PRs are audited from 220 processed payload identifiers**; #7667, #7668, #7693 and #7694 are issues rather than PRs. There are 30 payload identifiers remaining; next is #8040. Batch 3 remains partial.

Tracking progress comment: [#13879 comment 7162650298](https://github.com/developing-today/code/issues/13879#issuecomment-7162650298).

## Raw supplements

`raw/` contains all five PR bodies, changed-file responses, review evidence, source issue bodies/comment pages, later PRs #8188 and #8494, follow-up issue #15079 and task #23801 records, merged-PR searches, label readbacks, the Lane 2 closure pointer, and posted disposition comments.
