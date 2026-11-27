# AUDIT-7 merged-PR audit — batch 3 continuation (#7757–#7767)

## Brief

Continue the AUDIT-7 Lane 4 batch 3/14 audit for `developing-today/code`: read each selected PR's body and complete diff, compare it with the spawning issue and comments, check later merged work for any remainder, apply one Lane 4 outcome label, and record the ledger on #13879. The five payload entries at this boundary are #7757, #7762, #7763, #7764 and #7767.

## Method and evidence

- Verified all five payload entries resolve to merged PRs. Read each full PR body, changed-file response and PR comments; file and comment counts match metadata: #7757 (1/2), #7762 (2/1), #7763 (1/2), #7764 (3/3), #7767 (1/4).
- Read each source issue's body and comments, confirming #7750 (3), #7761 (2), #1084 (47), #5994 (46), and #5415 (53). Also read open hardware issues #1080 (8 comments) and #1097 (11) that PR #7763 names as gates.
- For #7763, verified that the epic's addressing/egress documentation is delivered but its hardware and management-network acceptance remains tracked in open #1080/#1097. The #1084 timeline records the bot closing it at 2026-11-10T21:11:14Z, seconds after PR merge, while its completion comment says the epic remains open. Posted the Lane 2 correction pointer to #13871 comment 7157579495 and cross-linked it from PR comment 7157583879. Lane 4 did not reopen the issue.
- For #7767, checked later merged PR #13683's full body and files plus #5415 comments #7113280610/#7113690860. The owner revised the requirement to support both Plasma and Hyprland routes; #13683 records that decision, and the issue documents active Sunshine plus real Moonlight sessions under Hyprland. This completes the route acceptance that #7767 itself had left open.
- Applied and read back `audit:complete` on #7757, #7762 and #7764; `audit:partial` on #7763; and `audit:completed-later` on #7767.

## Per-entry ledger

| Payload number | Spawning issue | Verdict                 | Finding                                                                                                                                                                                                                                                                                                                  |
| -------------: | -------------: | ----------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
|          #7757 |          #7750 | `audit:complete`        | Gates local-claims hex decoding on ASCII hexdigits and decodes byte chunks, preventing both `+a` alternate spellings and byte-boundary panics. Regression coverage matches the issue's malformed and valid cases.                                                                                                        |
|          #7762 |          #7761 | `audit:complete`        | Adds even-length and ASCII-hex gates to both atlas-search and fleet-search cursor parsers, returning the existing malformed-cursor error instead of panicking. Both copies receive regression tests.                                                                                                                     |
|          #7763 |          #1084 | `audit:partial`         | Commits the online addressing/segmentation and egress plan. Physical NIC/switch facts and BMC/KVM reachability remain gated on open #1080/#1097; #1084 was closed prematurely at merge. Lane 2 pointer: #13871 comment 7157579495.                                                                                       |
|          #7764 |          #5994 | `audit:complete`        | Records the class-by-tier import policy and per-class fixture specification. The issue's done shape explicitly allows fixtures to be specified or landed; the record supplies the positive pure control, compile-fail effectful cases, oracles and wiring, with extension fixtures deferred until their providers exist. |
|          #7767 |          #5415 | `audit:completed-later` | Adds KWallet to the user profile and starts KRdp in Plasma; it intentionally leaves Hyprland logon unverified. Later #13683 records the dual-route decision and #5415 contains live Sunshine/Moonlight Hyprland evidence, completing the revised requirement. The relationship is cross-linked on PR comment 7157584380. |

## Slice state

Five payload entries were processed: five actual merged PRs audited and labelled, with no invalid references. Three are complete, one is partial, and one was completed by later merged work. Across all slices to this point, **71 valid PRs are audited from 75 processed payload identifiers**; #7667, #7668, #7693 and #7694 are issues rather than PRs. There are 175 payload identifiers remaining; next is #7768. Batch 3 remains partial.

Tracking progress correction: [#13879 comment 7157615513](https://github.com/developing-today/code/issues/13879#issuecomment-7157615513); the immediately preceding comment 7157607263 repeated the prior slice count and is superseded.

## Raw supplements

`raw/` contains the five PR bodies, full changed-file responses, comments and label readbacks; five source issue bodies/comments; full issue comments for the #1080/#1097 hardware gates; the #1084 closure timeline and merged-PR search; and #13683's body/files/comments plus the updated #5415 decision and live-verification evidence.
