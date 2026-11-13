# #9200 — Priority distribution, provenance, and reconciliation

**Status:** Proposed component contract for review; no implementation or migration authorized.
**Inventory target:** `components/9200-priority-distribution.md` (provisional target in `inventory/statements.md`).
**Evidence baseline:** pinned tree `a038f084dbeb525494d7bb928ad91f597db2ac51`; census `census/track-a.md`, 2026-11-13T03:30Z.
**Related:** #9199 (priority model), #9201 (PM/mobile), #9203 (commands/events), #9204 (selection), #9208 (dependency types/field authority), #9209 (outbox/leases), #9073/#9074 (metadata), #9075/#9076 (session state/dual ingestion), #9079/#9081 (hosted), #9178/#9179/#9180/#7030.
**Source trace:** H-DISTRIBUTED, H-SURVIVE-A, C-3–C-8/C-13, O-PART1 §§10–14/18–20, P-10–P-12, S-LABELS/S-COMMANDS; generated ledger targets are provisional, not atomic coverage.

## Decision frame

Recommendation for parent review: **reuse the existing Fleet queue broker as canonical priority/claim authority; use authenticated intent ingress for multiple producers; append accepted decisions and produce versioned snapshots/projections; compute effective scheduling pressure from a pinned input snapshot.** No Kafka, new database, scheduler, new PM authority, or event-sourcing rewrite. Keep producer authority field-specific, PM Project single-writer, claim eligibility separate, and hosted targets as contract consumers. Parent retains final architecture decision.

## V1 — observed producer/consumer and state baseline

The following is evidence at the pinned commit, not an assertion about live deployment:

| Surface | V1 observed role | Known boundary / missing piece |
|---|---|---|
| Human ratification/manual queue values | #7030 comment [6715673367](https://github.com/developing-today/code/issues/7030#issuecomment-6715673367) (created 2026-11-08T01:33:40Z) ratifies `expedite=15`, `next-up=25` as stored priority-band convention; task ID 14516 [comment 6786217364 on #7030](https://github.com/developing-today/code/issues/7030#issuecomment-6786217364) (2026-11-11T03:33:51Z) records a live broker readback. | `resolvePriority` accepts integers 10–90 but snaps off-scale values to nearest discrete `[10,20,30,40,50,70,90]`, ties down: `15→10`, `25→20`. This is not rejection; it is lossy normalization at the producer/consumer boundary. `expedite=15` is not a Do Now/direct-session directive. #7030 remains open for staffing/coverage acceptance. Do not ask for re-ratification or renumber. |
| Existing Projector GitHub intake | Partial transport/projection capability exists: `pm-stack/projector/src/server.ts:147-201` serves `POST /github/webhook`, requires delivery GUID, bounded raw-body read, HMAC-SHA256 verification via existing `src/webhook.ts`, delivery-GUID dedup, local `applyEvent`, and freshness fanout. README documents disposable/rebuildable projector rows, hot-ledger sequence/high-water, and monotonic consumer checkpoints. | This is **not** the canonical priority command authority or a GH/Project mutator; README says Projector never writes back to GitHub/Kaneo/Pad/Grist. Current code does not establish priority label→Fleet command routing or actor/action authorization. Approved intake deployment liveness is unverified. Reuse the bounded HMAC/helper/delivery/cursor protocol as transport; fan out a typed operation to existing Fleet authority. |
| Queue broker and claimant | `turso_queue.rs:4210-4272` has atomic conditional selection/claim; order priority ascending then created time; one queue at `:3110-3129`. Task requirements and explicit claimer filter are two separate predicates and both must pass. | Priority provenance/policy revision and final stable claim tie key are not established. V1 list adds `id ASC`, claim does not. |
| PM Project #4/orchestrator | `pm-stack/README.md:94-112` documents Epic kind canonical, label as maintained mirror, native parent hierarchy distinct from `blockedBy`; orchestrator sole Project writer. | Deployment/live field writes not freshly verified; don't assume Project position is canonical broker rank. |
| PM aggregator/dashboard | `aggregator/queue-view.ts` projects board/Project inputs and owns no queue persistence; `actions.ts` writes action store and downstream fleet jobs apply actions; Project writes by orchestrator. | Useful view/control boundary, not live eligible claim order. Full mobile mutations/control and unified ranking are not established. |
| Hosted PM / Fleet | #9079 and #9081 are existing open component issues under the #9078 hosted design. | Design targets only; consume them, do not treat closed parent epic as hosted parity or build a parallel interface. |
| Identity/metadata | #9073 create-path metadata, #9074 metadata DB, #9075 session state, #9076 dual ingestion remain open. | Reuse/extend canonical identity and metadata interfaces; do not create duplicate records/DB. |

