# AUDIT-7 merged-PR audit — batch 3 continuation (#7660–#7666)

## Brief

Continue the AUDIT-7 Lane 4 batch 3/14 audit for `developing-today/code`: read each selected PR's body and complete diff, compare it with the spawning issue and comments, check later merged work for any remainder, apply one Lane 4 outcome label, and record the ledger on #13879. This slice covers PRs #7660, #7661, #7662, #7665 and #7666; #7663 and #7664 are not in the fixed batch payload.

## Method and evidence

- Read each complete PR body and every changed-file response. File counts match metadata: #7660 (1), #7661 (2), #7662 (2), #7665 (5), #7666 (2); all patches are present.
- Read each PR conversation comment set and the full source issue comment sets. Counts match metadata: PRs #7660 (2), #7661 (2), #7662 (1), #7665 (4), #7666 (3); issues #2300 (14), #6038 (18), #5891 (37), #3322 (33), #7569 (20).
- For #7660, read later PR #11164's full body and diff: it records the authorized tailnet health call and distinct caller identities that #7660 left unverified. For #7666, read the later mux-credit PR #13859's full body and diff: it adds accounting but explicitly states there is still no live leg.
- Applied and read back one Lane 4 outcome label on each PR. The raw responses, later PRs, follow-up issue/task and audit comments are retained in `raw/`.

## Per-PR ledger

|    PR | Merged     | Spawning issue | Verdict                 | Finding                                                                                                                                                                                                                                                                                                                                                                                                                                            |
| ----: | ---------- | -------------: | ----------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| #7660 | 2026-11-08 |          #2300 | `audit:completed-later` | The merged doc records live rejection of an invalid tailnet token. The issue also required a successful authorized non-loopback call and distinct consumer identity; these were demonstrated later and recorded in merged PR #11164 (2026-11-17). The link is on PR #7660.                                                                                                                                                                         |
| #7661 | 2026-11-08 |          #6038 | `audit:complete`        | The experiment reports cosine drift and rank movement across four context perturbations and derives a byte-identical-window reuse rule. The final merged report pins the model, scopes flip rates to the 50-window pool, and records the rerun after Codex findings.                                                                                                                                                                               |
| #7662 | 2026-11-10 |          #5891 | `audit:complete`        | The design record settles one generator core, the InProcess/LocalSocket/Iroh target surfaces, honest/adversarial profiles, reproducibility, measurement and assertion contracts, alternatives, invariants, and implementation order. The issue asked for a decision record; implementation is separately packetized.                                                                                                                               |
| #7665 | 2026-11-08 |          #3322 | `audit:complete`        | The archive-before-prune retention contract, visible history-gap health, backup set and manifest-verified restore evidence complete the remaining #3322 audit-history/backup slice. The full issue record later verifies the other acceptance criteria already delivered by PR #5137 and this slice by #7665.                                                                                                                                      |
| #7666 | 2026-11-08 |          #7569 | `audit:partial`         | The PR delivers only slice 1 (typed frame codec and credit primitive) and explicitly lists leg establishment, mux routing/scheduling, keepalive, resumption, and E2E forwarding as remaining. Later PR #13859 adds mux credit accounting but no live leg. Issue #7569 is closed despite those criteria remaining; successor issue #14918 and task 23711 now track the next transport-leg slice. Closure-review pointer: #13871 comment 7155601444. |

## Later-work and filing check

#11164 completed #7660's authorized-caller and per-consumer-identity remainder. #13859 implements the mux credit-accounting portion of #7666's remainder, but not the transport legs, keepalive, resumption, or E2E test. Because #7569 is closed, this lane did not reopen it; issue #14918 was filed for the remaining serve-proxy protocol and its first vertical slice was enqueued as task 23711 in `repo-health-code-pilot`.

## Slice state

This slice adds five audited PRs to the prior 20: **25/250 audited; 225 remain; batch 3 remains partial**. The next PR in the fixed payload list is #7667. All five outcome labels were read back. The batch ledger for all 250 PRs is not complete.

## Raw supplements

`raw/` retains the five audited PR bodies, full changed-file responses, comments and labels; source issue bodies and comment sets; later PRs #11164 and #13859; and the #14918 / task 23711 evidence and Lane 2 closure pointer.
