# AUDIT-7 merged-PR audit — batch 3 continuation (#8000–#8008)

## Brief

Continue the AUDIT-7 Lane 4 batch 3/14 audit for `developing-today/code`: read each selected PR's body and complete diff, compare it with the spawning issue and comments, check later merged work for any remainder, apply one Lane 4 outcome label, and record the ledger on #13879. The five payload entries at this boundary are #8000, #8001, #8002, #8006 and #8008.

## Method and evidence

- Verified all five payload entries resolve to merged PRs. Read full PR bodies, changed-file responses, PR comments, inline review comments and reviews; counts are #8000 (1 file/3 comments), #8001 (1/4), #8002 (2/3), #8006 (1/4), and #8008 (5/3). PR #8008 has three review comments; the CodeRabbit suggestion to skip the self-process test on unsupported platforms was fixed in commit `524715d415` and verified by the author.
- Read source issue bodies and full comment sets: #6554 (10 comments), #5783 (25), #7077 (34), #7686 (8), and #7063 (7).
- #8000 atomically replaces the encrypted Talos secret file only after successful, non-empty encryption and preserves the old copy on failure; #6554 closes with the fix.
- #8001 records the ESP32 platform design. The source issue remains open as the tracker for board, provisioning, firmware/OTA, power and field work; its latest notes say those remaining items have no separate follow-on packet. The existing issue retains that scope.
- #8002 changes the program-tending rule to key off `user/owned`, while the source issue's other sections were already label-based or had become moot because the three live cron schedules no longer exist; #7077 closes after the combined work.
- #8006 isolates the fixture config paths so TestConfigLayersProperty cannot import real host configuration; #7686 closes with the regression guard.
- #8008 adds Linux, Darwin and unsupported-platform process-stat implementations for mcpx handoff identity; #7063 closes with the fix and the review suggestion is addressed.
- Applied and read back `audit:complete` on #8000, #8002, #8006 and #8008, and `audit:partial` on #8001. Posted the remaining ESP32 scope on #8001.

## Per-entry ledger

| Payload number | Spawning issue | Verdict | Finding |
|---:|---:|---|---|
| #8000 | #6554 | `audit:complete` | Makes `save-secrets.sh` fail loudly and encrypt atomically through same-directory temporary files, preserving the prior ciphertext on failure; the issue closes with the fix. |
| #8001 | #5783 | `audit:partial` | Delivers the ESP32 platform decision record, but the source issue remains open for board, provisioning, firmware/OTA, power and field work. Issue notes say these items have no separate follow-on packet yet. PR note: #7161278646. |
| #8002 | #7077 | `audit:complete` | Replaces authorship-based program-tending phrasing with the label-based `user/owned` rule; the other issue sections were already corrected or moot, and the issue closes after the combined work. |
| #8006 | #7686 | `audit:complete` | Seeds the fixture layer paths directly and adds a regression guard against ancestor/host config leakage; the hermeticity issue closes with the fix. |
| #8008 | #7063 | `audit:complete` | Implements `procStat` with Linux `/proc`, Darwin `sysctl kern.proc`, and an explicit unsupported-platform error; updates the self-process test to skip with a reason where unsupported. |

## Slice state

Five payload entries were processed: five actual merged PRs audited and labelled, with no invalid references. Four are complete and one is partial. Across all slices to this point, **196 valid PRs are audited from 200 processed payload identifiers**; #7667, #7668, #7693 and #7694 are issues rather than PRs. There are 50 payload identifiers remaining; next is #8010. Batch 3 remains partial.

Tracking progress comment: [#13879 comment 7161302951](https://github.com/developing-today/code/issues/13879#issuecomment-7161302951).

## Raw supplements

`raw/` contains all five PR bodies, complete changed-file responses, PR comments, inline review comments and review receipts; source issue bodies/comment pages; merged-PR searches; label readbacks; and the posted partial-disposition comment.
