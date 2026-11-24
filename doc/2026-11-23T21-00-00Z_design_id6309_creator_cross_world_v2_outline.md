# Creator tooling & insight — cross-world creator tools v2 outline (#6309)

- **Issue:** [#6309](https://github.com/developing-today/code/issues/6309), core topic [#6282](https://github.com/developing-today/code/issues/6282), Phase 14 plan of record [#6312](https://github.com/developing-today/code/issues/6312).
- **Predecessor:** [v1 owner basics design (#6308)](2026-11-18_design_id6308_creator_owner_basics_v1.md).
- **Date:** 2026-11-23.
- **Status:** refined outline for review. Outline only; no code in this record.
- **Entry condition:** verified met. Gate #6308 is CLOSED (closed 2026-11-18T12:33:29Z; the queue packet's 2026-11-23 date is wrong). The v1 design record above exists and answers the v1 scope questions, so the dependency is discharged.
- **Principles:** #1240 — local-first always; hostable by anyone at any scale; P2P behavior identical to hosted; our hosted instance is a public backstop, never a requirement.

## Decision in brief

V2 adds **author-scoped organization across the worlds and programs one author owns**: find, group and sort them, mark each as draft or published, and schedule announcements. All of it is computed from the author's own local worlds and the signed program bundles those worlds already hold. Nothing requires a central index, a hosted directory or a server we run. The same code path works for a self-hosted operator and for our hosted instance, and a peer with no network sees the same answers as one with a server.

Cross-world means _the author's own worlds on the machine or peer in question_. It does not mean a global catalogue. Discovery of other people's worlds stays with #6277 (discovery, registry and federation) and publishing stays with #6278 (program ecosystem). V2 consumes their outputs; it does not redefine them.

## 1. Scope and non-goals

**In scope**

- Author-scoped listing and search across the author's worlds and their programs, by name, tag, and recorded timestamps, from local data only.
- Draft / published state per program version, recorded as a journal fact in the world that owns it. A "published" state means a signed bundle exists and was offered for publication under #6278; it does not mean a hosted gallery entry exists.
- Scheduled announcements: an author can schedule a notice about a published program or world. Delivery goes through the F12.5 notify path named in the issue. The schedule itself is local data, so it survives offline and is evaluated by the author's own node.
- Read-only organization views (text and JSON, same schema discipline as the v1 owner report).

**Out of scope**

- Any global or hosted search index, leaderboard, trending list, or ranking of other authors.
- Any server-side scheduler that we operate. A schedule that cannot fire because the author's node is offline fires late, not from our infrastructure, and the record says so.
- Audience analytics beyond the v1 owner report. Scheduled announcements do not add tracking.
- New roles or sharing permissions. Who may see a draft is the existing world role model.
- Changing the publish or bundle format. #6278 owns that.

## 2. Answers to the scope questions the issue raises

- **Search and organization across worlds and programs.** Answered: a local index built from each world journal the author holds, rebuilt from the journal rather than stored as a second source of truth. A missing or stale index degrades to a directory walk with a labelled status, never to a guess.
- **Draft / published states.** Answered: two states per program version, written as journal facts. A third "scheduled" state is not a program state; it is a property of an announcement. Keeping it out of the program state stops a schedule from changing what a program _is_.
- **Scheduled announcements via F12.5 notify.** Answered in direction: the announcement record is local and durable; F12.5 provides the delivery channel. Open: whether F12.5's delivery is push-capable on a self-hosted node. Recorded as an open question, not assumed.

## 3. Principles check

| Principle (#1240)                    | How V2 satisfies it                                                                                             |
| ------------------------------------ | --------------------------------------------------------------------------------------------------------------- |
| Local-first always                   | All state is journal facts or a rebuildable local index.                                                        |
| Hostable by anyone, any scale        | Same code runs on a laptop and on a hosted node; only index size differs.                                       |
| P2P identical to hosted              | A peer with no hosted instance gets the same listing and states.                                                |
| Hosted is a backstop, never required | No feature reads from our instance. Publishing reaches galleries only through #6278 paths the operator chooses. |

## 4. Open questions and recommendations

1. **Is the local index a cache or a projection?** Recommendation: projection, rebuilt on demand from journals, with its build time reported. This avoids a second source of truth.
2. **Should "scheduled" live on the program or on the announcement?** Recommendation: on the announcement, as stated in §2.
3. **Does F12.5 notify deliver to a self-hosted node without our infrastructure?** Open. Needs a check against the F12.5 design before the delivery slice is filed. Deferred to that slice's first step, with no default assumed.
4. **Late firing when the node is offline.** Recommendation: fire on next start if the scheduled time has passed, and record the lateness. Do not silently drop it.
5. **Which timestamps are authoritative for "published at"?** Recommendation: the signed bundle's publication record, not the local clock.

## 5. Dependencies

- #6308 v1 owner report: steps 1–4 of its breakdown supply the journal read path and the authorization pattern V2 reuses. Hard dependency for the index.
- #6278 program ecosystem: bundle identity and publication record. Hard dependency for the published state.
- #6277 discovery: consumed only for "is this published elsewhere"; not required for V2 to work.
- F12.5 notify: delivery channel for announcements. Soft dependency; the schedule record can land before delivery exists.

## 6. Implementation breakdown (proposed follow-up issues)

Filed as child issues of #6309. Ordered by dependency; each keeps file ownership narrow.

1. **Author-scoped local index as a journal projection** — #13741. Read-only, rebuildable, reports build time and staleness. Depends on v1 step 1 read path.
2. **Draft / published journal facts per program version** — #13742. A new durable fact with an explicit migration note. Depends on #6278 publication record shape.
3. **Scheduled announcement record and local evaluator** — #13743. Durable schedule, fires on next start when late, records lateness. Delivery adapter left behind an interface until open question 3 is answered.
4. **Organization views (text and JSON) over steps 1–3** — #13745. Same schema-versioning discipline as the v1 owner report.

Steps 1 and 3 can run in parallel if their owned files are disjoint. Step 4 depends on 1–3. Delivery wiring for step 3 waits on open question 3 and is not in these four slices.

## 7. Acceptance for this design

- Entry condition recorded on #6309 with the correct gate-closed date (2026-11-18).
- This outline is linked from #6309 and from core topic #6282.
- Child implementation issues filed and linked from #6309.
- Consistent with the #1240 principles as recorded on #6312.
- Docs build for this record passes; exact command and counts are recorded on #6309 in the outcome note.
