# #9198 — Requirement crosswalk and current-state census

**Status:** design/validation component; pinned source facts only, not implementation/deployment proof.  
**Code pin:** `a038f084dbeb525494d7bb928ad91f597db2ac51`. GitHub issue/PR evidence is dated separately.  
**Scope:** join every curated source-specific semantic requirement to source occurrences, design anchors, bounded V1 evidence, granular implementation/research keys, and explicit residual/missing evidence.  
**Primary artifacts:** [`../inventory/requirements.jsonl`](../inventory/requirements.jsonl) (389 records: 349 non-duplicate outcome nodes plus 40 explicit `duplicate_of` records), [`../inventory/dispositions.jsonl`](../inventory/dispositions.jsonl) (3,571 source occurrence dispositions), [`../census/requirements-crosswalk.jsonl`](../census/requirements-crosswalk.jsonl), and [`../census/requirements-crosswalk.md`](../census/requirements-crosswalk.md). These ledgers are distinct: source occurrence → curated semantic requirement → detailed design/issue slice → evidence/status.

## 1. Mandate and scope boundaries

The charter requires source-preserving, evidence-backed designs and a dependency-aware implementation backlog for two tracks. This component reconciles curated requirement IDs from all five source documents without dropping additions from the handoff, recovered original, proposal, or supplemental note. The crosswalk is per semantic requirement, not per epic and not a claim every occurrence is independently implemented. Duplicate/continuation occurrences retain their IDs in `dispositions.jsonl`; the crosswalk rows preserve requirement IDs and point back to every exact source occurrence.

Allowed status values are exactly: `verified existing`, `partially implemented`, `open implementation`, `design-only`, `research`, `superseded with rationale`, `blocked`, `unverified`. Use these as evidence judgments, not progress labels:

