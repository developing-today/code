# AUDIT-7 merged-PR audit — batch 3 continuation (#7654–#7659)

## Brief

Continue the AUDIT-7 Lane 4 batch 3/14 audit for `developing-today/code`: read each selected PR's body and complete diff, compare it with the spawning issue and comments, check later merged work for any remainder, apply one Lane 4 outcome label, and record the ledger on #13879. This slice covers PRs #7654, #7656, #7657, #7658 and #7659.

## Method and evidence

- Read each complete PR body and every changed-file response. File counts match metadata: #7654 (2), #7656 (1), #7657 (5), #7658 (2), #7659 (1); all patches are present.
- Read each PR conversation comment set and the complete source issue comment sets. Counts match metadata: PRs #7654 (2), #7656 (7), #7657 (2), #7658 (2), #7659 (3); issues #6061 (17), #6046 (4), #4837 (78), #7360 (8), #5502 (63).
- Checked later work touching the event-store sync path and the backup implementation. PR #11265's complete diff adds non-empty device-operation body delivery coverage but leaves #7657's absent-vs-empty framing unchanged. The backup follow-ups #8953, #9020, #10916 and #12401 correct later-found edge cases in #7658's implementation. PR #10002's complete record supplies the post-#7659 benchmark result required by #5502.
- For #7657's present-empty case, current `id-store::chain::sign` accepts `Some(&[])`: it builds a header from the body without rejecting zero bytes. Current main still has `decode_record` map an empty body suffix to `None`. No existing issue for that bug was found. Filed issue #14907, enqueued task 23710 in `repo-health-code-pilot`, labelled #7657 `audit:bug-filed`, and posted the suspect-closure pointer to Lane 2 on #13871; #4837 was not reopened.
- Read back one audit outcome label on every PR. All source responses, the follow-up issue/task, and audit comments are in `raw/`.

## Per-PR ledger

|    PR | Merged     | Spawning issue | Verdict                 | Finding                                                                                                                                                                                                                                                                                                                                                                                                                                                 |
| ----: | ---------- | -------------: | ----------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| #7654 | 2026-11-08 |          #6061 | `audit:complete`        | The design record contains the churn × retention model, per-source inputs, hot/cold and staging projection, worked example, ledger calibration loop, conservative defaults, and no-destructive-action guardrails required by #6061. The three implementation follow-ons are separate from the design acceptance.                                                                                                                                        |
| #7656 | 2026-11-16 |          #6046 | `audit:complete`        | The design record covers the golden-set schema/versioning, feedback-promotion and confirmation flow, change-class gate contract, metrics and blocking deltas, and stale-entry maintenance requested by #6046.                                                                                                                                                                                                                                           |
| #7657 | 2026-11-13 |          #4837 | `audit:bug-filed`       | The merged change persists and retrieves event bodies and carries them through range sync. A present empty body is accepted by ingest but encoded identically to an absent body; the receiver maps it to `None` and loses it. Filed #14907 and enqueued task 23710. PR #11265 later adds a non-empty body sync test but does not fix this framing. Closure pointer: #13871 comment 7155318337.                                                          |
| #7658 | 2026-11-08 |          #7360 | `audit:completed-later` | The PR implements the record, epoch-guarded upload/delete, budgeted download, restore bookkeeping, client helpers and acceptance tests. Later merged fixes completed the edge-case work: #8953 enforces the passphrase floor, #9020 blocks new bindings during pending restore, #10916 validates before signing, and #12401 preserves the generation floor after tombstone purge. The links are recorded on PR #7658.                                   |
| #7659 | 2026-11-10 |          #5502 | `audit:completed-later` | The PR moves journal `write_all` + `sync_data` to a dedicated writer thread and preserves append/compaction ordering. It did not carry the required post-change measurement. Later PR #10002 records p99 4,549.272 ms versus 5,650.792 ms (−19.5%), checkpoint/evict/reopen 100/100/100, and the 5,443.164 µs journal-append baseline; target remains above 50 ms and the next group-commit work is tracked by #8239. The link is recorded on PR #7659. |

## Later-work and filing check

Issue #14907 captures the newly found PR #7657 empty-body bug; it is live in `repo-health-code-pilot` as task 23710. No task was filed for #7658's already-merged fixes or #7659's later benchmark record. PR #10002 records the still-missed p99 target and the next step; #5502's completed lane record cross-references the measurement. No issue was reopened.

## Slice state

This slice adds five audited PRs to the prior 15: **20/250 audited; 230 remain; batch 3 remains partial**. The next PR in the fixed payload list is #7660. All five outcome labels were read back. The batch ledger for all 250 PRs is not complete.

## Raw supplements

`raw/` retains the five audited PR bodies, full changed-file responses, comments and label readbacks; source issue bodies and complete comment sets; current range-sync and chain signing sources; later PRs #10002 and #11265; the #7064 measurement record; backup follow-up PRs #8953, #9020, #10916 and #12401; and issue/task/comment readbacks for #14907 / 23710.
