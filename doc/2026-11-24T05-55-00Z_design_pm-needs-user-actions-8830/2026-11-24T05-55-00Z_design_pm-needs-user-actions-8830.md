# PM-stack needs-user action model — nine-point design record

**Date:** 2026-11-24 06:00 UTC  
**Tracking:** [developing-today/code#8830](https://github.com/developing-today/code/issues/8830) · register P1.4 in `pm-stack/design-register-2026-11-11.md`  
**Status:** Design and contract slices shipped; this record upgrades the existing living design to the repository's sufficient-design standard. It does not claim that every proposed interaction is present in the current implementation.

**Primary design:** [`pm-stack/needs-user-actions-design-2026-10-26.md`](../../../pm-stack/needs-user-actions-design-2026-10-26.md). That document remains the detailed schema, action, publisher, rendering, and test contract. This dated record adds the missing problem framing, alternatives, ordered delivery/acceptance, rollout, verification, and named open questions. Where the detailed design's earlier text conflicts with its 2026-10-27 decision update, the decision update wins. Where current shipped behavior conflicts with either design, current behavior is described as implementation evidence—not silently redefined by this record.

## 1. Problem and context

The PM stack's needs-user queue must distinguish work the orchestrator can complete from actions that genuinely require a person. The 2026-10-26 design was motivated by a published orchestration section whose needs-Drew items had only identity, kind, title, source, reason, timestamps, and occasional references/horizons/blockers; they had no structured choices, surveys, or delegation state. The dashboard consequently could explain why an item was waiting without reliably offering a bounded, auditable way to answer, delegate, or resolve it.

The existing system already has the relevant seams: `TierItem` and `OrchestrationSection` in the aggregator, `/attention` card rendering, and the shared `DashAction` store/consumer model. The design extends those seams additively. It must not make a malformed optional field invalidate the whole feed, let the orchestrator exercise human-only access or policy authority, or let delegated work disappear without a visible terminal outcome.

The detailed design records the original data-shape observations and source files in its “Grounding” section. A later current-state review (`doc/2026-11-24T01-11-42Z_research_pm-book-programme-13900/survey/disagreements.md`, D13) notes that shipped Needs User buckets and labels differ from the 2026 vocabulary; live code is authoritative for current behavior.

## 2. Goals and non-goals

### Goals

- Every needs-user item explains why a person is needed and what happens if it remains unattended.
- Offer a consistent set of standard actions, plus item-specific choices and questions where useful.
- Support manual, first-pass-with-approval, and authorized auto delegation on the same item identity.
- Keep proposals, inability, pending actions, and outcomes visible and auditable.
- Preserve compatibility: optional item extensions are validated per item and fail soft; old items still render.
- Keep human-only access and policy actions outside auto delegation, and require reasons for dismiss/reject.

### Non-goals

- Replacing the shared action envelope, orchestration feed, or existing attention/session models.
- Granting new authority, spending, credentials, or external side effects to the orchestrator.
- Turning every needs-user item into a survey or forcing custom options when no genuine alternatives exist.
- Reworking the whole PM stack, generic triage verbs, status requests, or session actions covered by other issues.
- Claiming that a design issue's closure or a tracking document proves production behavior; runtime evidence remains separate.

## 3. Alternatives considered

1. **Do nothing / rely on free-text reason and generic answer.** Lowest cost and backward-compatible, but leaves users to infer consequences and choices; delegation remains opaque or informal.
2. **Add page-local buttons and a second action store.** Easy to prototype, but duplicates idempotency, authorization, audit, and consume-once behavior; rejected in favor of the existing `DashAction` envelope.
3. **Put new fields in the section-level schema gate.** Simpler validation, but one malformed item could reject the whole feed (the failure class the existing fail-soft contract is intended to avoid); rejected in favor of per-item sanitization.
4. **Create separate queue items for delegated work or proposals.** Makes progress superficially visible, but splits identity and can duplicate/lossily reconnect the original decision; rejected in favor of delegation/proposal fields on the original item.
5. **Allow auto delegation for all item classes.** Maximizes automation but would pretend the orchestrator has user-only access or policy authority; rejected. `access` and `policy` remain hard exclusions.

Chosen approach: additive per-item contract plus shared actions, honest UI, and bounded delegation state transitions. See the primary design §§1–4 for exact fields and payloads.

## 4. Chosen approach and contracts

- Extend `TierItem` with optional `whyDrew`, `summary`, `ifIgnored`, `options[]`, `questions[]`, and `delegation`; do not change the section version or section-level validator. Sanitize extensions per item and drop malformed extensions rather than the item/section.
- Render a mobile-first card with class, age, reason, consequences, item options/questions, proposal or inability state, and standard actions. Keep controls usable without hover and show prohibited auto actions disabled with the reason.
- Send actions through the existing `DashAction` envelope with idempotency, consume-once result handling, superseding/undo semantics, and the established audit trail. Validate option/question references against the currently published item.
- Keep one stable item identity through delegation. First-pass drafts do not apply effects; approval is required. Auto may perform only already-authorized work and must return a structured `unable` when blocked. A delegated item must reach a visible outcome within two orchestration runs.
- Require a reason for dismiss/reject and resurface dismissed items only after a material new fact, with the reason marked as resurfaced.
- **Decision history:** the detailed design's 2026-10-27 update records the standing auto default ON for decision/missing-info/approval, with access/policy always excluded; this supersedes the earlier per-item-opt-in-only proposal. Current implementation must still be checked against that decision before making a runtime claim.

## 5. Dependency-ordered slices and acceptance

The original implementation issues are #1290–#1293, all recorded as closed on #8830. These slices describe the contract and the evidence each should provide; their closed state alone is not a test result.

1. **Publisher/item contract (#1290).** Add optional fields and publisher self-checks. Acceptance: old and extended sections both pass the unchanged section gate; invalid required extensions fail/drop at item scope; access/policy never publish auto; a working delegation older than two runs cannot silently republish.
2. **Shared actions and attention controls (#1291, coordinated with the existing actions-store work such as #1152).** Add controls using the shared envelope. Acceptance: reason-required operations reject missing reasons; invalid option/question references reject; duplicate client tokens and duplicate result posts apply once; existing cards remain functional.
3. **Delegation execution (#1292; depends on publisher contract and action verbs).** Implement first-pass and auto transitions. Acceptance: first-pass produces a non-applying proposal; auto either resolves within authority or returns tried/needs/reason; access/policy auto is rejected; stalled work becomes unable within two runs.
4. **Proposal review and race hardening (#1293; depends on the action/UI and delegation paths).** Handle expiry, approval/rejection races, and cross-surface counts. Acceptance: proposal identity remains attached to the original item; stale proposals cannot apply; approval applies at most once and records the action id; rejection requires a reason.

A slice is complete only with its issue-specific acceptance evidence and reviewable implementation. Do not infer the runtime command or test result from this design record.

## 6. Rollout and rollback

1. Deploy optional publisher fields first; old aggregators ignore unknown optional properties and existing publishers remain valid.
2. Deploy per-item sanitization and rendering with the shared action store. Keep malformed extensions fail-soft and monitor feed/action errors before enabling new publisher output broadly.
3. Enable delegation modes only after the consumer and its self-checks are deployed. Preserve the recorded standing-default decision, but verify the live toggle/configuration before rollout; never enable auto for access/policy.
4. Roll back by disabling new publisher fields and delegation defaults first, then reverting UI/consumer changes if needed. Existing section shape and action store remain the compatibility boundary; do not delete action history or rewrite item identity during rollback.
5. On a bad action or partial rollout, stop the consumer from applying new effects, retain the audit records, and return affected items to a visible manual state. Any already-applied external effect uses its own compensating procedure; this design does not promise generic reversal.

## 7. Verification plan

- **Contract:** fixtures prove the unchanged section validator accepts old/new item shapes; sanitizer tests prove malformed fields cannot discard an item or section.
- **Actions:** test required reasons, current-item option/question membership, access/policy restrictions, idempotency, consume-once results, and superseding behavior.
- **UI:** render tests cover standard actions, options, surveys, pending/applied states, proposals, inability, disabled prohibited controls, mobile target sizes, and one-card/one-item identity.
- **State machine:** model/property tests cover allowed transitions, single item identity, no first-pass side effect before approval, and the two-run terminal bound. Oracle: the transition table in primary design §§2 and 4.3; invalid transitions must be rejected or become visible unable states.
- **Live proof:** for each enabled mode, retain a redacted run record showing either completion or a structured inability, and verify the action id appears in the audit trail. Never include credential values in logs or evidence.
- **Regression:** run the narrow aggregator/action test suites and existing feed/attention checks; verify a prior-shape fixture still renders. Record exact commands and results with the implementation PR, not as assumed facts here.

## 8. Open questions (with deciders)

1. **Which behavior is still incomplete in production despite #1290–#1293 being closed?** Decider: PM-stack implementation owner (dezren39); evidence required: code/test/live-run pointers for each slice. Until reconciled, this document is a design contract, not a shipped-feature certification.
2. **Does the current dashboard's bucket/label vocabulary intentionally supersede `needsDrew` in the old design, and what is the canonical public naming?** Decider: PM-stack product owner (Drew); default: use current Needs User labels and preserve legacy wire fields until an explicitly versioned migration.
3. **Is the recorded standing auto default still enabled, and where is its authoritative preference stored?** Decider: Drew; default for any new deployment is no new auto behavior until the existing recorded decision is verified in the live preference path; access/policy remain excluded regardless.
4. **What compensating action is required for each effect that auto mode can apply?** Decider: owner of that effect; default: only allow effects with an existing documented compensating procedure or established idempotent completion path.

## 9. Tracking and record

- Source/tracking issue: [#8830](https://github.com/developing-today/code/issues/8830).
- Detailed living design and contracts: [`pm-stack/needs-user-actions-design-2026-10-26.md`](../../../pm-stack/needs-user-actions-design-2026-10-26.md).
- Register: `pm-stack/design-register-2026-11-11.md`, entry P1.4; design-tracking manifest records the original document/contract lineage.
- Contract implementation issues: [#1290](https://github.com/developing-today/code/issues/1290), [#1291](https://github.com/developing-today/code/issues/1291), [#1292](https://github.com/developing-today/code/issues/1292), [#1293](https://github.com/developing-today/code/issues/1293).
- Nine-point checklist: problem/context ✓; goals/non-goals ✓; alternatives ✓; approach/contracts ✓; dependency-ordered slices with acceptance ✓; rollout/rollback ✓; verification ✓; named open questions ✓; dated record/tracking/queue context ✓.
