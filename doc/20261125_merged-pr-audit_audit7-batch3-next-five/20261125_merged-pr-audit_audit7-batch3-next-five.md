# AUDIT-7 merged-PR audit — batch 3 continuation (five PRs)

## Brief

Continue the AUDIT-7 Lane 4 batch 3/14 audit for `developing-today/code`. The task requires each merged PR to be read in full, compared with its spawning issue and issue comments, labelled with one Lane 4 outcome, and recorded on tracking issue #13879. This slice covers PRs #7638, #7639, #7640, #7644 and #7645 only.

## Method and evidence

- Read the complete PR bodies and fetched every changed-file record. The number of fetched file records matches each PR's `changed_files` count; all files are at most 100, so no diff page was omitted.
- Read all PR conversation comments; the fetched counts match each PR's `comments` count.
- For each PR with a spawning issue, read the full issue body and all comments. Fetched issue-comment counts match the live issue's count: #6036 (6), #6243 (22), #6017 (38), and #5904 (12).
- Compared the issue's done criteria and the merged PR's complete diff. No residual gap or bug was found that needed a new issue. The #6243 history was checked for the overlapping PR #7642; its session-record fix and tests were salvaged into #7639, and #7642 was closed as superseded.
- Applied and read back one Lane 4 outcome label on each PR. Raw API responses, including PR bodies, file diffs, issue bodies/comments and label readbacks, are retained in `raw/`.

## Per-PR ledger

|    PR | Merged     | Spawning issue | Verdict          | Finding                                                                                                                                                                                                                                                                                                                                                  |
| ----: | ---------- | -------------: | ---------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| #7638 | 2026-11-08 |          #6036 | `audit:complete` | The merged design record supplies the requested rolling hash and window, strong verification, minimum anchor run, span-reference/literal token contract tied to sibling #6035, per-source cost model, failure cases, and worked 17–241 example. The issue remains open by design; the PR uses `Refs` and satisfies its design-record done criteria.      |
| #7639 | 2026-11-08 |          #6243 | `audit:complete` | The merged implementation and the issue inventory cover all seven required dispositions, put the ref contract beside `isValidRef`, guard unsafe ref production/consumption, trace dropped candidates, and pin the bad-ref and valid-ref cases. The issue history confirms the overlapping #7642 fix was salvaged into #7639 before #7642 was superseded. |
| #7640 | 2026-11-08 |          #6017 | `audit:complete` | The merged design record provides the audience-by-surface observability matrix and append-only capability history while consuming the #5958 registry schema and keeping its field naming boundary explicit, as requested.                                                                                                                                |
| #7644 | 2026-11-16 |              — | `audit:no-issue` | Renovate security update for `wrangler`; no spawning issue is referenced in the PR body. The dependency update is judged on its stated intent.                                                                                                                                                                                                           |
| #7645 | 2026-11-08 |          #5904 | `audit:complete` | The merged decision record settles offline/degraded behavior, buffering and gap semantics, cached-grant limits, per-class actuation, rejected alternatives, and implementation invariants while respecting parent #5686 and grant design #5900.                                                                                                          |

## Later-work and filing check

No gap or bug was identified, so no issue was filed. #7639's issue history records the relevant later disposition explicitly: PR #7642's overlapping session-record change and tests were salvaged into #7639, and #7642 was closed as superseded. The remaining PRs satisfy their linked issues' design or implementation criteria; the sibling and parent issues cited in those records own their separately scoped work.

## Slice state

This continuation adds five audited PRs to the prior five-PR record for batch 3: **10/250 audited; 240 remain; batch 3 is still partial**. The next batch item is PR #7649. The audit outcome labels are on all five PRs. The batch ledger for all 250 PRs is not complete.

## Raw supplements

`raw/` contains the source API responses for each audited PR, its changed files and comments, the linked issue bodies and complete comment sets, current PR label readbacks, and the tracking and umbrella snapshots used to check lane activity.
