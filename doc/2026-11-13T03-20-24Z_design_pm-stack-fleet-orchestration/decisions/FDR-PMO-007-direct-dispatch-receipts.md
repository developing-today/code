# FDR-PMO-007 — Direct dispatch and durable launch receipts

- **Date:** 2026-11-13
- **Status:** accepted — local design contract only, not production authorization
- **Adopted:** Summit owner, task 15381, 2026-11-13; see `../reviews/component-contracts.md`. No live launch is authorized; #6533 remains separate.
- **Owner:** Summit design owner, task 15381 / session `ses_e5e2d10e06effeEylSDDVaN0PL` for this proposal; future maintenance by Fleet maintainers through #9180.
- **Decision maker:** Delegated Summit architecture owner for this design pass; adoption follows independent final design review by the parent and Fleet owner recording through #9180. No human name is inferred.
- **Scope:** authenticated command/event routing, direct dispatch, provider adapters, durable evidence of launch/bind/progress
- **Supersedes:** none
- **Superseded by:** none
- **Related records:** FDR-PMO-001, -003, -004, -006

## Context and problem

An actionable authorized comment or PM action should reach the current bound session, or directly launch eligible parked/prepared work where policy allows, rather than merely changing ordinary queue rank. Delivery spans event intake, authorization, queue reservation, provider launch, task/session binding, prompt delivery, and acknowledgment; these are separate remote effects. #8187 is a code baseline for launch-on-handoff/inline triage, not full webhook or parked-item direct launch. #6533 remains a distinct live launch-and-bind acceptance gap. The observed PM dispatch PostgREST bypass is tracked by #9028 and blocks claims of canonical reservation until reconciled.

