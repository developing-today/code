# AUDIT-7 merged-PR audit — batch 3 continuation (#7685–#7691)

## Brief

Continue the AUDIT-7 Lane 4 batch 3/14 audit for `developing-today/code`: read each selected PR's body and complete diff, compare it with the spawning issue and comments, check later merged work for any remainder, apply one Lane 4 outcome label, and record the ledger on #13879. This slice covers PRs #7685, #7687, #7689, #7690 and #7691 only.

## Method and evidence

- Read each complete PR body and every changed-file response. File counts match metadata: #7685 (1), #7687 (1), #7689 (1), #7690 (9), #7691 (2); all patches are present.
- Read each PR comment set and full source issue comment set. Counts match metadata: PRs #7685 (11), #7687 (1), #7689 (14), #7690 (1), #7691 (4); issues #5443 (34), #5785 (39), #5847 (56), #1949 (36), #6004 (93).
- Revalidated the linked follow-ups for the Joplin design and Bevy design: #5847's issue history records the design and its split implementations landing; #5781's issue history records adoption and the implementation slices. These confirm the records meet the design issues' own acceptance, not that their implementations belong in these PRs.
- Applied and read back `audit:complete` on each PR. The #7690 body also records a pre-existing #7686 test failure; it is separate from the run-seed acceptance and is not treated as a regression from this PR.

## Per-PR ledger

| PR | Merged | Spawning issue | Verdict | Finding |
|---:|---|---:|---|---|
| #7685 | 2026-11-09 | #5443 | `audit:complete` | The NixOS configuration declares `users.users.user.linger = true` and documents the keyring boundary. This is the exact declarative change requested by #5443. |
| #7687 | 2026-11-08 | #5785 | `audit:complete` | The design record settles markup encoding contexts, safe-plain as a per-client rendering mode, no-evaluation on ingest, and ANSI/OSC sequence handling, with the surface inventory and compatibility boundary requested by the issue. |
| #7689 | 2026-11-09 | #5847 | `audit:complete` | The Logseq importer design covers pages/blocks, stable identity and manifest, references/assets, round-trip expectations, lossy cases, invariants, and the synthetic/real corpus plan. The design issue later closed after its linked splits landed. |
| #7690 | 2026-11-08 | #1949 | `audit:complete` | A UUIDv7 run seed now reaches every Go test process through a wrapper and CI seed job; local replay, fail-closed CI, shuffle derivation, artifact collection and wrapper regressions are implemented as requested. The noted #7686 failure is explicitly pre-existing and separately filed. |
| #7691 | 2026-11-10 | #6004 | `audit:complete` | The design note provides the reserved/delegable authority table, platform invariants, the LambdaMOO governance rationale, quota treatment and the checklist requested by #6004. |

## Slice state

This slice adds five valid PRs to the prior 33: **38 valid PRs audited from 40 processed payload identifiers**; #7667 and #7668 remain flagged as non-PR issue-number entries. The payload has 210 identifiers remaining; the next is #7692. All five outcome labels were read back. Batch 3 remains partial.

## Raw supplements

`raw/` contains the five audited PR bodies, full changed-file responses, comments and label readbacks; source issue bodies and complete comment sets; related issue history used to confirm the design outcomes; and current tracking/umbrella snapshots.
