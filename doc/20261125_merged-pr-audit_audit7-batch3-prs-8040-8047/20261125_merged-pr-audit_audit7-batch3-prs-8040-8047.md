# AUDIT-7 merged-PR audit — batch 3 continuation (#8040–#8047)

## Brief

Continue the AUDIT-7 Lane 4 batch 3/14 audit for `developing-today/code`: read each selected PR's body and complete diff, compare it with the spawning issue and comments, check later merged work for any remainder, apply one Lane 4 outcome label, and record the ledger on #13879. The five payload entries at this boundary are #8040, #8041, #8045, #8046 and #8047.

## Method and evidence

- Verified all five payload entries resolve to merged PRs. Read full PR bodies, changed-file responses, PR comments, inline review comments and reviews; counts are #8040 (2 files/3 comments), #8041 (1/5), #8045 (1/2), #8046 (3/3), and #8047 (1/3). No inline review threads were returned for this slice.
- Read source issue bodies and full comment sets: #7760 (6 comments), #5871, #7349, and #5119 (92). For #8047, read the closed archive PR #7529; it contains only the salvaged test file and references the archive-disposition programme #7297, not a feature issue.
- #8040 changes both claimant reads from the nonexistent `/messages` endpoint to `/message`, and fails closed on an unknown response; it resolves #7760's fail-open zero-message gate.
- #8041 settles talk-page creation, read-authority intersection, lifecycle, moderation/binding, and scoped ordering for #5871; the design issue closes with the record.
- #8045 adds `--worker` parsing to queue `health` with attribution-only semantics for #7349; its regression test preserves rejection of unknown flags.
- #8046 implements phase 2 pull-policy gating for #5119. Later PRs #8356 and #8413 persist/seeding provisioning, and PR #8529 supplies the final freshness admission path that closes #5119.
- #8047 re-applies a test-only policy/manifest reply-shape probe from archived PR #7529. The probe pins existing reply refusal behavior, and no separate spawning issue or remaining source bug was found.
- Applied and read back `audit:complete` on #8040, #8041 and #8045, `audit:completed-later` on #8046, and `audit:no-issue` on #8047. Posted the later-completion and no-issue dispositions.

## Per-entry ledger

| Payload number |          Spawning issue | Verdict                 | Finding                                                                                                                                                                                            |
| -------------: | ----------------------: | ----------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
|          #8040 |                   #7760 | `audit:complete`        | Reads the correct singular `/message` endpoint and treats unknown/error responses as unknown rather than confirming zero; resolves the fail-open gate.                                             |
|          #8041 |                   #5871 | `audit:complete`        | Settles lazy talk-page creation, narrowing-only permissions, lifecycle/orphan behavior, owner moderation and scoped ordering; the design issue closes with this decision record.                   |
|          #8045 |                   #7349 | `audit:complete`        | Adds `--worker` to `health` with attribution-only behavior while retaining strict rejection of unknown arguments; the issue closes with the fix.                                                   |
|          #8046 |                   #5119 | `audit:completed-later` | Implements the pull-policy gate (slice 2). Later PRs #8356/#8413 complete provisioning and world-directory seeding; PR #8529 completes freshness admission and closes #5119. PR note: #7162746566. |
|          #8047 | none (salvage of #7529) | `audit:no-issue`        | Re-applies a test-only probe for malformed `policy` and `manifest_state` replies. No separate spawning issue or unhandled defect remains. PR note: #7162748314.                                    |

## Slice state

Five payload entries were processed: five actual merged PRs audited and labelled, with no invalid references. Three are complete, one was completed by later merged work, and one has no separate spawning issue. Across all slices to this point, **221 valid PRs are audited from 225 processed payload identifiers**; #7667, #7668, #7693 and #7694 are issues rather than PRs. There are 25 payload identifiers remaining; next is #8049. Batch 3 remains partial.

Tracking progress comment: [#13879 comment 7162778224](https://github.com/developing-today/code/issues/13879#issuecomment-7162778224).

## Raw supplements

`raw/` contains all five PR bodies, complete changed-file responses, PR comments, inline review comments and review receipts; source issue bodies/comment pages; archived PR #7529; later completion PRs #8356, #8413 and #8529; merged-PR and source searches; label readbacks; and posted disposition comments.
