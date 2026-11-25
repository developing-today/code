# AUDIT-7 merged-PR audit — batch 3 continuation (#7874–#7878)

## Brief

Continue the AUDIT-7 Lane 4 batch 3/14 audit for `developing-today/code`: read each selected PR's body and complete diff, compare it with the spawning issue and comments, check later merged work for any remainder, apply one Lane 4 outcome label, and record the ledger on #13879. The five payload entries at this boundary are #7874, #7875, #7876, #7877 and #7878.

## Method and evidence

- Verified all five payload entries resolve to merged PRs. Read each full PR body, changed-file response and comments; counts match metadata: #7874 (1/5), #7875 (1/1), #7876 (5/3), #7877 (1/1), #7878 (1/7).
- Read source issue bodies and full comment sets, confirming #5966 (26), #5838 (21), #1400 (35), #5900 (31), and #5749 (44).
- #7876 implements the unpriced-variant detail-button item in open umbrella #1400. Its later triage confirms the broader Cost Explorer parity checklist remains active; later #14461 completes the comparison overlays, but other checklist items still remain. Cross-linked the partial outcome on PR comment 7159183064.
- Applied and read back `audit:complete` on #7874, #7875, #7877 and #7878, and `audit:partial` on #7876.

## Per-entry ledger

| Payload number | Spawning issue | Verdict | Finding |
|---:|---:|---|---|
| #7874 | #5966 | `audit:complete` | Answers the gas, timeout and multithreading questions with evidence and a recommendation for per-function fuel budgets, epoch interruption as a backstop, and parallel Stores rather than guest threads. It gives cited overhead estimates and names an implementation benchmark follow-on; the research issue is closed by the record. |
| #7875 | #5838 | `audit:complete` | Records local-first multi-device sync, sibling signed revisions for conflicts, offline-first behavior, tiered encryption and lazy blob sync, reconciled with the existing ID sync designs. |
| #7876 | #1400 | `audit:partial` | Implements unpriced-variant detail buttons under Cost Explorer charts. The wider parity umbrella remains open for chart settings, per-band overrides, evidence/coverage data, projection pricing, sliders and compare-tray work. |
| #7877 | #5900 | `audit:complete` | Records provider grant shape, observation/actuation distinction, sensitivity-scaled approval, attenuation/revocation/audit invariants and parent-boundary rules. The design issue is closed. |
| #7878 | #5749 | `audit:complete` | Caps pending email-code state globally and per account, preserves deterministic refusal/resend/expiry behavior, and adds tests for capacity, isolation and one-time verification. The issue is closed by this fix. |

## Slice state

Five payload entries were processed: five actual merged PRs audited and labelled, with no invalid references. Four are complete; #7876 is partial with the remainder tracked on #1400. Across all slices to this point, **136 valid PRs are audited from 140 processed payload identifiers**; #7667, #7668, #7693 and #7694 are issues rather than PRs. There are 110 payload identifiers remaining; next is #7881. Batch 3 remains partial.

Tracking progress comment: [#13879 comment 7159220591](https://github.com/developing-today/code/issues/13879#issuecomment-7159220591).

## Raw supplements

`raw/` contains the five PR bodies, complete changed-file responses, comments and label readbacks; the five source issue bodies/comment pages; merged-PR search results; and the #7876 partial-outcome cross-reference.
