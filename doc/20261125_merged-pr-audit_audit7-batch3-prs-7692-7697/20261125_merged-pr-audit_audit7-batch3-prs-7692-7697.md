# AUDIT-7 merged-PR audit — batch 3 continuation (#7692–#7697)

## Brief

Continue the AUDIT-7 Lane 4 batch 3/14 audit for `developing-today/code`: read each selected PR's body and complete diff, compare it with the spawning issue and comments, check later merged work for any remainder, apply one Lane 4 outcome label, and record the ledger on #13879. The five payload entries at this boundary are #7692, #7693, #7694, #7696 and #7697.

## Method and evidence

- Verified each identifier against the issue/PR endpoint. #7693 and #7694 are closed issues, not PRs: their issue objects have no `pull_request` field. No PR body/diff or PR label exists for those payload entries.
- Read complete PR bodies and changed-file responses for #7692 (4 files), #7696 (4 files) and #7697 (8 files); each file count matches PR metadata and all patches are present. Read each PR comment set: #7692 (1), #7696 (2), #7697 (4).
- Read full source issue bodies and comments: #5766 (31), #6904 (69), #7016 (77). For #7692's remaining acceptance, read the merged PR #7886 body and complete file set (5 files, 8 comments); it implements the real activation slice and reports the laptop E2E acceptance.
- For #7697, the PR body states that the first-run acceptance was pending deployment: the units were not installed and the running broker returned `unknown command /coverage`. Current `systemctl --user` readback is `not-found` / `inactive`. No later PR or live queue task for that deployment/first-run acceptance was found. Filed issue #14929 and enqueued task 23719 in `repo-health-code-pilot`.
- Applied and read back one outcome label on each actual PR. The two invalid payload identifiers are recorded separately without a PR label.

## Per-entry ledger

| Payload number | Live object | Spawning issue | Verdict | Finding |
|---:|---|---:|---|---|
| #7692 | Merged PR (2026-11-08) | #5766 | `audit:completed-later` | This slice delivers rendering, dry-run plans, generation log, status and rollback planning, while explicitly leaving real activation open. Later merged PR #7886 implements systemd activation, health-check flip-back, rollback and the laptop E2E acceptance. The link is recorded on PR #7692. |
| #7693 | Closed issue, not a PR | — | `invalid-reference` | The payload number is a closed issue about symlink-loop error handling; its issue object has no `pull_request` field. No PR exists under this number to compare or label. |
| #7694 | Closed issue, not a PR | — | `invalid-reference` | The payload number is a closed issue about surrogate-pair truncation; its issue object has no `pull_request` field. No PR exists under this number to compare or label. |
| #7696 | Merged PR (2026-11-10) | #6904 | `audit:complete` | A single parser now handles MCPX_TEST_REQUIRE for binaries and real servers, and suite-level enforcement covers every listed entry, including `python3`. Tests pin case handling and the required-entry behavior. |
| #7697 | Merged PR (2026-11-09) | #7016 | `audit:gap-filed` | The PR adds the broker `/coverage` path, observe/import reconciler, timer units and registry row, but its own body leaves first-run acceptance pending: units not installed and running broker did not yet serve `/coverage`. Current timer is still not found/inactive. Issue #14929 / task 23719 now tracks deployment and a verified first run; closure pointer: #13871 comment 7156656492. |

## Slice state

This boundary processed five payload entries: three actual PRs audited and labelled, plus two invalid issue-number references. Across all slices so far, **41 valid PRs are audited from 45 processed payload identifiers**; #7667, #7668, #7693 and #7694 are issues, not PRs. There are 205 payload identifiers remaining; the next is #7698. Batch 3 remains partial pending reconciliation of the invalid entries and completion of the remaining audit.

## Raw supplements

`raw/` contains the three audited PR bodies, full changed-file responses, PR comments and label readbacks; source issue bodies and complete comment sets; the later #7886 PR body/files/comments; the invalid issue-number records; the follow-up #14929 / task 23719; and the tracking/umbrella snapshots.
