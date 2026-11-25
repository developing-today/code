# Design-spawn backfill — native parenting of sub-design tracking issues + spawn rollup (#9068)

- **Date:** 2026-11-24
- **Status:** design (slice 1 of the design-gaps mill); no implementation in this doc.
- **Issue:** [#9068](https://github.com/developing-today/code/issues/9068) — `impl(pm-stack): design-spawn backfill`.
- **Tracking / queue:** design-gaps audit [#13873](https://github.com/developing-today/code/issues/13873); queue task 21672 (slice 1 — write/upgrade this design to the 9-point standard).
- **Prior design:** `doc/2026-11-12T22-50-00Z_design_pm-board-hierarchy-genealogy/note.md` §5 (D6, S7) — the register-on-paper-only analysis and the native-parenting + spawn-rollup representation. This doc does not re-open that representation; it turns the remaining backfill into ordered, verifiable slices.

## 1. Problem + context (what breaks today, with evidence)

The design register names 69 sub-designs under 41 top-level designs — on paper only. A spawned design's tracking issue is not natively parented under its spawning design's tracking issue, so the tree exists only in prose and the spawn rollup ("spawned 5 designs: 2 done, 2 in progress, 1 blocking \<final design\>") cannot be computed from live data. Evidence: #9068 body (register counts + acceptance); genealogy note §5 ("on paper only, because `design/<name>` is top-level-only and tracking issues are unparented").

Partial backfill has landed and is read back, not assumed. Live state 2026-11-24 (sub-issue lists read directly):

- F4 family home [#12950](https://github.com/developing-today/code/issues/12950) (parented under #8986): **10 children — 9 open, 1 closed** (#8838). Children: #2179, #8823–#8828, #8838, #1334 (ex-#8991), #8575 (ex-#8990).
- P2 family home [#12951](https://github.com/developing-today/code/issues/12951) (parented under #8986): **1 child, open** (#8831).
- #9068 itself: **0 sub-issues** — the backfill work is tracked here only in prose.
- Spawn-rollup API: PR #12283 merged (rollup live per the 2026-11-23 reconciliation notes).

What remains is exactly the two decision-shaped gaps plus the unverified remainder: the #1184 capacity block, the disputed F4/P2 homes, and the unapplied safe links — see §8.

## 2. Goals and explicit non-goals

**Goals:** (a) every filed sub-design tracking issue natively parented under its spawning design's tracking issue, with the applied list read back from both ends (child's live parent + parent's sub-issue list); (b) per-design spawn rollup computable from `subIssuesSummary` + Status + `blockedBy` and matching the register for at least one sampled family end-to-end, rendering Drew's example shape from real data; (c) tracking issues filed for named sub-designs lacking one **only where the register shows live work** — no speculative filing.

**Non-goals:** re-designing the genealogy representation (§5 of the 2026-11-12 note stands); the crate-split worked example (§8 of that note); PR-side genealogy (known gap, separately sliced); roadmap/priority/milestone machinery (§§2/4 of that note); changing any label family (existing `design/<name>` labels are reused, never invented here — naming of parent labels is §8 Q3).

## 3. Alternatives considered (with honest tradeoffs, including do-nothing)

- **Do nothing (register stays paper-only):** Pro: zero moves, zero risk of mis-parenting. Con: the tree stays unverifiable prose; the rollup renders nothing; #9068 stays open forever. Rejected — this is the problem the issue exists to kill.
- **#8986-as-hub (parent everything under the register epic):** Pro: one move target, no capacity questions. Con: flattens the tree — native chain says "tracked by the register", never "spawned by design X"; the rollup per design is uncomputable. Rejected as the native relationship; #8986 remains tracking context, not an alternate native parent (recorded 2026-11-23 decision).
- **Bulk re-parent script over the whole register:** Pro: fast. Con: GitHub allows exactly one native parent per issue and rejects adding an already-parented child (422); blind moves would collide with the #1184 100-child cap and with disputed homes. Rejected — moves are applied family by family with pre- and post-readback.
- **Chosen: design-lineage native parents + computed rollup, applied incrementally with read-back.** Pro: each move is verifiable the same day from live data; blocked items surface as themselves instead of stalling the batch. Con: slower; leaves named decision gaps open — accepted, those gaps are decisions, not lane work (§8).

## 4. Chosen approach — interfaces, data model, security/capability

- **Native relationship:** a sub-design's tracking issue is parented under its parent design's valid tracking issue when a slot and home exist; the child keeps its existing `design/<name>` membership labels (additive-retention: no labels changed by a move). One native parent only — enforced by GitHub, and the working reparent path is GraphQL `removeSubIssue` (node IDs) followed by REST add, since REST remove 404s in every tried form and REST add rejects already-parented children (422). Found 2026-11-23, recorded here so the next lane does not rediscover it.
- **Slot rule:** a move is safe only if the destination home exists and is under GitHub's 100-child cap; #1184 is at the cap, so #8829/#8830 do not move until the capacity call lands (§8 Q1).
- **Spawn rollup (read model, no new store):** per parent design, children via the native chain; done/in-progress split via `subIssuesSummary{total, completed}` + Status; the blocking spawn via `blockedBy` (spawn S blocks the final design's tracking issue). Sampled family F4 today reads as: "spawned 10 designs: 1 done, 9 in progress" (#12950: 10 children, #8838 closed). The "1 blocking \<final design\>" leg renders once a spawn in the family carries a `blockedBy` edge onto the final design — asserted in S3, not assumed here.
- **Security/capability:** moves are metadata-only (parent pointers + comments on both ends); no code, no secrets, no data access. The reconcile comment on child + parent is the audit trail. No capability change: lanes already holding repo + issue scope can perform moves; nothing here needs elevation.

## 5. Dependency-ordered build slices (each with acceptance)

- **S1 (this doc — done):** 9-point record for #9068, linked from the issue. Acceptance: all 9 sections present; live counts read back, not copied from older notes; open questions name deciders.
- **S2: remaining safe-link backfill.** Revalidate the live parent map (homes, caps, existing parents), then apply only safe links — explicitly excluding the §8 Q1/Q2 items. Acceptance: applied list + per-link read-back (child's live parent and each parent's sub-issue list), recorded on #9068; zero 422/404 surprises (pre-checked).
- **S3: rollup proof on the sampled family.** Compute the F4 rollup from live `subIssuesSummary` + Status + `blockedBy` and render Drew's example sentence from real data; file the blocking-leg assertion or the missing-edge finding. Acceptance: sentence rendered from live data on #9068, matching the register for the sampled family.
- **S4: label pass (gated on §8 Q3).** Once the `design/<name>` naming for parent designs is decided, apply parent labels to sub-designs carrying the parent's family. Acceptance: label map read back; no invented labels outside the decision. Not started until Q3 is decided.

## 6. Rollout + rollback

Rollout is S1→S4 in slice order; S2 moves are family-by-family with same-day read-back, so a bad move is caught before the next family. Rollback of a move is a re-parent back plus a comment on both ends (native re-point is symmetric; labels were never changed so there is nothing to un-label). No migration, no server-side state, no flag. The weekly monitor family (`design-register-weekly-monitor` precedent) is the enforcement surface for drift after S2 — GitHub cannot hard-block mis-parenting.

## 7. Verification plan (how we prove it works)

- **Link verification (S2):** every applied move is read back from both ends the same pass — the child's live parent field and the parent's sub-issue list — exactly as the 2026-11-23 gap-2 reconciliation did for all 11 F4/P2 rows. A move without both read-backs is not done.
- **Rollup verification (S3):** Drew's example sentence rendered from live API data for the F4 family and pasted on #9068 next to the register's expectation; mismatch is a finding, not a failure to report.
- **Regression rule:** #8986/#8990/#8991/#1184 sub-issue totals are recorded before and after each S2 batch (gap-2 precedent: 27→19, →0, →11) so a move that lands in the wrong home shows up as an arithmetic break.
- **Tests/live proof:** no unit tests — the surface is GitHub metadata, and the proof is live read-back. No corpus, production, or secret-tier data is touched by any slice.

## 8. Open questions (each with a named decider)

1. **#8829/#8830 → #1184: #1184 is at GitHub's 100-child cap — which children leave, or does the family get a new home?** — **Decider: Drew** (capacity call for the PM-stack program, not a lane call).
2. **The 2026-11-23 07:04 decision ("do not invent parent issues for F3/F4/P2; no valid parent homes for F3/F4/P2") contradicts the existing #12950 (F4, 10 children) and #12951 (P2, 1 child) homes created 2026-11-23 05:51 and read back live today.** Either the decision is superseded for F4/P2 (ratify the homes) or the 11 children move back. F3 still has no home under either reading. — **Decider: Drew** (decision record; a superseding decision comment unblocks S2 for F4/P2 either way).
3. **`design/<name>` labels for the parent designs** — reuse which existing labels, and who mints one if a family genuinely lacks it? — **Decider: Drew** (label-naming call; gates S4 only, never S2/S3).

## 9. Record

This dated doc (`doc/2026-11-24_design_id9068_design-spawn-backfill.md`) + source issue #9068 (link back from the issue) + queue task 21672 (complete on link + 9-point confirmation). Slices S2–S4 and all moves/rollup proofs are out of this doc's scope.
