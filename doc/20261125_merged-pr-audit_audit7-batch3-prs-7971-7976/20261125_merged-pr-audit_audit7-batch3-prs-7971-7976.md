# AUDIT-7 merged-PR audit — batch 3 continuation (#7971–#7976)

## Brief

Continue the AUDIT-7 Lane 4 batch 3/14 audit for `developing-today/code`: read each selected PR's body and complete diff, compare it with the spawning issue and comments, check later merged work for any remainder, apply one Lane 4 outcome label, and record the ledger on #13879. The five payload entries at this boundary are #7971, #7973, #7974, #7975 and #7976.

## Method and evidence

- Verified all five payload entries resolve to merged PRs. Read full PR bodies, changed-file responses, PR comments, inline review comments and reviews; counts are #7971 (1 file/3 comments), #7973 (22/1), #7974 (1/3), #7975 (2/4), and #7976 (3/5). No inline review threads were returned for this slice.
- Read source issue bodies and full comment sets: #4593 (62 comments), #4592 (56), #5612 (39), #6045 (33), #2832 (19), #2343 (37), and the follow-up #7970 (3). Read later PR #7199 for the clippy remainder of #5612.
- #7971 delivers the #4593 closing audit inventory and contract statement. The one per-entry-version admission gap was correctly filed as #7970, which was later fixed by PR #10944; fixing it was explicitly out of scope for the audit issue.
- #7973 fixes the cargo-fmt remainder of #5612. It leaves clippy to PR #7199, which merged on 2026-11-11. The source issue had closed with #7973 on 2026-11-09, before that remaining slice landed; Lane 2 closure pointer: #13871 comment 7160938580.
- #7974 records the in-toto-shaped per-batch ingest attestation requested by #6045; the design issue closes with the record.
- #7975 validates `opencode.jsonc` structure at the parse boundary and guards JSONC tree editing, resolving #2832's named-key diagnostic and clean refusal requirements.
- #7976 provides the `id.device.v1` MQTT bridge specification and labelled PoC for #2343; the issue closes with this scoped prototype. Production transport/firmware remains outside the requested PoC.
- Applied and read back `audit:complete` on #7971, #7974, #7975 and #7976, and `audit:completed-later` on #7973. Posted the completed-later relationship on #7973 and the closure pointer on #13871.

## Per-entry ledger

| Payload number | Spawning issue | Verdict | Finding |
|---:|---:|---|---|
| #7971 | #4593 | `audit:complete` | States the replication-admission contract and inventories restore preconditions against accept. Files the confirmed entry-version gap as #7970; later PR #10944 fixes that follow-up. The audit issue's own deliverable is complete. |
| #7973 | #5612 | `audit:completed-later` | Applies the formatting-only remainder. The clippy leg was left to #7199, which merged later and completed the issue. The issue closed before that slice landed; Lane 2 pointer: #13871 comment 7160938580. PR note: #7160942654. |
| #7974 | #6045 | `audit:complete` | Specifies the in-toto statement shape, storage, emission points, rejection-path behavior, verification and decision gates; the design issue closes with the record. |
| #7975 | #2832 | `audit:complete` | Validates root/provider/model shapes after JSONC parsing and refuses malformed intermediate tree-edit paths with file and key diagnostics; missing provider is safely treated as nothing to sync. |
| #7976 | #2343 | `audit:complete` | Delivers the MQTT topic/schema specification and labelled fake-broker/device PoC with fail-closed auth, retained-wants guard, wire bounds, journaling, tick floor and backoff; the source issue closes with the scoped prototype. |

## Slice state

Five payload entries were processed: five actual merged PRs audited and labelled, with no invalid references. Four are complete and one was completed by later merged work. Across all slices to this point, **181 valid PRs are audited from 185 processed payload identifiers**; #7667, #7668, #7693 and #7694 are issues rather than PRs. There are 65 payload identifiers remaining; next is #7977. Batch 3 remains partial.

Tracking progress comment: [#13879 comment 7160967303](https://github.com/developing-today/code/issues/13879#issuecomment-7160967303).

## Raw supplements

`raw/` contains all five PR bodies, changed-file responses, PR comments, inline review comments and review receipts; source issue bodies/comment pages; follow-up issue #7970; later PR #7199; merged-PR searches; label readbacks; and the Lane 2 closure pointer.
