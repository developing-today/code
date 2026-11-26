# AUDIT-7 merged-PR audit — batch 3 continuation (#7889–#7894)

## Brief

Continue the AUDIT-7 Lane 4 batch 3/14 audit for `developing-today/code`: read each selected PR's body and complete diff, compare it with the spawning issue and comments, check later merged work for any remainder, apply one Lane 4 outcome label, and record the ledger on #13879. The five payload entries at this boundary are #7889, #7890, #7891, #7893 and #7894.

## Method and evidence

- Verified all five payload entries resolve to merged PRs. Read each full PR body, changed-file response and comments; counts match metadata: #7889 (11/4), #7890 (4/2), #7891 (16/1), #7893 (1/1), #7894 (1/2).
- Read source issue bodies and full comment sets, confirming #1936 (41), #4967 (47), #1065 (43), #6058 (22), and #5837 (33).
- For #7889, checked later PR #11171 and the #1936 close-out: this PR's feedback log was the first of two remainders; #11171 adds Dropbox/Drive/Outlook, joining the earlier Tailscale declaration, and closes the source issue.
- For #7891, the Rust SDK is only one #1065 slice. The issue's done comment says the second SDK and component-model/WIT decision remain, while the timeline records an automated close one second after PR merge. Posted a Lane 2 correction pointer at #13871 comment 7159579443, and linked it on PR #7891 comment 7159587388; Lane 4 did not reopen the issue.
- Applied and read back `audit:completed-later` on #7889, `audit:partial` on #7891, and `audit:complete` on #7890, #7893 and #7894.

## Per-entry ledger

| Payload number | Spawning issue | Verdict | Finding |
|---:|---:|---|---|
| #7889 | #1936 | `audit:completed-later` | Adds explicit capability requests and an append-only log of empty registry searches. Later #11171 ships the other requested API declarations; #1936 closes after both halves land. |
| #7890 | #4967 | `audit:complete` | Detects oversized legacy checkpoints before replication, forcing legal compaction or returning an actionable error with the ordinal, encoded size, bound and recovery guidance. Tests cover compaction, no-program error and fast path. |
| #7891 | #1065 | `audit:partial` | Delivers the Rust F12.8 guest SDK and playable example. The issue's second SDK and component-model/WIT decisions remain open; its bot close immediately after merge contradicts the source completion note. Lane 2 pointer: #13871 comment 7159579443. |
| #7893 | #6058 | `audit:complete` | Records the embedding-cost ledger schema, delta reuse-ratio alert, per-source baseline slots, shared batch identity and invariants; unpopulated baselines are explicitly gated on ingest issues. |
| #7894 | #5837 | `audit:complete` | Settles graph scope/query, edge types, filtering/read authority, scale targets, and multiplayer consistency against the #2048 object model and SQL/PGQ direction. The design issue closes by this record. |

## Slice state

Five payload entries were processed: five actual merged PRs audited and labelled, with no invalid references. Three are complete, one partial, and one completed by later merged work. Across all slices to this point, **141 valid PRs are audited from 145 processed payload identifiers**; #7667, #7668, #7693 and #7694 are issues rather than PRs. There are 105 payload identifiers remaining; next is #7896. Batch 3 remains partial.

Tracking progress comment: [#13879 comment 7159762057](https://github.com/developing-today/code/issues/13879#issuecomment-7159762057).

## Raw supplements

`raw/` contains the five PR bodies, complete changed-file responses, comments and label readbacks; five source issue bodies/comment pages; #1936 later-PR close-out evidence; and the #1065 closure timeline/umbrella-pointer evidence.