Sources: [architecture, “Architectural choice,” “Identity and authority,” “Shared command and result interface,” and “Lifecycle and cross-system reliability”](../architecture.md); [source 01 lines 13, 30–49, 53–65](../sources/01_handoff_pm_stack_fleet_orchestration.md); [source 02 §§7–8, 10, 16–17](../sources/02_charter_v1_1.md); [#9203, “Non-negotiable boundaries,” “V1 — observed baseline,” and “Direct-session routing and launch reservation”](../components/9203-direct-dispatch-slash-commands-event-routing.md); [#9201, “Requirement and decision summary,” “Intent and effect are distinct,” and “Action request / result contract”](../components/9201-pm-projection-mobile-control.md); [Track A census, “Track A evidence matrix” and “Issue / collision dispositions”](../census/track-a.md).

## Options considered

1. **Route typed authenticated intent through shared Fleet operation boundary; reserve canonically and report staged receipts (recommended).** Uses existing queue/dispatch/provider seams, exposes uncertainty and avoids duplicate launches.
2. **Only raise queue priority and wait for ordinary selection.** Does not satisfy direct authorized routing for live comments or parked-item requests and confuses urgency with dispatch intent.
3. **Launch directly from webhook/UI to provider.** Low latency but bypasses canonical lease/eligibility/authorization and can double-start on retries.
4. **Treat provider HTTP success/session ID as launch complete.** Convenient but does not prove task binding, prompt delivery, acknowledgment, or meaningful progress.
5. **Do nothing.** Preserves current partial baseline but leaves direct instruction and live proof unmet.

**Tradeoff comparison:** Shared reservation plus staged receipts adds state-machine work and some launch latency, but improves reliability under retries and extends to provider adapters without new infrastructure. Direct provider launches can be fast and horizontally scalable, yet lose canonical claim safety and duplicate-start protection. Priority-only deferral is simplest but fails prompt routing. Treating HTTP 2xx as completion is low-complexity but observably unreliable. Current live capacity/SLA measurements are absent; no scale claim is made.

## Proposed operational decision

Normalize GitHub webhook comments, PM actions, and CLI/service requests to common `Command`/`Result`. For webhooks: bound/read raw bytes, verify signature and event schema, derive delivery identity, authenticate/authorize actor and target, resolve unambiguous canonical work, parse strict allowlisted grammar, dedupe, recheck operation permission, and durably reserve before external action. Event/comment text is untrusted data; never execute shell or treat a classifier as authority. Edited commands are new versioned candidates; do not silently rerun prior command.

For an active task, resolve authoritative current binding and supported adapter capability; send instruction with operation correlation ID and record delivery vs acknowledgment separately. For a parked/new session, authorize direct start, apply #9178/#9179 manual/takeover/park rules, hard dependency and current lease/concurrency checks, then reserve via canonical claim/bind protocol. `do_now` is a one-shot directive only; it does not rewrite priority, bypass `manual_only`, or bypass eligibility/auth/lease/capacity. Existing #8187 behavior is reused, not mistaken for full event routing.

`manual_only` prevents automatic claim. Only an explicit, authenticated manual/owner operation permitted by #9178 may request such work; Do Now alone does not clear the persistent flag or supply owner override authority.

Return durable stage receipts separately: accepted, launch request accepted, session created, task bound, prompt delivered, acknowledged, meaningful progress, terminal verification. On ambiguous timeout query/reconcile operation/task/session identity before retry; never blind-start. Provider adapter capability matrix advertises only verified operations. Slack/channel post means transport delivery, not session acknowledgment.

## Rationale and evidence boundary

**Facts:** census identifies existing dispatch client and PM service behavior, the PostgREST bypass, #8187 baseline and open #6533 proof. Additional pinned-source inspection confirms the projector already verifies bounded raw-body HMAC-SHA256 and delivery IDs and stores disposable projection events/hot checkpoints. This does not establish Fleet command acceptance, actor/field authorization, operation receipt or launch. No live dispatch is authorized or performed by this record. Provider behavior beyond inspected adapter capabilities is unverified.

**Inference:** a durable reservation plus evidence stages offers idempotent recovery without pretending distributed launch is atomic. A route failure should remain pending/uncertain rather than claim no launch or launch a duplicate.

## Consequences, compatibility, and migration

- #9028 producer reconciliation is a hard integration gate for direct dispatch that claims canonical task reservation; versioned offline contracts may proceed meanwhile.
- #9178 claim/manual-only/takeover and #9179 park rules are consumed, not overridden. Capacity/provider-specific constraints remain adapter-owned and observable.
- Live #6533 test is separately authorized/recorded; mocks and merged fixes cannot substitute for actual prompt-delivery, task-binding, and meaningful-start evidence.
- Keep existing fleet dispatch and provider seams; no new webhook service, queue, scheduler, agent system, or dispatch DB.
- No launch, session, webhook mutation, or production routing change authorized here.

## Negative cases and test oracles

- **Webhook replay oracle:** same provider delivery ID is no-op; same semantic comment command via distinct delivery route dedupes by source event/revision and operation identity; unauthorized replay reveals no prior receipt.
- **Parser oracle:** invalid/ambiguous trailing command, shell metacharacters, or bare cross-repo number rejected before side effects.
- **Reservation oracle:** active conflicting lease, hard blocker, manual_only policy, or capacity failure prevents start and returns reason; Do Now cannot bypass.
- **Lost acknowledgment oracle:** launch applies then times out; lookup by operation/task/session reconciles before any retry; no second launch.
- **Receipt oracle:** adapter accepted, session created, bound, prompt delivered, acknowledged, and meaningful progress are distinct states; session shell alone never counts as active work.
- **Integration oracle:** #6533 live acceptance requires actual launch-and-bind evidence under authorized environment; no mock can pass that gate.

## Revisit triggers

Reopen if #9028 establishes a different supported producer path, #9178/#9179 rulings change, provider adapter changes acknowledgment/bind guarantees, live #6533 exposes protocol gaps, or a measured dispatch SLA/capacity requires additional infrastructure. Any such path must retain dedup, policy, and receipt boundaries.

## Implementation, research, and source links

- Existing: [#9203](https://github.com/developing-today/code/issues/9203), [#9201](https://github.com/developing-today/code/issues/9201), [#9028](https://github.com/developing-today/code/issues/9028), [#8187](https://github.com/developing-today/code/issues/8187), [#6533](https://github.com/developing-today/code/issues/6533), [#9178](https://github.com/developing-today/code/issues/9178), [#9179](https://github.com/developing-today/code/issues/9179).
- Reuse evidence: `pm-stack/projector/src/webhook.ts:7–57`, `pm-stack/projector/src/server.ts:147–201`, `pm-stack/projector/test/webhook-delivery-id.test.ts:90–154`, and `pm-stack/projector/README.md:3–8,23–34,145–179`. Projector state is disposable projection data; ACK is not canonical Fleet operation acceptance.
- Existing granular component keys: #9203 `DSP-01`–`DSP-10`; #9201 `MOB-07`; #9209 `REL-01/03/06`; #6533 is the existing live acceptance fixture, not a new key/issue. #9028 remains the producer-reconciliation gate.
- Pinned baseline `a038f084dbeb525494d7bb928ad91f597db2ac51`; evidence summarized in Track A census and #9203.
