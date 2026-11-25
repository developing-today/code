# AUDIT-7 merged-PR audit — batch 3 continuation (#8016–#8021)

## Brief

Continue the AUDIT-7 Lane 4 batch 3/14 audit for `developing-today/code`: read each selected PR's body and complete diff, compare it with the spawning issue and comments, check later merged work for any remainder, apply one Lane 4 outcome label, and record the ledger on #13879. The five payload entries at this boundary are #8016, #8018, #8019, #8020 and #8021.

## Method and evidence

- Verified all five payload entries resolve to merged PRs. Read full PR bodies, changed-file responses and PR comments; counts are #8016 (2 files/5 comments), #8018 (5/4), #8019 (10/5), #8020 (2/4), and #8021 (2/1). No inline review threads were returned for this slice.
- Read source issue bodies and full comment sets: #7892 (3 comments), #7948 (3), #2168, #2188 (18), and #6536 (16). Read later PR #11236 for the #7948 follow-on.
- #8016 rejects non-unsigned `chat.history` count values rather than silently using the default; #7892 closes with the fix.
- #8018 lands GitHub identity configuration, soft/hard block classification, rotation state and token minting for #7948. The daemon call path was explicitly left to follow-on #8017; later #11236 wires it, but #8017 remains open for live activation pending an authoritative App identity roster. #7948 closed despite the issue note saying it should remain open; Lane 2 pointer: #13871 comment 7161621192.
- #8019 implements shared script-name resolution for CLI and daemon `/v1/exec`, including `--remote-file`; #2168 closes with the feature.
- #8020 is the post-merge maintenance follow-up that registers `--remote-file` in the existing #2188 exec-flag coverage contract and refreshes the generated flag index. It has no separate source issue.
- #8021 makes DNS state encryption atomic and verifies the produced ciphertext before replacing the saved state; #6536 closes with the fix.
- Applied and read back `audit:complete` on #8016, #8019 and #8021, `audit:partial` on #8018, and `audit:no-issue` on #8020. Posted the partial closure pointer and the no-issue disposition.

## Per-entry ledger

| Payload number | Spawning issue | Verdict | Finding |
|---:|---:|---|---|
| #8016 | #7892 | `audit:complete` | Rejects non-unsigned `chat.history` count values with the documented invalid-arguments response instead of silently substituting the default. |
| #8018 | #7948 | `audit:partial` | Adds GitHub identity lists, soft/hard classification, durable rotation state and App token minting. Daemon call wiring later landed in #11236, but live activation remains open in #8017; #7948 closed prematurely. Lane 2 pointer: #13871 comment 7161621192. PR note: #7161623223. |
| #8019 | #2168 | `audit:complete` | Shares script resolution between CLI and daemon, resolves bare names on `/v1/exec`, and adds the explicit `--remote-file` mode while preserving default `--remote` behavior. |
| #8020 | none (coverage-contract follow-up to #8019) | `audit:no-issue` | Adds a reasoned exemption for daemon-only `--remote-file` in the #2188 exec-flag coverage contract and regenerates the flag index. No separate source issue was filed. PR note: #7161624797. |
| #8021 | #6536 | `audit:complete` | Encrypts DNS Terraform state into a same-directory temporary file, verifies non-empty and decrypt-back equality, then atomically replaces the saved ciphertext; failure preserves the prior record. |

## Slice state

Five payload entries were processed: five actual merged PRs audited and labelled, with no invalid references. Three are complete, one partial, and one has no separate spawning issue. Across all slices to this point, **206 valid PRs are audited from 210 processed payload identifiers**; #7667, #7668, #7693 and #7694 are issues rather than PRs. There are 40 payload identifiers remaining; next is #8025. Batch 3 remains partial.

Tracking progress comment: [#13879 comment 7161649598](https://github.com/developing-today/code/issues/13879#issuecomment-7161649598).

## Raw supplements

`raw/` contains all five PR bodies, complete changed-file responses, PR comments, source issue bodies/comment pages, later PR #11236, merged-PR searches, label readbacks, the Lane 2 closure pointer, and the partial/no-issue disposition comments.
