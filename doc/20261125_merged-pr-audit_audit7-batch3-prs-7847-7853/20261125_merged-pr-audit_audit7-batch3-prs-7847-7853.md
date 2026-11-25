# AUDIT-7 merged-PR audit — batch 3 continuation (#7847–#7853)

## Brief

Continue the AUDIT-7 Lane 4 batch 3/14 audit for `developing-today/code`: read each selected PR's body and complete diff, compare it with the spawning issue and comments, check later merged work for any remainder, apply one Lane 4 outcome label, and record the ledger on #13879. The five payload entries at this boundary are #7847, #7848, #7850, #7851 and #7853.

## Method and evidence

- Verified all five payload entries resolve to merged PRs. Read each full PR body, complete changed-file response and comments; counts match metadata: #7847 (5/1), #7848 (1/2), #7850 (1/4), #7851 (1/6), #7853 (2/5).
- Read each source issue body and full comments: #2841 (47), #1858 (40), #5874 (17), #5884 (15), #6012 (22), and #5814 (17).
- #7847 implements item (3) of the still-open #2841 set-remainder tracker. The latest #2841 triage says items (1)–(4) are landed and item (5), v1 plugin retirement, remains; the source issue is not complete, and that work stays on the tracker.
- For #7853, checked later merged PR #13734's body, complete file list and issue comment: it settles the harness/oracle/interop matrix and closes #5814. Cross-linked this later completion on PR comment 7158769786.
- Applied and read back `audit:partial` on #7847, `audit:completed-later` on #7853, and `audit:complete` on #7848, #7850 and #7851.

## Per-entry ledger

| Payload number | Spawning issue | Verdict | Finding |
|---:|---:|---|---|
| #7847 | #2841 (item 3 of #1858) | `audit:partial` | Adds the v2 daemon-picker tool as the supported substitute for v1 TUI `select()`. The five-item tracker remains open; item (5), v1 plugin retirement, is still outstanding. |
| #7848 | #5874 | `audit:complete` | Records signed resolution/proposal event types and derived folds, edit-linked acceptance, authorization/no-leak rules, invariants and implementation split. This is the requested decision record. |
| #7850 | #5884 | `audit:complete` | Records the native identity-creation ceremony, OS-CSPRNG custody, signed setup commitment, first artifacts, validation and failure invariants, consuming the recovery interface without changing its parent decision. |
| #7851 | #6012 | `audit:complete` | Delivers the visual-feature cost metrics/ranks, three gate layers, fallback substitution and viewer-facing downgrade policy. The #7019 ownership/fallback conflict is explicitly escalated rather than silently decided; the ranking design issue is closed. |
| #7853 | #5814 | `audit:completed-later` | Provides the initial adapter-conformance design skeleton and marks fixture/matrix rows as remainder. Later #13734 settles the harness, oracles and generated matrix; #5814 closes on that later record. |

## Slice state

Five payload entries were processed: five actual merged PRs audited and labelled, with no invalid references. Three are complete, one partial, and one completed by later merged work. Across all slices to this point, **121 valid PRs are audited from 125 processed payload identifiers**; #7667, #7668, #7693 and #7694 are issues rather than PRs. There are 125 payload identifiers remaining; next is #7857. Batch 3 remains partial.

Tracking progress comment: [#13879 comment 7158795410](https://github.com/developing-today/code/issues/13879#issuecomment-7158795410).

## Raw supplements

`raw/` contains the five PR bodies, full changed-file responses, comments and label readbacks; source issue bodies/comment pages; and complete later-PR evidence for #13734 and the #5814 close-out.