Queue Table Improvements was not located in repo text/history, GitHub issue search, or PR search. It remains unlocated/unverified. Current queue and #7030 are overlapping evidence, not equivalence. Any schema migration remains gated on a definitive pointer or owner disposition.

## V2 — distribution contract

### Authority and precedence table (proposal)

Precedence below is subordinate to the existing **protected owner priority policy**:
[`architecture.md`](../architecture.md#protected-owner-priority-policy). A routine manual
or automatic producer cannot change Drew's protected expedite value, peer placement,
membership or priority-label intent. Check current membership/policy plus `owner_policy_ref`
atomically; only the trusted configured Drew owner action can amend it. A mutable mirror
label is not membership authority. Deny with `OWNER_POLICY_PROTECTED`, or conflict on a stale
policy reference; preserve before/after and source proof for allowed owner amendments.
DST-01/DST-04 share these fixtures with PRI-01/PRI-05/AUTH-01; no re-ratification is requested.

This table defines *candidate interpretation* and must be checked against source authority/access controls before acceptance. Source-of-doc precedence (handoff > charter > proposal > recovered; supplement non-overriding) is not the same as field-write precedence.

| Input / producer | Accepted as | Field authority and effect | Conflict / precedence behavior |
|---|---|---|---|
| Authorized human/manual CLI or shared PM command | Typed command with actor, source and expected revision | May explicitly set intrinsic urgency, nullable importance if enabled, peer-order command if approved, state/directive only where authorized. | Highest field authority among routine value edits; manual command is never rejected merely by automatic hysteresis. Stale expected revision returns conflict; do not blindly overwrite. |
| Ratified `expedite=15`, `next-up=25` or supported priority label input | Normalized priority-band command accepted by existing Fleet authority | Preserve exact ratified stored band convention through a lossless, versioned adapter/profile; do not alter canonical value simply because unchanged derived label was echoed. | Current `resolvePriority` snaps `15→10` and `25→20`; DST-02 compatibility work must remove silent divergence, not re-ratify values. `expedite=15` remains a band/value. A future operation named `expedite` is a distinct typed directive only if explicitly added with compatibility mapping; it cannot redefine this label/value. Projector may verify webhook HMAC and deduplicate transport delivery, but actor authorization and canonical priority acceptance belong to Fleet authority. |
| Final priority-label removal | `priority-label.removed` intent | Preserve canonical intrinsic value; mark `needs-triage`; prioritize triage selection, not execution selection. | Does not create emergency/default score. Other valid remaining label is normalized by configured mapping. |
| Authenticated slash comment / direct-session or `do-now` | Durable one-shot directive request, routed by #9203 | `do_now` or `direct_session` request and operation identity only; not a numeric score. Routed to existing active session or supported direct dispatch; hard gates remain. | Idempotent by source delivery + request identity. Reject/hold unless authenticated principal has current capability; a client actor reference is not authorization. Report blocked/lease/concurrency reason. Never silently demote to normal queue. |
| Dependency metadata / edge event | Typed graph command | Hard-blocker edges affect eligibility and derived pressure; hierarchy/advisory edges do not. | Graph revision is explicit; cycle/invalid edge rejects or quarantines. Does not write intrinsic urgency. See #9208/#9199. |
| Policy triage/aging/boost process | Versioned policy evaluation / recommendation | May emit evidence-backed bounded intrinsic change only if explicitly authorized by policy; otherwise effective factor or triage hint. | Lower precedence than manual command; one ordinary move per sweep; cooldown/hysteresis and provenance required. |
| Fleet claimant | Selection/claim request with lane/capabilities/filter/revisions | Reads canonical profile and effective-pressure contract; task requirements **AND** the explicit caller filter must both pass; atomic claim revalidates against broker state. | Claim never trusts stale projection/sampled candidate. Hard gates before order; directive cannot bypass lease/auth/dependency. Persistent `manual_only` is an eligibility gate under #9178, not a one-shot directive. |
| PM Project, dashboard, GitHub output, hosted consumer | Projection, or authenticated command ingress when routed through shared Fleet interface | No duplicate owner. Project fields are written only by orchestrator; outbound GitHub labels use the authorized Fleet `gh` operation adapter; Projector remains a disposable read-only projection and transport intake. | Projection events do not feed back as new human intent; source operation/correlation IDs distinguish echo. Consumers show stale/conflict explicitly. Never make Projector's local store or helper the canonical command authority or an outbound GH/Project mutator. |

### Reason-code registry and effect classes (proposed V2 baseline)

Reason codes explain an operation; none grants authority by itself. Authorization is evaluated from authenticated principal/capability before applying the command. Results separate canonical base mutation from derived effective pressure, advisory hint, and rejection/conflict:

| Reason code | Producer/evidence | Permitted effect class | Must not do |
|---|---|---|---|
| `manual_exact_urgency` | Authorized human/CLI exact-value request and expected work version | Accept explicit base urgency set, subject to profile/range | Derive importance or bypass authorization/version |
| `brief_ruling` | Authorized owner ruling with evidence ref | Accept only named base field(s), or return triage hint if underspecified | Turn free text into Do Now |
| `label_band_change` | Authenticated band label change and source event | Accept base band/value via versioned ratified mapping | Snap 15/25 to 10/20 or mutate from projection echo |
| `label_removed_needs_triage` | Authenticated removal of final priority label | Preserve base; request/raise triage priority | Set Emergency/default score or start execution |
| `label_conflict` | Conflicting labels, ambiguous event ordering or untrusted input | Conflict/reject; triage hint only, no base mutation | Choose by arrival time or silently resolve |
| `dependency_pressure` | Hard-edge path, graph revision, downstream work evidence | Derived effective-pressure component only | Rewrite intrinsic urgency or use advisory edge |
| `deadline_boost` | Verified deadline evidence and expiry | Derived temporary effective pressure with pinned start/expiry | Make permanent base change or bypass eligibility |
| `opportunity_expired` | Expiry event and opportunity reference | Remove temporary boost; create retriage hint | Delete or silently bury work |
| `strategic_pivot` | Authorized pivot evidence and affected project/work refs | Triage hint; base change only after authorized versioned ruling | Demote unrelated descendants automatically |
| `aging_reconsideration` | Review interval + last meaningful triage + evidence | Triage hint or explicitly authorized bounded policy adjustment | Age-only Critical escalation or unbounded drift |
| `clump_refinement` | Versioned sweep, cohort/rationale, before/after | Bounded base adjustment under policy; at most one ordinary move/item/sweep | Force unique scores, mass renumber, repeat same item in sweep |
| `hysteresis_suppressed` | Boundary, confidence, cooldown and candidate transition | Rejected/no mutation; optional review hint | Report transition accepted or erase evidence |
| `projection_echo` | Target operation ref proves value is prior Fleet projection | Ignore/idempotent observation; advance readback only | Treat mirror as new human intent |
| `stale_snapshot` | Expected work/snapshot version mismatch | Conflict/retry; no mutation pending reread | Last-write-wins or claim stale order |
| `unauthorized_input` | Auth layer lacks principal/capability | Reject; retain only permitted audit ref | Accept client actor ref or `authorized` boolean |

This is the proposed shared registry for #9199/#9200/#9204. Reconcile exact code ownership/version in the shared index before filing. Unknown code under a supported schema returns typed `unsupported_reason_code`/`rejected` with no canonical mutation; unknown major schema/policy returns `unsupported_version` and read-only/quarantine. Never alias unknown codes. `Result.outcome` remains the shared architecture vocabulary; `effects` names requested/observed/verified base, effective, hint or rejected field effects.

### Minimal event identity, revision, snapshot and receipt

Do not conflate four different sequences:

1. **External delivery ID**: e.g. GitHub `X-GitHub-Delivery`; same on redelivery. Deduplicates transport attempts.
2. **Operation/idempotency identity**: use the shared architecture `operation_id`, `idempotency_key`, and canonical `request_digest`. A source delivery/event reference (`source_event_ref`) is separate; it may follow CloudEvents identity rules but does not replace the command's idempotency contract.
3. **Per-work version**: broker-assigned monotonic integer/string for accepted state transitions; `expected_work_version` prevents stale writers. Idempotency is separate: same idempotency key + same canonical request digest returns the original operation; same key + different digest is conflict. Competing distinct commands against the same expected version serialize; only one may advance the work version, and the other returns conflict. Never last-write-wins.
4. **Read snapshot/watermark**: stable queue/project/graph/policy high-water marks identifying the exact cross-item view used to calculate ordering. Not an item revision or global event sequence inferred from timestamps.

Use the shared `Command`/`Result` interface in `architecture.md`, not an independent priority envelope. Work identity is the string pair `(authority_id, work_id)`; `fleet://work/842` is only local shorthand when authority context is unambiguous. `actor_ref` is an audit/reference field, **not** authorization; authenticated principal and capabilities are supplied by the auth layer and rechecked before delayed consequential delivery. Wire keys are snake_case. Numeric IDs, revisions, rank keys and versions that may exceed JavaScript's safe integer range are canonical decimal strings; never coerce through floating point. Pin UTC instants and snapshot/policy inputs for replay.

Every accepted command/decision records `schema_version`, policy profile/version, taxonomy mapping version, canonical pair identity and stable external refs, authenticated principal/capability decision, `actor_ref`/`source_ref`, `operation_id`, `idempotency_key`, `request_digest`, `source_event_ref` and delivery reference if any, occurred/received UTC, expected/observed/accepted work version, reason code/evidence refs/confidence, previous/new values, and outcome. Keep raw comment body as untrusted data; retain only as allowed by existing source/privacy policy and prefer references/hash for evidence. Never accept a client `authorized: true` field as proof.

Every projection snapshot identifies desired work version, effective-policy version, as-of time, graph/eligibility snapshot revision, comparator version, canonical order hash (when it represents an ordered slice), and its projection cursor/watermark. Snapshot is **not** proof the external write was applied. Use the shared `Result.outcome` vocabulary (`rejected`, `accepted`, `running`, `pending_sync`, `partial`, `conflict`, `uncertain`, `verified_complete`) plus typed `effects` with requested/observed/verified state; don't invent a second receipt state machine.

### JSON contract example (illustrative shape, not an adopted wire schema)

```json
{
  "schema_version": "fleet.command/v1",
  "operation_id": "op_...",
  "idempotency_key": "github-delivery-derived-key",
  "request_digest": "sha256:...",
  "work": {"authority_id": "fleet-local", "work_id": "842"},
  "kind": "priority.set_band",
  "arguments": {
    "profile_id": "live-ratified",
    "profile_version": "3",
    "taxonomy_version": "2",
    "band": "next-up",
    "canonical_priority": "25"
  },
  "actor_ref": "github:user:...",
  "source_ref": "github:developing-today/code",
  "source_event_ref": "github:delivery:...",
  "expected_work_version": "41",
  "observed_snapshot_ref": "queue:rev:...;graph:rev:...",
  "owner_policy_ref": "owner-priority-policy:accepted-version-ref",
  "reason": {"code": "priority_label_changed", "evidence_refs": ["github:event:..."], "confidence": null},
  "dry_run": false
}
```

This composes with the common `Command` interface; authentication/capabilities are checked out of band, and this payload deliberately has no `authorized` boolean. The shared `Result` fields are `schema_version`, `operation_id`, `work`, `work_version`, `outcome`, `effects`, `warnings`, `error`, `next_action`, and `observed_at`. Integer-like values crossing JSON use the common canonical decimal-string rule when necessary. `expected_work_version` and `observed_snapshot_ref` are distinct: producer snapshot may be stale. Timestamps provide audit chronology only, not conflict ordering. Optional fields are explicitly nullable and unknown fields/version handling follow the shared architecture contract.

### Cross-target order and reconciliation

The byte framing and exact full-population `fleet.selection-order/v1` DTO in
[`architecture.md`](../architecture.md#digest-and-serialization-contract) are normative for
`order_hash`: fixed ASCII purpose + one NUL byte + RFC 8785 canonical UTF-8, SHA-256, then
`sha256:` plus 64 lowercase hex characters. Include snapshot/policy/graph/protection/lane
inputs and every ordered item's comparator keys; do not hash only a top-page display or
silently substitute target-local references. Numeric work-ID tie order preserves `9 < 10`
despite string wire encoding. DST-06 tests framing vectors, included-input changes and
cross-target hashes; a partial candidate set cannot claim full-order conformance.

Same total order can be promised only if targets share the same `(authority_id, work_id)` string identities, taxonomy mapping, normalized fields, policy/comparator versions, both matching predicates, eligibility/capability set, as-of time and queue/graph snapshot. Apply both matching predicates, then hard eligibility, then effective urgency ascending, exact peer rank within equal effective urgency, creation time, and normalized canonical identity, per `architecture.md`. Compute a deterministic `order_hash` over authority-qualified identities and comparator inputs for the relevant eligible lane snapshot. User-facing Project order/`POSITION` may be a lossy display projection; do not claim it equals queue claim order unless verified. Claim service remains authority and must atomically recheck candidate against current work version, graph/policy snapshot, eligibility and lease. Stale order means re-evaluate, not claim stale head.

Every accepted mutation that requires a remote operation commits canonical state, journal facts, and its required remote-operation intent in one **local** transaction; never hold that transaction over HTTP. The existing creation outbox is not evidence of a general lifecycle/priority outbox. This requirement is an implementation slice, coordinated with #9209's granular operation-intent contract; it does not imply a new database or a distributed transaction with GitHub/Project. Reconciler first drains an existing dirty-item set/cursor, then full reconciliation by an existing schedule interface; do not create a schedule/timer here. Full scan recovery is `O(N)`; dirty-set pass is `O(D)`, target pagination/rate limits may dominate. Compare desired/observed hashes/revisions, use optimistic compare-and-apply, detect echoes, verify remote effect before retry, and keep conflicts visible. Never silently drop oversized/invalid payload: return typed `payload_too_large`, `invalid_schema`, or `unsupported_version` and retain recoverable intent where authorized.

### Replay/version semantics

Replay consumes accepted normalized commands/decisions only, in per-item accepted revision order; projections are rebuilt from current snapshots and policy versions. Time-sensitive calculations pin policy hash, taxonomy hash, `asOf`, exact graph revision, item revisions, eligibility set/lane and evidence hashes. Unknown major schema or policy version is quarantined/read-only; no “best effort” numeric interpretation. Known additive fields may be ignored only if marked optional and negotiated. Same revision with divergent payload is terminal conflict; missing revision is detectable gap and triggers fetch/reconcile. Out-of-order older events are recorded as observed but cannot roll canonical state backwards. Do not use wall-clock LWW, webhook arrival, Project position, or frontend order as precedence.

## V3 — staged slices / proposed issue specs

The path consumes the existing broker, Project writer, Fleet GitHub command boundary and existing PM action/projection services. Schema ownership remains gated by Queue Table Improvements and #9073/#9074; those schema dependencies do not block offline contract fixtures. No issue is filed in this design pass.

Proposed granular prerequisite keys (reconcile in shared index before issue creation): `DST-CMD-01` shared Command/Result and string-pair identity; `DST-INTENT-01` accepted operation/idempotency record; `DST-SNAPSHOT-01` versioned projection observation; `DST-RECON-01` remote reconcile semantics; `DST-ORDER-01` canonical comparator/order hash. Do not make a child issue depend on the whole #9199/#9201/#9203/#9208/#9209 domain starter when a contract key is the actual prerequisite; that can create lifecycle/completion cycles. Existing external issue gates are cited only where the specific owned capability is required.

In the issue table, bare Rust paths `src/work/...` and `src/turso_queue.rs` are relative to `pkgs/fleet/crates/fleet-queue/`; bare TypeScript `src/...` and `test/...` paths are relative to `pm-stack/projector/`. These are exact proposed paths in existing packages, not new owners or current functionality.

| Key / tier | Deliverable; owned seam candidates | Hard dependencies | Done check / oracle |
|---|---|---|---|
| `DST-01` / crucial | Shared `Command`/`Result` contract, pair identity, version, effect-separated reason-code registry, auth-vs-actor distinction and fixtures. Owns proposed `pkgs/fleet/crates/fleet-queue/src/work/contracts.rs`. | Census and common `architecture.md` interface; no metadata DB implementation required | Schema/fixtures validate all reason-code effect classes; actor ref cannot authorize; unknown code/version is typed/no mutation; exact integer-string serialization. |
| `DST-02` / crucial | Preserve ratified `expedite=15`/`next-up=25` band values losslessly. Owns existing `pm-stack/dispatch/src/validate.ts`, `pm-stack/dispatch/test/validate.test.ts`. | #7030 evidence; `DST-CMD-01`; `PRI-PROFILE-01` contract | Named TS tests prove 15/25 survive exactly and do not alias 10/20; tests are listed/nonzero; unsupported values have typed outcome. |
| `DST-03` / crucial | Accepted-intent write records work version + decision provenance + required operation intent through existing broker. Owns proposed `src/work/distribution.rs`, shared `src/work/contracts.rs`, integration `src/turso_queue.rs`; no new table before schema gate. | `DST-CMD-01`, `DST-INTENT-01`; #9209 granular operation-intent contract; Queue Table Improvements only before persisted schema | Failure-injection oracle: atomic accepted state/intent, retry idempotency, same-revision divergent-payload conflict, no fictional verified receipt. |
| `DST-04` / recommended | Extend existing Projector webhook intake as authenticated bounded transport/dedup, then fan out a typed field-scoped operation through the existing Fleet authority adapter; no new service. Reuse `pm-stack/projector/src/server.ts` route, `src/webhook.ts` HMAC/bounded-body helper, delivery ledger and hot-cursor protocol; test at `pm-stack/projector/test/priority-webhook.test.ts` plus Fleet authority module. | `DST-CMD-01`; granular `AUTH-OP-01` from #9208; `DST-INTENT-01`; existing Fleet command adapter contract. Projector deployment liveness remains unverified. | Delivery GUID dedup is distinct from Fleet operation idempotency; Projector store remains disposable/non-authoritative; authenticated principal/capability is checked by Fleet authority; no projector GH/Project mutation; unauthorized/conflicting label cannot mutate canonical state; final-label removal preserves score and requests triage. |
| `DST-05` / recommended | Versioned desired/observed snapshot reconciler with dirty cursor + full audit. Owns proposed `pm-stack/projector/src/priority-reconcile.ts`, `test/priority-reconcile.test.ts`. | `DST-INTENT-01`, `DST-SNAPSHOT-01`, `DST-RECON-01`; Project single-writer baseline | Reorder/duplicate/outage/rate-limit/partial-effect fixtures converge or report conflict; full scan finds omitted dirty marker; no second Project writer. |
| `DST-06` / recommended | Cross-target fixture and canonical order hash. Owns Rust contract fixtures in `pkgs/fleet/crates/fleet-queue/src/work/distribution.rs` and existing hosted adapter test surfaces. | `DST-ORDER-01`, `PRI-CMP-01`; #9079/#9081 are consumers, not starter blockers | Same authority/profile/snapshot yields byte-identical canonical IDs/reasons/hash; version/target divergence fails. |
| `DST-07` / contingent | Typed degraded/unsupported/stale behavior and payload limits. Owns proposed `src/work/distribution.rs`, `src/work/contracts.rs`, and the same crate's fixture tests. | `DST-CMD-01`, `DST-SNAPSHOT-01`; #9209 granular receipt contract | Stale/unsupported never reports fresh/applied; oversize/unknown version is typed and retains authorized recovery intent. |

The files above are proposed future owned paths in existing modules, not existing functionality. Storage remains existing queue broker (`turso_queue.rs`); GitHub mutation uses existing Fleet `gh` boundary (#8887/#9016); Project writes use orchestrator; PM UI uses existing action service; hosted work consumes #9079/#9081. Do not add a competing GitHub client or priority DB. Hard issue edges must reference granular contract keys above, not whole domain starters. #9073/#9074 are hard only before persisted metadata/schema writes, not for offline schemas/evaluators; hosted and mobile presentation are soft consumers.

**Future red-first test commands / oracles per issue:** Exact future Rust test commands are: DST-01 `cargo test --manifest-path pkgs/fleet/Cargo.toml -p fleet-queue --lib --features client,queue-broker pmo_distribution_command_identity_reason_and_version`; DST-03 `cargo test --manifest-path pkgs/fleet/Cargo.toml -p fleet-queue --lib --features client,queue-broker pmo_distribution_atomic_intent_revision`; DST-05 `cargo test --manifest-path pkgs/fleet/Cargo.toml -p fleet-queue --lib --features client,queue-broker pmo_distribution_projection_replay`; DST-06 `cargo test --manifest-path pkgs/fleet/Cargo.toml -p fleet-queue --lib --features client,queue-broker pmo_distribution_host_order_hash`; DST-07 `cargo test --manifest-path pkgs/fleet/Cargo.toml -p fleet-queue --lib --features client,queue-broker pmo_distribution_unknown_version_degrades`. Before each, run that exact command with `-- --list` and assert the named test exists; zero-test success is not acceptance. DST-01 oracle enumerates each reason code and checks field-effect class/no mutation on rejection or unsupported code. DST-02: `cd pm-stack/dispatch && bun test test/validate.test.ts` proves 15/25 round-trip exactly, not current snapping. DST-04 first asserts `test -f pm-stack/projector/test/priority-webhook.test.ts`, then runs `cd pm-stack/projector && bun test test/priority-webhook.test.ts`; DST-05 likewise asserts `test -f pm-stack/projector/test/priority-reconcile.test.ts` before `cd pm-stack/projector && bun test test/priority-reconcile.test.ts`. Both projector oracles cover replay, stale revision, partial effects and readback repair. These are future red-first specs, not current tests. No test may trust `actor_ref` or client-supplied authorization. Hard blockers are granular keys above; #9079/#9081 hosted and #9201 mobile are soft consumers.

## V4 — hosted contract conformance

Consume **#9079 PM service** and **#9081 Fleet-tool equivalent**. Require both to: parse same schema/profile versions; preserve canonical ID and expected revision; return same typed conflict/unsupported/stale statuses; calculate same order hash on same complete snapshot; maintain reason/provenance; and project desired-vs-observed target status. Storage/RPC may differ, semantics may not. During version skew, older target remains read-only for unsupported major versions and caller sees `unsupported_version`; no silent coercion or divergent order. #9078 closure is not acceptance evidence: #9079–#9087 remain open per census. Live deployment, Project reconciliation and actual cross-hosted parity need separate verification issues.

## Alternatives considered (preserve as future choices)

* Full event-sourced priority aggregate: strongest historical replay but high reducer/version/retention cost and no benefit that justifies rewrite now; revisit if audit/replay need exceeds append decision snapshots.
* Multi-master/LWW/CRDT priority: rejected for canonical fields because authenticated field authority and expected revisions are required; source timestamps/clocks cannot safely settle human intent. Could revisit for non-authoritative cached activity only.
* Kafka/global event bus: rejected as over-scope; key ordering is partition-local and current broker already exists. A future transport could carry the contract if scale demonstrates need; it cannot replace authoritative ordering.
* GitHub Project or labels as canonical priority: rejected because Project is documented projection/single-writer and label taxonomy cannot represent full pressure/provenance; preserve as authorized command input only.

## Consolidated FDR links and verification boundary

Use the existing eight records in [`../decisions/README.md`](../decisions/README.md); do not mint extra FDR-PMO IDs or reuse #9180's reserved unprefixed seed IDs. Canonical broker authority, shared command seam, and producer/consumer boundary alternatives belong under [FDR-PMO-001](../decisions/FDR-PMO-001-canonical-broker-command-seam.md). Priority profiles, precedence, versions, ordering and provenance belong under [FDR-PMO-002](../decisions/FDR-PMO-002-priority-profiles-ordering-provenance.md). Durable dispatch effects/receipts belong under [FDR-PMO-007](../decisions/FDR-PMO-007-direct-dispatch-receipts.md). Preserve the alternatives described above as sections within those records, not phantom IDs. Their statuses are proposed until the shared index records adoption.

Contract test oracle is shared-schema validators + state-machine/reference model + cross-target fixture order hash; generated event sequences cover duplicate/reordered delivery, stale work versions, divergent same-version requests, version skew, webhook replay, cursor gaps, projection failure and full rescan. No live GitHub/Project write, queue schema migration, dispatch action or production rollout is approved here.
