# FDR-PMO-001 — Canonical broker and distributed command seam

- **Date:** 2026-11-13
- **Status:** accepted — local design contract only, not production authorization
- **Adopted:** Summit owner, task 15381, 2026-11-13; see `../reviews/component-contracts.md`. Global registration through #9180 remains separate.
- **Owner:** Summit design owner, task 15381 / session `ses_e5e2d10e06effeEylSDDVaN0PL` for this proposal; future maintenance by Fleet maintainers through #9180.
- **Decision maker:** Delegated Summit architecture owner for this design pass; adoption follows independent final design review by the parent and Fleet owner recording through #9180. No human name is inferred.
- **Scope:** canonical work-state and claim authority; ingress from PM, GitHub, CLI, and hosted producers
- **Supersedes:** none
- **Superseded by:** none
- **Related records:** FDR-PMO-002, -003, -006, -007

## Context and problem

The charter requires a coherent queue authority across distributed producers and projections without adding a second queue, scheduler, database, or PM authority. At the pinned baseline, Track A census reports atomic same-row claim behavior and one-queue filter semantics, but also finds PM dispatch enqueue/direct task PATCH through Supabase/PostgREST; #9028 tracks that bypass. Deployment liveness is not established. “One authority” is therefore the target contract, not a claim all V1 writers already comply.

