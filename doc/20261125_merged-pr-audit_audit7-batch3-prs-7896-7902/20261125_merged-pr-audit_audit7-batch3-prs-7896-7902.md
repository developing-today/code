# AUDIT-7 merged-PR audit — batch 3 continuation (#7896–#7902)

## Brief

Continue the AUDIT-7 Lane 4 batch 3/14 audit for `developing-today/code`: read each selected PR's body and complete diff, compare it with the spawning issue and comments, check later merged work for any remainder, apply one Lane 4 outcome label, and record the ledger on #13879. The five payload entries at this boundary are #7896, #7897, #7898, #7900 and #7902.

## Method and evidence

- Verified all five payload entries resolve to merged PRs. Read each full PR body, changed-file response and comments; recorded counts are #7896 (2 files/2 comments), #7897 (2/7), #7898 (13/3), #7900 (3/8), and #7902 (2/4).
- Read source issue bodies and full comment sets: #5836 (37 comments), #6822 (52), #1161 (55), #5870 (53), and #6037 (26).
- For #7898, checked the merged PR and closed source issue #1161; the patch maps a missing subscription to an explicit lapsed state, removes stale percentage display, and keeps terminal ingest from becoming an error.
- For #7902, source discussion confirms the structural-first chunk-boundary policy and that #6037 closed after the decision; span reuse remains provisional under parent #6028, which is also closed.
- Applied and read back `audit:complete` on #7896, #7897, #7898, #7900 and #7902.

## Per-entry ledger

| Payload number | Spawning issue | Verdict          | Finding                                                                                                                                                                                                              |
| -------------: | -------------: | ---------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
|          #7896 |          #5836 | `audit:complete` | Records the personalized note search ranking decision, including own-edit-first, ordered relevance/evidence/graph/recency signals, explainability and graffiti demotion. The issue closes with this decision record. |
|          #7897 |          #6822 | `audit:complete` | Guards malformed E2E timing seeds, including non-object values, non-finite numbers and booleans; the issue's requested CI hardening is covered.                                                                      |
|          #7898 |          #1161 | `audit:complete` | Displays an expired OpenCode Go workspace as lapsed, avoids stale quota percentages, and treats terminal ingest as non-error.                                                                                        |
|          #7900 |          #5870 | `audit:complete` | Settles chat-anchor data with typed links, revision-bound part anchors, survival/edit/delete states and parent privacy. The issue closes on the design decision.                                                     |
|          #7902 |          #6037 | `audit:complete` | Records the structural-first chunk-boundary policy with overlap and migration guidance. The parent #6028 tracked the provisional span-reuse question and is closed.                                                  |

## Slice state

Five payload entries were processed: five actual merged PRs audited and labelled, with no invalid references. All five are complete. Across all slices to this point, **146 valid PRs are audited from 150 processed payload identifiers**; #7667, #7668, #7693 and #7694 are issues rather than PRs. There are 100 payload identifiers remaining; next is #7905. Batch 3 remains partial.

Tracking progress comment: [#13879 comment 7160055271](https://github.com/developing-today/code/issues/13879#issuecomment-7160055271).

## Raw supplements

`raw/` contains the five PR bodies, complete changed-file responses, comments and label readbacks; and the five source issue bodies and comment pages.