- `verified existing`: exact named behavior is demonstrated by pinned source/test or dated authoritative evidence; does not imply deployment.
- `partially implemented`: exact behavior exists for a bounded subset; name the subset and residual. Adjacent commands/features do not count.
- `open implementation`: a behavior/design exists or is specified but code capability is not evidenced; slice and missing implementation stated.
- `design-only`: architecture/contract/design output exists, but no implementation is claimed. Use for this design pass's artifacts and proposed-only outcomes.
- `research`: evidence acquisition/crosswalk or primary-source fact is still the work item (e.g., QTI provenance #9241); state its bounded acceptance, not an assumed answer.
- `superseded with rationale`: only when an explicit better mechanism replaces a source mechanism; preserve outcome, cite replacement, reason and how behavior survives. Never use as deletion shorthand.
- `blocked`: named hard prerequisite prevents the stated activity; distinguish schema-coupled work from offline contracts that can proceed.
- `unverified`: available evidence cannot determine behavior or authority; do not upgrade from neighboring implementation, issue title, mock, or closed parent.

`requirements-crosswalk.jsonl` uses one row per `requirements.jsonl` ID with original `modality`, `source_refs`/occurrence IDs, exact allowed `status`, reason, actual Markdown `design_refs` (filename plus real heading), `existing_evidence` object (`pin`, `path`, `symbol`, `lines`, `test`, `issue_url`, `timestamp`), `implementation_slice_keys`, `residual`, `missing_evidence`, and duplicate/candidate disposition. Evidence fields are null where no current evidence is asserted. `requirements-crosswalk.md` gives counts, source→requirement→design/slice joins, detailed census and unmapped/gap lists. `census/commands.jsonl` (215 exact command/option/event keys) and [`../census/command-slice-map.jsonl`](../census/command-slice-map.jsonl) (one primary slice per exact key plus supporting slices) are owned by Track B; consume this exact map and do not duplicate it. If a row lacks a valid join, surface it in unmapped report; never report false zero.

## 2. V1 census — actual state and bounded facts

### Repository/source baseline

All code claims are at the pinned commit above. See [`../census/track-a.md`](../census/track-a.md) and [`../census/track-b.md`](../census/track-b.md) for full evidence, limitations, issue snapshots, and QTI search. These artifacts preserve the original detailed code/test/source citations rather than restating every source fact here.

- **Queue and requirements:** `pkgs/fleet/crates/fleet-queue/src/turso_queue.rs:3110–3129` records one-queue matching; task requirements and explicit claim-time filter are conjunctive AND with empty defaults true. Identity is recorded, not an implicit filter. `:4210–4272` selects and conditionally claims the same row atomically. Claim sorting uses priority nonnull, ascending score and created time (`:4267–4272`) without final canonical ID; list adds `id ASC` (`:6454–6457`). Atomic source behavior does not establish claim/list total-order parity or deployed operation.
- **Priority compatibility:** `pm-stack/dispatch/src/validate.ts:11–27,50–60` defines `[10,20,30,40,50,70,90]` and nearest snapping (`15→10`, `25→20`). #7030 ratified exact `expedite=15`, `next-up=25`; its residual staffing/coverage acceptance is not the same as value ratification. Keep values; fix adapter without another ratification. Historic charter 450/1000 remains a profile candidate, not a runtime mapping.
- **PM projection/writer:** `pm-stack/README.md:94–112` documents Project #4, Epic Kind/label, native parent vs blockedBy, and orchestrator single writer. `aggregator/src/queue-view.ts` is a Project/board projection, not canonical claim order. `actions.ts` stores action requests; downstream consumers apply. Live Project mutation/deployment was not verified.
- **Dispatch:** `fleet-dispatch` provides request boundary and PM dispatch has a Supabase/PostgREST path in `pm-stack/dispatch/src/server.ts:50–53,79–83,214–250`; #9028 tracks bypass of packet validator/broker. Canonical broker is the target, not universal V1 compliance. #8187 is launch-on-handoff baseline; #6533 remains open live launch-and-bind proof.
- **Projector ingestion substrate:** `pm-stack/projector/src/webhook.ts:7–57` provides bounded raw-body reading and HMAC-SHA256 validation; `pm-stack/projector/src/server.ts:147–201` serves `POST /github/webhook`; `test/webhook-delivery-id.test.ts:90–154` covers invalid-signature/no-effect, missing/empty delivery IDs and same-GUID duplicate behavior. `README.md:23–34` documents selected event kinds and reconciliation cursors; `README.md:145–179` and `src/store.ts` describe the namespaced disposable hot ledger with delivery/subevent IDs, monotone sequence, server-issued high-water and consumer ACK after a sweep. This is reusable intake/projection substrate—not canonical Fleet command acceptance, field/action authorization, durable operation receipt or generic Fleet activity/outbox. README lines 3–8 state the projector never writes back to GitHub/Kaneo/Pad/Grist; it is not the place for authoritative action policy or remote mutation.
- **CLI/lifecycle:** `client.rs:5301–5345` dispatches a queue-focused command set; `repo-queue` aliases the same binary. Queue `park`, `wake`, `complete`, fallback, stack replay, session and outbox primitives have bounded tests/surfaces. Exact proposed `fleet work`, `fleet github`, metadata/dependency/activity/sync verbs are absent at pin. See [`../census/commands.jsonl`](../census/commands.jsonl), current test matrix in Track B, and its command slice map for exact per-key status; nearby verbs are not aliases.
- **Completion/outbox:** queue `complete`, merge-intent stack replay and scoped creation outbox are substrate only. `doc/fleet/creation-outbox.md:54–60` documents one producer `new-issue-creator-code-a`; no general lifecycle mutation outbox or direct verified completion is established.
- **QTI/schema:** exact-name search finds no QTI identity within enumerated accessible repository/history/index surfaces. #3424, #8575, #8579/#8602 and #8580–#8585 are related compatibility work, not presumed equivalence. #9241 under #9198/task 15480 is the bounded research item. No shared schema changes before source/owner compatibility disposition; offline contract fixtures proceed.
- **Prior design work:** #9060 and #9078 parent epics are closed while component children remain open. Their open components and #9016/#8887 are consumed as seams; parent closure is not feature verification. #9178/#9179/#9180 remain live behavior/process anchors. #7030's ratification is not reopened.

### Evidence classification rules

Every crosswalk `existing_evidence` entry distinguishes pinned code path/symbol/line from tests and dated issue/readback facts. A test is named only if census/source cites it. Timestamp must be the actual evidence snapshot from `track-a.md`, `track-b.md`, or issue comment; do not assign one timestamp to a historical code fact unless the source snapshot provides it. “No matching implementation at pin” is a bounded code-census claim, not universal absence. Existing command parser evidence never upgrades an adjacent proposed command. `verified existing` and `partially implemented` require a concrete evidence object; all design-only/unverified/research/blocked/open rows name missing evidence explicitly.

## 3. V2 — contract and compatibility crosswalk

The common architecture is [`../architecture.md`](../architecture.md); it governs authority, identity, JSON precision, event delivery/revision, both claim predicates, snapshot atomicity, graph races, outbox, late activity, managed body capability and module locality. Track designs: [Track A](../fleet-priority-urgency-pm-stack.md) and [Track B](../fleet-work-item-orchestration.md). Components and actual section anchors are linked by each machine row; there is no “H1” placeholder mapping.

Crosswalk domain ownership:

| Requirement family                              | Design/contract owner | V1 baseline/evidence                                                                                                                   | Primary residual gates                                                                   |
| ----------------------------------------------- | --------------------- | -------------------------------------------------------------------------------------------------------------------------------------- | ---------------------------------------------------------------------------------------- |
| Census, source preservation, authority map, QTI | #9198                 | Track A/B census, source packet, inventory                                                                                             | #9241 QTI result; #3424/#8575/#8579 compatibility; batch issue index pending             |
| Priority dimensions, mapping, rank and triage   | #9199                 | priority scalar and dispatch adapter mismatch; list/claim tie defect                                                                   | exact profile, rank field, provenance, aging/triage tests                                |
| Distributed command/projection provenance       | #9200                 | Project writer docs, action/projection and dispatch seams                                                                              | #9028 producer reconciliation; accepted revision/operation schema; Project readback      |
| Mobile board/actions                            | #9201                 | aggregator board/read model/action store                                                                                               | all action controls, accessibility, service operation results and hosted parity          |
| Authenticated comment/command/direct dispatch   | #9203                 | Projector has bounded HMAC delivery intake/dedup and disposable projection hot ledger; Fleet dispatch/PM route partial; #8187 baseline | canonical Fleet operation acceptance/authorization/receipt, #9028, #6533 live acceptance |
| Normal selection/exploration/fairness           | #9204                 | atomic queue claim/order, no final tie, no sampler                                                                                     | exact claim parity, filtered lanes #8097, calibration, fairness objectives               |
| Lifecycle and command surface                   | #9206                 | queue-oriented exact commands only                                                                                                     | every command/option from JSONL; #9178/#9179; one CLI glue owner                         |
| Hydration and activity                          | #9207                 | no registered work hydrate/activity surface demonstrated                                                                               | immutable bundle, cursor/page atomicity, endpoint limits, attribution                    |
| Typed relations, journal, authority             | #9208                 | legacy blocker/event/status fields; no typed graph/journal proof                                                                       | #3424 and schema home; privacy, cycle, field authority, managed-content CAS capability   |
| Lease/outbox/completion                         | #9209                 | worker lease, creation pilot, stack merge intent, queue close substrate                                                                | fence, general mutation outbox, GitHub effects/readback, crash recovery                  |

### Data, ownership and compatibility boundaries

The UI and command/candidate projections may expose separate facts; only each source owner mutates its owned field. PM action store is a request ledger, not canonical applied state. GitHub labels are normalized input or projection by explicit mode. GitHub bodies/comments remain GitHub-owned; Fleet title/summary are independent after initialization. Session shell/username is not a task-bound acknowledged session. Queue completion or GitHub closure alone is not work completion. Parent hierarchy does not imply hard blocker or child completion. Hard edge and permission changes need expected versions, provenance, and operation receipts. External effect intent is transactional only within existing Fleet store after semantics verified; remote effects require readback. GitHub lacks universal Fleet fence/CAS/Idempotency; claim only operation-scoped effectively-once where recognizable, never universal exactly-once.

No migration or new storage schema is chosen in V2. QTI/#9241 and #3424/#8575/#8579 are hard for overlapping schema/migration, not for pure contracts, reference models, offline fixtures, read-only adapters, or research. Current `fleet-queue` module paths are a concrete design default, not table/schema owner allocation. Creation outbox pilot is not generalized by implication.

## 4. V3 — staged issue-ready work, without filing claims

Granular slice keys are defined in the component appendices. Preserve existing keys where already specified and add no broad parent blocker that creates cycles. The prioritized graph is [`../delivery-plan.md`](../delivery-plan.md); actual issue URLs and resolved keys populate `issues/index.json` only after batch filing. A future issue must name exact owned files, failing oracle/test, acceptance command, negative case, compatibility/migration/rollback, hard dependencies and tier. Pure model/reference tests run before remote adapters. Full multi-domain suite is not a prerequisite for a narrow offline fixture.

Program-level bounded follow-up families (local keys until filed):

1. `CEN-01` — requirement/source-occurrence/design/issue trace validator and unmapped report (this artifact plus existing 389/3,571 ledgers; no new semantic IDs).
2. `CEN-02` — pinned current-code/source test crosswalk and independent census review; verify V1 negative/positive claims at exact pin; no deployment inference.
3. `QTI-01` — consume #9241/task15480 to reconcile QTI source identity and #3424/#8575/#8579/#8580–#8585 compatibility; research/review only, no schema write and no duplicate research issue.
4. `COMPAT-01` — cross-model/source-owner compatibility matrix and additive schema/read/write decision gate; only after QTI and actual migration owner provide evidence.
5. `MODEL-01` — independent reference model/fixtures for shared identity, priority ordering, claim AND predicates, graph, cursor, journal and effect receipts; use explicit test oracles; no production writes.
6. `ADAPTER-01` — real-adapter conformance and CI adoption after offline fixtures; named package scripts, secrets/test endpoints and failure-injection boundaries must come from existing owners.
7. `MIG-01` — migration/rollback plan and owner integration tests, only for approved persisted contract; no new DB; preserve old reads/receipts/remote effects.
8. `ROADMAP-01` — dependency-graph/coverage audit and issue-index completion after batch filing; hard-vs-soft edges; actual issue IDs only.
9. `DOC-01` — command help/source/track guide consistency, implementation handoff and coverage audit, integrated with #9206’s one CLI registration owner and #9208 docs slice (avoid overlapping ownership).

Two small future option slices, not new infrastructure: `OPT-IMPORTANCE-01` researches whether/where a distinct importance signal exists and whether axis-mixture exploration provides value; until then importance stays null/unfabricated. `OPT-REPLAY-01` measures whether full historical state reconstruction is needed beyond append-only decisions/checkpoints; no blanket event-sourcing conversion. Assign under #9199/#9204 and #9208 only after existing slices/index reconcile; do not create duplicate starters.

## 5. V4 and non-goals

Hosted #9079/#9081 consume same schemas, state-machine/ordering fixtures and degraded/unsupported states; they may use different storage internally. Prove hosted deployment/readback separately. No second scheduler, queue, broker, priority engine, database, Project writer, GitHub client or event-sourcing program. Do not state rollout/production architecture has been adopted; owner review of design does not authorize production changes.

## 6. Change/status separation

The crosswalk distinguishes proposed design, V1 source fact, existing test, live issue evidence, and deployment/live acceptance. The current product issue index is pending. Parent issues/epic closure do not imply child completion. Research #9241 is actual; QTI remains unlocated until its acceptance is completed. PR #9225 is design-buildout context only, not implementation of orchestration features.

## 7. CEN granular validation/research issue specs (proposed; local keys only)

These eight slices make the census/crosswalk and compatibility handoff executable without pretending they are filed issue IDs. Add each real URL to `issues/index.json` only after the parent batch creates/reads the issue. File ownership is one slice per artifact; the research package remains public-safe. Requirements IDs and source occurrence IDs are immutable inputs, not regenerated/renumbered by these checks.

### CEN-01 — Requirement/source-occurrence crosswalk integrity

- **Tier:** crucial. **Goal:** prove each of the 389 semantic record IDs occurs exactly once in the crosswalk, every occurrence citation resolves to the 3,571-row disposition ledger, duplicates preserve `duplicate_of`, groups preserve children, and no source/modality is silently lost.
- **Owns:** `census/requirements-crosswalk.jsonl`, `census/requirements-crosswalk.md`, and new strict validator/test files `census/check_requirements_crosswalk.py` and `census/test_requirements_crosswalk.py`; it may call but not change `inventory/check_inventory.py` or `inventory/check_requirements.py`. No edits to semantic inventory/dispositions. Exact design refs and local keys must parse against actual files/headings and `census/command-slice-map.jsonl`.
- **Hard dependencies:** frozen `inventory/requirements.jsonl` and `inventory/dispositions.jsonl`; crosswalk document in this package.
- **First failing oracle:** `test_requirements_crosswalk_rejects_missing_duplicate_occurrence_and_unresolved_design_anchor` in `census/test_requirements_crosswalk.py` mutates each relation in a temporary fixture and expects failure; oracle is exact ID/span set equality plus heading existence, not production implementation.
- **Done check:** `python3 -I inventory/check_inventory.py --check` and `python3 -I inventory/check_requirements.py --check` remain passing; `python3 -I census/check_requirements_crosswalk.py --check` validates 389 IDs, group/alias edges, source occurrences, exact status vocabulary, evidence object on verified/partial, real anchors, 215 command-map keys and no wildcard. Check emits explicit per-class unmapped rows; it never treats absence of a join as zero gaps.
- **Negative / rollback:** duplicate ID, missing occurrence, wrong quote/span, dead file/heading, status typo, partial row with no source evidence, group child not found and wildcard command mapping each fail. Research artifacts only; rollback is restore the prior generated crosswalk, not to edit source ledgers.
- **Operator-script contract:** the future validator accepts only `--check`; it parses the complete argv before opening any source/output, writes nothing in check mode, rejects unknown/extra args, and follows the strict sibling argument pattern.

### CEN-02 — Pinned V1 source/test and dated-issue evidence audit

- **Tier:** crucial. **Goal:** independently re-check positive and negative evidence claims in census A/B against the pinned source tree and timestamped issue evidence; separate source code, existing tests, live issue observations, and deployment.
- **Owns:** additions only to `census/requirements-crosswalk.md`, JSONL evidence fields and new `census/test_census_evidence.py`; consume `census/track-a.md`, `track-b.md`, `commands.jsonl`, `anchors.json`, and the parent-owned projector audit. Do not edit the source census files under this pass.
- **Hard dependencies:** CEN-01 ID/occurrence join; pinned commit `a038f084dbeb525494d7bb928ad91f597db2ac51` and Track A/B anchors.
- **First failing oracle:** `test_census_evidence_requires_exact_symbol_and_bounded_claim` in `census/test_census_evidence.py` provides counterexamples (adjacent verb, closed parent issue, mock, projector delivery ACK) and asserts each cannot upgrade a separate target capability.
- **Done check:** table-driven evidence audit validates required source path/symbol/lines/test/timestamp for every `verified existing`/`partially implemented` row and explicit missing evidence for others. Includes projector `webhook.ts:7–57`, `server.ts:147–201`, `webhook-delivery-id.test.ts:90–154`, README hot-ledger semantics, `turso_queue.rs` claim/filter, `validate.ts` 15/25 snapping, PM PostgREST path/#9028, #8187/#6533 split.
- **Negative / rollback:** no inferred deployment, no claim that projector ACK is Fleet command acceptance, no claim of #6533 proof from #8187, no issue-title-as-test evidence. Dated evidence corrections are additive citations; never rewrite pinned code claims as live current behavior.

### CEN-03 — QTI provenance recovery and reuse of existing #9241

- **Tier:** crucial research gate for overlapping schema only. **Goal:** consume existing [#9241](https://github.com/developing-today/code/issues/9241) / task 15480 and record a reproducible exact-name QTI search disposition and per-candidate relation matrix; do not create duplicate research issue.
- **Owns:** this component's QTI compatibility table/linked receipt and `census/requirements-crosswalk.md`; #9241 owns its research deliverable and issue state.
- **Hard dependencies:** #9241 current result/receipt; enumerated accessible source and search boundary.
- **First failing oracle:** `test_qti_candidates_have_bounded_provenance_verdict` in `census/test_qti_crosswalk.py` asserts each candidate includes source URL/key, date/status, owner/scope, data model/API/authority, code/PR, compatibility mapping and `consume/extend/supersede/no-overlap` verdict; a missing candidate fails.
- **Done check:** exact QTI identity found with primary evidence, or `not located within enumerated accessible surfaces` with searches, refs, timestamps and inaccessible surfaces. Explicitly compare #3424/#8180, #3418, #8575, #8579/#8602, #8580–#8585 and current one-queue broker. No schema or production write.
- **Negative / rollback:** synonyms do not establish identity; no candidate is silently declared QTI. Research disposition can be corrected additively with new evidence; no existing record is rewritten. Soft for V2 contracts/read-only adapters; hard only before overlapping persisted schema/migration.

### CEN-04 — Cross-model schema owner and migration compatibility matrix

- **Tier:** crucial before schema-coupled design or implementation. **Goal:** crosswalk each selected Fleet fact/field against #3424, #8575, #8579 and related #8580–#8585 plus QTI result; identify one owner, source-of-truth, API, migration and rollback.
- **Owns:** `components/9198-requirement-crosswalk-current-state-census.md` compatibility matrix, `delivery-plan.md` gates, and new test `census/test_schema_compatibility_matrix.py`; actual migration/schema remains owned by the existing storage owner.
- **Hard dependencies:** CEN-03 bounded QTI disposition; dated #3424/#8575/#8579 evidence; current source/migration-owner confirmation.
- **First failing oracle:** `test_schema_fact_has_one_owner_and_legacy_mapping` checks queue work ID/state, urgency/profile, lease, blocked deps, journal/events, notes/checkpoints, cursors, operation intent and Project fields; every fact is either reuse, extend, absent, or conflict with evidence and rollback path.
- **Done check:** all overlapping schema facts map to one authority; unresolved fields are listed as gated; no design invents tables or treats `events`, `blocked_deps`, `merge_intent`, or creator outbox as equivalent by name.
- **Negative / rollback:** no second store/two writers; uncertain legacy data stays unknown/quarantined; migration plan preserves old reader/data/pending remote receipts and has a disable/rollback boundary. This slice is a design gate, not migration authorization.

### CEN-05 — Offline reference model and experiment runner

- **Tier:** crucial, implementation-independent. **Goal:** run deterministic test-only/reference conformance and priority/exploration/lifecycle models using immutable synthetic fixtures; no production reads/writes.
- **Owns:** new proposed `validation/experiment.py`, `validation/fixtures/` and `validation/test_experiment.py`; this runner emits machine-readable result artifacts only. Exact run arguments and fixture schema become part of its strict CLI contract.
- **Hard dependencies:** CEN-01 semantic IDs; shared architecture and component pure-model inputs. No QTI/schema, deployed adapter or owner threshold gate for synthetic fixtures.
- **First failing oracle:** `test_experiment_replays_same_fixture_and_rejects_bad_contract` in `validation/test_experiment.py` compares against an independent reference evaluator/golden vectors; includes requirement AND filter, numeric work-ID ordering (9 before 10), RFC 8785 DTO bytes, cycle race model, cursor crash, operation apply-then-timeout, and seeded sampler probabilities.
- **Done check:** seeded inputs, generator versions, oracle and output schema are pinned; repeated run produces byte-stable machine result. Property/model tests cover negative cases; no production code is used as its own oracle. Run only named offline test command in the filed issue, with exact test listing check.
- **Negative / rollback:** invalid Unicode/duplicate JSON key/lossy ID/unknown major schema, incomplete graph/page and unverified remote effect fail closed. Disable runner by removing CI invocation; no DB/production side effect exists.
- **Operator-script contract:** parser declares the full invocation and validates all flags/values before reading fixtures or writing result artifacts; unknown arguments are usage errors; no subprocess/network/production target is part of this experiment runner.

### CEN-06 — Experiment result interpretation/report writer (separate child)

- **Tier:** recommended. **Goal:** translate immutable CEN-05 output into a reviewable evidence report, separating measured observations from assumptions, thresholds and recommendation; do not let experiment code author its own acceptance narrative.
- **Owns:** separate proposed `validation/report.py`, `validation/test_report.py`, and versioned report output `research/priority-exploration-experiment.md`. It reads an immutable result artifact; it does not edit `validation/experiment.py` or write production settings.
- **Hard dependency:** CEN-05 output schema and reproducible result artifact.
- **First failing oracle:** `test_report_marks_missing_measurement_and_does_not_claim_production_benefit` in `validation/test_report.py` rejects missing run ID, seed, sample count, oracle, uncertainty, fixture provenance or unmeasured threshold; verifies synthetic simulation is not called production result.
- **Done check:** report reproduces input/result digest, separates external source facts, measured model outputs, inference and recommendation; a reviewer can regenerate it from the immutable artifact.
- **Negative / rollback:** do not choose thresholds post hoc or mark exploration enabled from simulated benefit alone. Remove/replace report only with a new versioned artifact and preserve prior run provenance.

### CEN-07 — Real-adapter conformance and CI adoption

- **Tier:** recommended after offline model, before any mutation capability is advertised. **Goal:** run shared golden command/receipt/selector/cursor/authority fixtures through existing adapters and add their owner-approved command to CI; no new service/backend.
- **Owns:** adapter-owned tests in existing `pkgs/fleet/crates/fleet-queue` and `pm-stack/projector/test/webhook-delivery-id.test.ts`, plus proposed `.github/workflows/pmo-contract-conformance.yml`; workflow calls package-owner scripts already declared in Cargo/Bun manifests, not a new service.
- **Hard dependencies:** CEN-05 fixture/output contract, #9200/#9203/#9207/#9208/#9209 adapter seams, current test scripts; #9028 before canonical producer reservation tests.
- **First failing oracle:** `test_adapter_conformance_distinguishes_projection_ack_from_fleet_acceptance` in the shared conformance suite proves same request/event delivered twice produces one projection event, while canonical operation owner separately emits one accepted operation only after authorization; comparator/manifest/digest result matches independent golden oracle. These are two authorities and two distinct assertions.
- **Done check:** CI runs existing package suite(s) and exact named filters; unexpected upstream/provider behavior is typed/degraded. Mocks prove contract only; if a live read/write/dispatch acceptance is required, it remains a separately authorized issue.
- **Negative / rollback:** CI failure prevents claiming conformance; reverting CI hook leaves test scripts but does not roll back any live state. Never silently point projector writes at Fleet or treat projection ACK as command acceptance.

### CEN-08 — Compatibility gate, dependency graph, and documentation handoff

- **Tier:** crucial for implementation issue batch and recommended before final design handoff. **Goal:** publish prioritized granular dependency graph and issue-index/crosswalk coverage, including hard-vs-soft edges, source/census validation, migration/rollback gates and docs ownership.
- **Owns:** `delivery-plan.md`, `census/requirements-crosswalk.md`, eventual `issues/index.json`, and proposed `census/test_issue_dependency_coverage.py` through the filing owner; does not edit other component/census documents or create duplicate issue URLs.
- **Hard dependencies:** CEN-01/02; CEN-03/04 for schema-coupled edges; command-slice map (215 exact rows); issue filer must provide actual issue URLs/state/parent links.
- **First failing oracle:** `test_issue_index_covers_every_granular_slice_without_dependency_cycle` in `census/test_issue_dependency_coverage.py` asserts every local implementation key is either mapped to an actual filed issue or explicitly pending, every absent exact command has a slice, shared CLI glue has one writer, and hard-edge graph is acyclic. Missing issue URLs are listed as gaps, not synthesized.
- **Done check:** a fresh reader can follow source ID → semantic ID → real design heading → local slice → actual issue URL or explicit pending. QTI/compat gate blocks only overlapping migration; offline contracts have no artificial dependency on domain-parent completion. Documentation/handoff matches exact CLI map, FDR status and V1 limits.
- **Negative / rollback:** no cycle introduced by whole-domain parent blockers; no `done` parent/PR substitutes for component acceptance; rollback updates index/link state without erasing source/crosswalk history. This planning artifact authorizes no migration or rollout.