Sources: [architecture, “Architectural choice” and “Identity and authority”](../architecture.md); [source 01 lines 30–43, 69–73](../sources/01_handoff_pm_stack_fleet_orchestration.md); [source 02 §§1–2, 6–8](../sources/02_charter_v1_1.md); [Track A census, “Track A evidence matrix” and “Evidence limits”](../census/track-a.md); [#9200, “V1 — observed producer/consumer and state baseline” and “Authority and precedence table”](../components/9200-priority-distribution.md); [#9203, “Non-negotiable boundaries,” “V1 — observed baseline,” and “Direct-session routing and launch reservation”](../components/9203-direct-dispatch-slash-commands-event-routing.md); [prior art A, “Four distinct architecture shapes” and “Cross-cutting design analysis”](../research/track-a-state-of-art.md).

## Options considered

1. **Existing broker as canonical writer; authenticated command ingress and versioned read projections (recommended).** One place validates accepted state and claim; clients remain producers or projections.
2. **Multi-master writes with timestamps/LWW or CRDT merge.** Avoids a synchronous central write path, but human intent, source authority, clock skew, and stale writes cannot be safely resolved by arrival time; claim ordering would not have one authority.
3. **New queue/database/event bus or workflow service.** Could add independent throughput or durable execution, but duplicates authority and expands operations/migration without measured scale evidence requiring it.
4. **Do nothing.** Retains current behavior and bypass; incompatible producer paths continue to risk divergent work state and claim semantics.

**Tradeoff comparison:** Option 1 keeps one authority and the fewest moving parts; its single-writer throughput/availability ceiling must be measured, not guessed. Options 2–3 may scale ingress or writes horizontally, but introduce conflict/reconciliation or another operational authority and weaken claim consistency. Option 4 has the least migration cost today but retains known reliability/authority divergence. No benchmark currently proves the broker is a bottleneck; extensibility comes from typed adapters, not peer stores.

## Proposed operational decision

Use the existing Fleet broker/authority store as the sole target for canonical work-state mutations and claims. Accept typed, authenticated, field-scoped commands from multiple sources; persist accepted decisions with operation/source identity and return versioned observations. PM Project remains orchestrator-written projection; GitHub/provider-owned fields remain under their respective owners. A producer is not authorized to write directly merely because it has a valid identity or currently reaches a database.

Treat #9028 as a required reconciliation input before canonical dispatch cutover. Repoint or quarantine the PostgREST producer; until then, describe actual behavior as nonconforming and deployment status as unknown. A temporary compatibility adapter may forward only to the canonical broker, preserve operation/dedup identity, and verify authority/task/session binding. It must not dual-write, independently mutate canonical task state, or fail back to the bypass if broker forwarding fails. Return unavailable/uncertain and retain recoverable intent instead.

The one-queue matching contract is conjunctive: task-declared requirements **and** caller's explicit filter must both pass; each empty predicate is true. Identity is recorded and is not an implicit filter. Claims revalidate at the authoritative transaction boundary (FDR-PMO-003).

For new `work-command/v1` request fingerprints, use RFC 8785 JCS then SHA-256 with a purpose/version domain separator over a validated, default-expanded typed intent DTO. Include schema version, authority/work pair, kind, typed arguments, expected work version, lease generation, observed snapshot and reason. Exclude operation/key/delivery IDs, receipt times, formatting, wait/timeout and dry-run mode; scope idempotency by authenticated principal and authority and recheck authorization independently. Reject duplicate JSON properties, invalid Unicode, non-finite values and lossy numeric IDs before hashing; preserve strings without Unicode normalization. This applies only to the new protocol and does not migrate or equate legacy creation-outbox hashes. Shared CLI-02 owns cross-language golden vectors; this record adds no hash service.

## Rationale and evidence boundary

**Measured/source facts:** at pinned SHA `a038f084dbeb525494d7bb928ad91f597db2ac51`, the census cites atomic conditional claim in `turso_queue.rs:4210–4272`, requirements plus caller filter at `:3110–3129`, and the PM dispatch direct PostgREST path. #9028 is the recorded bypass issue. No live production route was tested by this record.

**Inference:** broker authority plus command ingress is the smallest design consistent with one-queue behavior and the known code seams. It makes one write boundary a potential throughput/availability bottleneck; no measured load or latency supplied here justifies a second authority. Queue outage fails closed for canonical writes rather than silently changing authority.

## Consequences, compatibility, and migration

- All mutation producers need auth, field/action permissions, expected revision, idempotency, and typed failure/uncertain receipts. Read projections may be stale but cannot grant a claim.
- #9028 reconciliation precedes direct-dispatch cutover. During transition, explicitly identify any legacy writer; do not claim single authority until removed or quarantined.
- Schema ownership is not decided here. Reconcile #3424/#8575/#8579 and QTI first; QTI remains unlocated/unverified and #9241 owns the current bounded crosswalk. Offline contract/model work is not blocked by that search; schema-coupled implementation is.
- No production change, queue migration, new service, or new outbox producer is authorized. The existing creation-outbox pilot remains bounded.
- Do not claim global linearizability of hosted projections or remote APIs; conformance compares the same authority and pinned snapshot.

## Negative cases and test oracles

- **Model oracle:** two producer paths submit same-key/same-digest command → one operation/receipt; same key with different digest → conflict; stale expected revision cannot overwrite accepted state.
- **Authority oracle:** client `actor_ref` or `authorized:true` without authenticated capability is rejected; read-only projection echo does not become a new command.
- **Eligibility oracle:** requirements=true/filter=false and requirements=false/filter=true are each ineligible; empty predicates are true; no OR interpretation.
- **Failure oracle:** broker unavailable after accepted intent yields unavailable/pending/uncertain receipt; adapter never writes fallback store or loses original dedup identity.
- **Cross-target oracle:** same authority and versioned fixture produce same command interpretation; a target unable to provide atomic claim revalidation cannot advertise claim capability.
- **Digest oracle:** published RFC 8785 vectors plus independent Rust/TypeScript typed-command fixtures match canonical bytes/hash. Changing included intent/preconditions conflicts; changing delivery ID, wait, output or dry-run mode does not alter the intent digest. A matching digest never bypasses authorization.
- Test fixtures are future requirements; they are not evidence of implemented behavior.

## Revisit triggers

Reopen if measured broker capacity/availability fails documented service needs, the authoritative queue owner changes, #9028 resolves to a different maintained contract, a verified QTI design assigns canonical work state elsewhere, or hosted implementations cannot conform without changing this authority boundary. Any alternative must specify one conflict/claim authority rather than reintroducing silent peer writes.

## Implementation, research, and source links

- Existing issue links: [#9028](https://github.com/developing-today/code/issues/9028), [#9198 census/crosswalk](https://github.com/developing-today/code/issues/9198), [#9241 QTI recovery research](https://github.com/developing-today/code/issues/9241), [#9200](https://github.com/developing-today/code/issues/9200), [#9203](https://github.com/developing-today/code/issues/9203).
- Serialization reference: [RFC 8785 evidence note](../research/canonical-json-evidence.md).
- Existing granular component keys: #9200 `DST-01` shared contract and `DST-03` accepted intent; #9203 `DSP-06` canonical dispatch reservation. #9028 is the existing producer-reconciliation gate; #9178 is the existing claim-semantics authority. No third/local backlog is created by this record.
- Pinned commit: `a038f084dbeb525494d7bb928ad91f597db2ac51` (source census only).
