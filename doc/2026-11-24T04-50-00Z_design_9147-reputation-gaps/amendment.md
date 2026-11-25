# #9147 amendment — gaps to the sufficient-design standard

- **Status:** proposed amendment, not implemented protocol. **Date:** 2026-11-24.
- **Parent:** [#9147](https://github.com/developing-today/code/issues/9147) — design(id): reputation system + public claims/dispute log.
- **Base design (merged):** `doc/2026-11-13T03-40-00Z_design_id-atlas-summit/`, PR #9238. The #9147 scope lives in Part B `versioned-design.md`, §V4 (reputation, takeover, public claims), §§82–94 (routing/reputation policy, claims-log privacy, retention/quarantine boundary), §120 (open questions with defaults), §§109–117 (executable invariants/test oracles).
- **Slices (filed):** #10185 (evidence-based ratings + how reputation affects prioritization/class changes, OPEN) then #10231 (public signed append-only claims/disputes log + right of reply, OPEN); #10152 abuse-protection research CLOSED. Order matters: #10185's ratings model is the input #10231's log outcomes consume.
- **Author:** queue task 21568, lane any-n (OpenChamber scheduled lane). Design only; no implementation.

## 9-point mapping

| #   | Standard point                                     | Base-design coverage                                               | This amendment                    |
| --- | -------------------------------------------------- | ------------------------------------------------------------------ | --------------------------------- |
| 1   | Problem + context, with evidence                   | V1 baseline + §architectural choice; B26 reputation-abuse research | — (covered)                       |
| 2   | Goals + explicit non-goals                         | Goals in §V4 + main.md ownership; non-goals implicit               | Non-goals stated explicitly below |
| 3   | Alternatives incl. do-nothing, honest tradeoffs    | Partial (CRDT "adopt none")                                        | Filled below                      |
| 4   | Chosen approach (interfaces, data model, security) | `Claim`/`Reply` schema, evidence classes, thresholds, rate limits  | — (covered)                       |
| 5   | Dependency-ordered slices w/ acceptance            | 32-slice manifest + filed issues                                   | Per-slice gates restated below    |
| 6   | Rollout + rollback                                 | Migration/compatibility; rollback unstated                         | Filled below                      |
| 7   | Verification plan                                  | Executable invariants + test oracles, `verification.md`            | One addition below                |
| 8   | Open questions, each with a named decider          | Questions + defaults (§120); deciders mostly unnamed               | Deciders assigned below           |
| 9   | Dated doc + tracking + queue item                  | This file; #9147; task 21568                                       | — (this record)                   |

## Non-goals (explicit)

- No globally authoritative reputation scalar. Ratings are local policy derived from evidence; portable only as signed claims, locally weighted, capped, freshness-decayed, separated by evidence class.
- No first-party privilege and no first-party adjudicator: a unilateral allegation never automatically alters any reputation, and unverified public claims/popularity carry no verdict weight.
- No Sybil/collusion/whitewashing elimination: documented as irreducible residual with stated trust assumptions and redress (corrections/appeals as new signed records).
- No perpetual-retention proof: a signed acknowledgment proves only its bounded assertion at that time, never later query eligibility.

## Alternatives considered (incl. do-nothing)

1. **Do-nothing (static trust classes only).** Keep directional peer classes (`fully_trusted | send_only | accept_signed_from`) with manual, operator-driven class changes and no behavioral ratings or claims log. Cheapest; zero new abuse surface. Rejected: trust cannot rise with good behavior, so high-performing peers never earn cheaper verification paths, and disputes have no record — the exact gap #9147 was filed to close. Kept as the universal fallback (see rollback).
2. **Centralized moderator verdicts.** A privileged role issues global reputation verdicts. Rejected: single point of compromise and of coercion; contradicts the no-first-party-adjudicator constraint; global scalar becomes a target.
3. **Imported web-of-trust (third-party attestations as primary input).** Faster bootstrap from existing attestations. Rejected as primary: external attestations are unverifiable-by-us evidence with unknown freshness; admitted only as signed portable claims subject to the same local weighting, caps, and decay — a complement, never the base.
4. **Claims log without reputation coupling.** Ship the append-only log purely as a publication/audit record with no consumer in local policy. Viable staging subset (and the safe shape of an early #10231), but without the #10185 ratings consumer the log cannot adapt prioritization — half the issue's acceptance. Adopted as the rollout order, not the end state.

## Rollout + rollback

Order: #10185 (ratings model + thresholds as proposed starting defaults) → #10231 (log), with the log initially shipping in alternative-4 shape (publication-only, no policy consumer) behind a default-off flag that binds log outcomes to local rating weights.

- **Per-slice gates:** #10185 lands only with the hysteresis/quarantine thresholds as configuration (no hardcoded weights) plus a simulation showing no oscillation on a scripted event sequence; #10231 lands only with redaction-correctness proof (public commitment leaks no content; redaction never implies guilt) and the 3-claims/day + 64 KiB rate/size enforcement tests.
- **Rollback (per slice, no history rewrite):** ratings rollback = set all rating weights to neutral → system behaves as alternative 1 (static classes), no code revert required. Log rollback = stop appending and mark the log quiescent; the log is append-only so rollback never edits or deletes records — consumers pin the last-good policy digest and ignore later entries. Emergency path (compromised key/peer) is the V2 disable: block new acceptance/relay immediately, quarantine affected records, preserve bytes.
- **What rollback never does:** rewrite a verdict, delete a claim, or infer omission from absence.

## Open questions — named deciders

| Question (base §120 + §V4)                                                                           | Default (unchanged)                                                                  | Decider                                                                 |
| ---------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------ | ----------------------------------------------------------------------- |
| Exact historical revocation policy                                                                   | Quarantine affected interval, preserve bytes; signer wall-clock proves nothing       | Drew (product/policy) on #9147; #9111 owner for the config shape        |
| Dedup TTL/capacity                                                                                   | Finite config, bounded hard maximum, request-binding tombstone through origin expiry | #9111 design owner                                                      |
| Whether complete responses are allowed                                                               | Opt-in                                                                               | #9111 design owner                                                      |
| #9084 transfer/retention boundary                                                                    | Only digests/references until agreed                                                 | #9084 owner; Drew breaks ties                                           |
| Threshold tuning (quarantine/review triggers, decay bands, 3/10-min and 3-of-5/5-consecutive values) | Proposed starting defaults, not universal weights                                    | Drew, on measured evidence from #10185's simulation + first live window |
| Evidence-class weights and per-identity contribution caps                                            | Capped, decayed, separated by class                                                  | Drew, same venue                                                        |
| Original Part B candidate provenance                                                                 | See source log (`inventory-live/candidate-provenance.md`)                            | Design-mill record; informational only                                  |

No question blocks #10185/#10231 starting under the defaults; threshold/weight choices are revisited with live data, never silently.

## Verification (addition to base oracles)

Base oracles stand (envelope PBT, claims/evidence oracle, contradiction-evaluator scoping). One addition for this scope: a deterministic threshold simulation — a scripted sequence of attributable events, timeouts, and recoveries must produce exactly the specified quarantine/review/restore transitions (including hysteresis non-oscillation and retry-count-once semantics) before #10185 merges, and the same harness re-runs against live-derived parameters at each tuning change.
