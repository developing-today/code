# AUDIT-7 merged-PR audit — batch 3 continuation (#7832–#7842)

## Brief

Continue the AUDIT-7 Lane 4 batch 3/14 audit for `developing-today/code`: read each selected PR's body and complete diff, compare it with the spawning issue and comments, check later merged work for any remainder, apply one Lane 4 outcome label, and record the ledger on #13879. The five payload entries at this boundary are #7832, #7834, #7835, #7836 and #7842.

## Method and evidence

- Verified all five payload entries resolve to merged PRs. Read each full PR body, changed-file response and comments; counts match metadata: #7832 (1/2), #7834 (6/5), #7835 (2/2), #7836 (1/2), #7842 (9/1).
- Read source issue bodies/comment sets for #7098 (14), #7325 (11), #7375 (5), #7366 (6), #7356 (7), #7706 (4), and #5640 (27). The source issues for #7835/#7836 were created later as retroactive records #10080/#10081, not as originating issues.
- #7834's PR body links five issues and the source comments confirm they were closed by this merged fix. #7822 and #7835/#7836 were already audited separately; #7834 resolves the remaining ChainExt fixture build break.
- Applied and read back `audit:partial` on #7832, `audit:complete` on #7834 and #7842, and `audit:no-issue` on #7835 and #7836. Cross-linked the partial/no-issue outcomes on PR comments 7158664139, 7158664480 and 7158664912.

## Per-entry ledger

| Payload number | Spawning issue(s) | Verdict | Finding |
|---:|---|---|---|
| #7832 | #7098 | `audit:partial` | Removes OpenClaw's configured ACP invocation path to the real Claude CLI, following the recorded order. It covers one headless-invocation path; #7098's broader recovery included the backup snapshot and upstream write-boundary guard, with combined deployment/acceptance recorded on #7098 comments #6833106147 and #6837006586. |
| #7834 | #7325, #7375, #7366, #7356, #7706 | `audit:complete` | Repairs queue-row reading, checkpoint-stage lifting, shadow-mode safety, authenticated prompt delivery/reconciliation, and broker payload retrieval. The PR closes all five listed defects; issue comments confirm the closures. |
| #7835 | — | `audit:no-issue` | No originating issue/task was linked at merge. The PR implements the direct quota-attribution follow-up to #7830; retroactive home #10080 was created later. |
| #7836 | — | `audit:no-issue` | No originating issue/task was linked at merge. The PR removes the remaining duplicate `ChainExt` fields in `chain_properties` tests; retroactive home #10081 was created later. |
| #7842 | #5640 | `audit:complete` | Makes all nine Atlas/Fleet search home-resolution sites fail loudly for unset/empty `HOME`, preventing CWD-relative token/config/state paths. The source issue is closed with the full test/probe result. |

## Slice state

Five payload entries were processed: five actual merged PRs audited and labelled, with no invalid references. Two are complete, one is partial, and two have no originating issue. Across all slices to this point, **116 valid PRs are audited from 120 processed payload identifiers**; #7667, #7668, #7693 and #7694 are issues rather than PRs. There are 130 payload identifiers remaining; next is #7847. Batch 3 remains partial.

Tracking progress comment: [#13879 comment 7158684561](https://github.com/developing-today/code/issues/13879#issuecomment-7158684561).

## Raw supplements

`raw/` contains the five PR bodies, full changed-file responses, comments and label readbacks; seven source issue bodies/comment pages; source-search results for the no-issue PRs; and #7098's combined completion evidence.
