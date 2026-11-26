# Creator tooling & insight — public-world safety v3 outline (#6310)

- **Issue:** [#6310](https://github.com/developing-today/code/issues/6310), core topic [#6282](https://github.com/developing-today/code/issues/6282), Phase 14 plan of record [#6312](https://github.com/developing-today/code/issues/6312).
- **Predecessors:** [v1 owner basics design (#6308)](2026-11-18_design_id6308_creator_owner_basics_v1.md); [v2 cross-world outline (#6309)](2026-11-23T21-00-00Z_design_id6309_creator_cross_world_v2_outline.md).
- **Date:** 2026-11-24.
- **Status:** refined outline for review. Outline only; no code in this record.
- **Entry condition:** verified met. Gate #6309 is CLOSED (closed 2026-11-24T09:13:10Z as implemented; v2 outline landed on main via PR #13749, commit `e9dfdca9e07`). The v2 open questions are answered with recommendations except Q3 (F12.5 push delivery to a self-hosted node), which is explicitly deferred to that slice's first step with no default assumed — so the "answered or explicitly deferred" condition is discharged. Gate #6308 is CLOSED (2026-11-18).
- **Principles:** #1240 — local-first always; hostable by anyone at any scale; P2P behavior identical to hosted; our hosted instance is a public backstop, never a requirement.

## Decision in brief

V3 adds **per-operator public-world safety tooling**: reports against registry-listed and hosted public worlds land in a local review queue, a moderator works the queue through the existing world role model, every action appends to a transparency log, and every affected party can appeal through the same queue. All of it is computed from data the operator's own node already holds. Nothing requires a central moderation authority, a network-wide takedown switch, or a server we run. Our hosted instance moderates only its own hosting — it is one operator among many, and its decisions never travel as protocol truth.

"Safety" here means _operator tooling for the operator's own worlds and listings_, not a network safety guarantee. A peer with no hosted instance gets the same review queue, the same log shape, and the same appeals path over the worlds it operates.

## 1. Scope and non-goals

**In scope**

- Reporting: any participant (or the operator's own heuristics) can file a report against a registry-listed or hosted public world or program. A report is a signed, timestamped local fact naming the target, the reason class, and the reporter; it is not a verdict.
- Review queue: reports land in a per-operator, locally ordered queue. Ordering is by the operator's own stated policy (e.g. severity class, then age), computed from local data only.
- Moderation actions through the existing world role model: the v1 audit discipline applies unchanged — each action is authorized by the existing role/scope checks and durably records actor, target, action, timestamp/order, and outcome (v1 design §5).
- Transparency log: an append-only, replayable per-operator log of moderation actions and their outcomes, with a read-only view. Same journal-fact discipline as the v1 moderation audit; the log is the operator's own record, not a shared ledger.
- Appeals: an affected party can appeal an action; the appeal re-enters the same review queue as a distinct item linked to the original action. An appeal never auto-reverses; it is decided by a moderator through the same action path, and the decision is logged the same way.

**Out of scope**

- Any network-wide moderation authority, global blocklist, reputation score, or cross-operator verdict propagation.
- Automated takedowns or classifier-driven enforcement. Reports may be filed by heuristics, but every action is a moderator decision through the role model.
- New roles, delegated moderation, or configurable moderation rules — v1 explicitly deferred these to their own governance design, and this outline does not reopen that.
- Changing the registry listing protocol (#6277) or the bundle/publication format (#6278). V3 consumes their outputs; it references listings, never rewrites them.
- Audience analytics or reporter profiling beyond what the queue needs to function.

## 2. Answers to the scope questions the issue raises

- **Reporting and review queues for registry-listed and hosted public worlds.** Answered: reports are local signed facts against targets the operator lists or hosts; the queue is a local projection over those facts, rebuilt from the journal rather than stored as a second source of truth (same projection discipline as the v2 index). Registry listing (#6277) determines _eligibility_ for reporting, not the outcome.
- **Transparency log of moderation actions.** Answered: the v1 durable moderation record (§5 of the v1 design) is the log's foundation — actor, target, action, timestamp/order, result, replayable. V3 adds the read-only view and the policy that every safety action, including appeal decisions, lands in it. No separate audit database (v1 non-goal, kept).
- **Appeals.** Answered in shape: an appeal is a queue item linked to the logged action it contests, decided through the same authorized action path, with the decision appended to the same log. Open: the exact link representation between appeal and action (reference by log order vs. content hash). Recommendation: content hash of the logged action, so the link survives log compaction if a compaction scheme is ever adopted; recorded as an open question for the appeals slice, not assumed.
- **Safety boundaries for published creator content.** Answered: a "published" program (v2 draft/published facts) that is reported does not change state while under review — the report is metadata on the listing/queue, never a mutation of the program. Only a moderator action through the role model changes anything, and only within the operator's own scope (their worlds, their listings).

## 3. Principles check

| Principle (#1240)                    | How V3 satisfies it                                                                                                           |
| ------------------------------------ | ----------------------------------------------------------------------------------------------------------------------------- |
| Local-first always                   | Reports, queue order, log, and appeals are journal facts or rebuildable local projections.                                    |
| Hostable by anyone, any scale        | Same code runs on a laptop and on a hosted node; only queue/log size differs.                                                 |
| P2P identical to hosted              | A peer with no hosted instance gets the same queue, log shape, and appeals path over the worlds it operates.                  |
| Hosted is a backstop, never required | No feature reads from our instance. Our hosted moderation covers only our own hosting and never propagates as protocol truth. |

## 4. Open questions and recommendations

1. **Appeal-to-action link: log order or content hash?** Recommendation: content hash of the logged action (see §2). Deferred to the appeals slice with no default assumed in any shared schema.
2. **Report reason classes: fixed enum or free text + class?** Recommendation: a small fixed enum (e.g. spam, abuse, impersonation, other) plus free-text detail. The enum drives queue ordering; the text is never used for automated decisions.
3. **Who may read the transparency log?** Recommendation: the operator's existing owner/admin read path (same sensitivity rule as v1 §7 — moderation records are sensitive world data). Public excerpts, if ever wanted, need their own privacy design; not in these slices.
4. **Do reports expire?** Recommendation: reports never auto-expire or auto-resolve; only a moderator decision closes them, and the closure is logged. Stale-report triage policy is operator configuration, not protocol.
5. **How does the queue compose with scheduled announcements (v2)?** Open, deferred: whether a program under review pauses its scheduled announcements is operator policy. The default in these slices is no coupling — announcements fire as scheduled; a moderator may act separately. Revisit only if an operator asks for it.

## 5. Dependencies

- #6308 v1 moderation authority and audit (§5 of the v1 design): the authorization path and durable actor/target/action record V3 reuses. Hard dependency — already landed.
- #6309 v2 draft/published facts: the "published" state reports attach to. Hard dependency for report targeting — already landed.
- #6277 discovery/registry: listing eligibility for reporting. Consumed, not changed; soft dependency.
- #6278 program ecosystem: bundle identity for report targets. Consumed, not changed; soft dependency.
- F12.5 notify (v2 open Q3): only if a future slice wants reporter/appealant notifications. Not in these slices.

## 6. Implementation breakdown (proposed follow-up issues)

To be filed as child issues of #6310. Ordered by dependency; each keeps file ownership narrow.

1. **Report fact and intake** — signed, timestamped report records against operator-listed/hosted targets; validation (target exists in the operator's scope) at intake. Depends on #6277 listing shape and v2 published facts.
2. **Review queue projection** — local, rebuildable, policy-ordered queue over report facts; reports close only by moderator decision. Depends on slice 1.
3. **Transparency log view** — read-only view over the v1 durable moderation record plus appeal decisions; replayable, same sensitivity rules as v1 §7. Depends on v1 audit path; parallel-safe with slices 1–2 if owned files are disjoint.
4. **Appeals path** — appeal as a queue item linked by action content hash, decided through the same authorized action path, decision appended to the same log. Depends on slices 1–3 and the open-question-1 decision.

Slices 1 and 3 can run in parallel if their owned files are disjoint. Slice 4 depends on 1–3. No notification wiring (F12.5) is in these four slices.

## 7. Acceptance for this design

- Entry condition recorded on #6310 with the correct gate-closed date (2026-11-24).
- This outline is linked from #6310 and from core topic #6282.
- Child implementation issues filed and linked from #6310.
- Consistent with the #1240 principles as recorded on #6312.
- Docs build for this record passes; exact command and counts are recorded on #6310 in the outcome note.
