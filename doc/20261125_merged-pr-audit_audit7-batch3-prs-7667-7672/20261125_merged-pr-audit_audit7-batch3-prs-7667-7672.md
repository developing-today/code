# AUDIT-7 merged-PR audit — batch 3 continuation (#7667–#7672)

## Brief

Continue the AUDIT-7 Lane 4 batch 3/14 audit for `developing-today/code`: read each selected PR's body and complete diff, compare it with the spawning issue and comments, check later merged work for any remainder, apply one Lane 4 outcome label, and record the ledger on #13879. The five payload entries at this boundary are #7667, #7668, #7669, #7671 and #7672.

## Method and evidence

- Verified the pull-request endpoint and issue metadata for all five payload identifiers.
- #7667 and #7668 are closed **issues**, not pull requests: `pulls/7667` and `pulls/7668` return 404, while `issues/7667` and `issues/7668` have no `pull_request` field. They cannot receive PR outcome labels. Their linked work is outside this batch's listed PR numbers; the original payload entries remain recorded as invalid references.
- For the three actual PRs, read full bodies and changed-file responses: #7669 (3 files, 1 PR comment), #7671 (1 file, 1 PR comment), #7672 (1 file, 1 PR comment). Source issue comment counts match metadata: #4620 (14), #6009 (7), #5781 (21).
- Applied and read back `audit:complete` on the three PRs. No gap or bug was found in their stated scope.
- The tracking and umbrella snapshots used for the live-claim check, the invalid-reference responses, and final label readbacks are retained in `raw/`.

## Per-entry ledger

| Payload number | Live object | Spawning issue | Verdict | Finding |
|---:|---|---:|---|---|
| #7667 | Closed issue, not a PR | — | `invalid-reference` | PR API returned 404; issue #7667 is a closed issue. There is no PR to compare or label under Lane 4. |
| #7668 | Closed issue, not a PR | — | `invalid-reference` | PR API returned 404; issue #7668 is a closed issue. There is no PR to compare or label under Lane 4. |
| #7669 | Merged PR (2026-11-08) | #4620 | `audit:complete` | `state=all` now returns action history, unknown states return 400, and the regression test covers applied/all/new/bogus states. This meets the issue's exact false-zero fix. |
| #7671 | Merged PR (2026-11-08) | #6009 | `audit:complete` | The design note normalizes public-domain content to CC0-1.0 and code to 0BSD/Unlicense, keeps public domain and CC0 as separate taxonomy entries, and states the UI caveats. The issue's design deliverable is complete. |
| #7672 | Merged PR (2026-11-08) | #5781 | `audit:complete` | The decision-ready Bevy incorporation design covers journal authority, state mapping, event flow, authority postures, determinism, provider boundaries, open questions and implementation slices. Later issue comments adopt it as the design of record and close the design issue. |

## Slice state

This boundary processed five payload identifiers: **three actual PRs audited, two invalid issue-number references**. Combined with earlier slices, **28 valid PRs have been audited out of 30 processed payload entries**. There are 220 payload entries remaining; some may require the same live PR/issue check. The next payload number is #7673. The 250-PR batch ledger remains partial.

## Raw supplements

`raw/` contains the three audited PR bodies, complete file responses, PR comments and label readbacks; their issue bodies and full issue comment sets; the two issue-vs-PR endpoint readbacks; and the tracking/umbrella snapshots.
