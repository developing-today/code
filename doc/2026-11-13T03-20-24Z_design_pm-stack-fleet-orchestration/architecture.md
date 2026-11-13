# PM/Fleet orchestration — architecture and shared interface

**Scope:** design decisions for #9196, task 15381. No production authorization.
**Baseline:** `a038f084dbeb525494d7bb928ad91f597db2ac51`. V1 evidence is in
[`census/`](census/); component proposals are not V1 implementation evidence.

## Architectural choice

Keep **one existing Fleet queue/broker as the claim and canonical work-state authority**.
Accept authenticated, field-scoped commands from distributed producers; retain accepted
decisions and durable remote-operation intents; expose versioned observations to consumers.
Derive effective scheduling pressure from an identified snapshot. Do not create another
queue, scheduler, database, Fleet executable, secret resolver, or frontend policy engine.

This combines canonical state with distributed command ingress and replayable observations;
it is neither multi-master priority replication nor a wholesale event-sourcing migration.
The existing creation outbox is evidence of a bounded pattern, **not** an already-general
transactional lifecycle outbox. Its pilot is not expanded by this design pass.

"One authority" is the target interface and existing queue doctrine, **not a claim that
every V1 producer already complies**. Pinned `pm-stack/dispatch/src/queue.ts` still contains
a Supabase/PostgREST enqueue and direct task-PATCH path; #9028 already owns that bypass.
Its deployment liveness is unverified. Consume #9028: reconcile or quarantine that producer
before canonical dispatch cutover. Any temporary compatibility adapter must forward to the
authoritative broker, retain the same operation/dedup identity, verify the returned authority
and binding, and never dual-write stores or roll back to the bypass on broker failure.

```mermaid
flowchart LR
  Human[Human rulings / CLI] --> Command[Shared validated command interface]
  GH[Signed GitHub events] --> Intake[Durable intake / authorization / deduplication]
  Mobile[PM mobile actions] --> Intake
  Intake --> Command
  Command --> Broker[Existing Fleet broker / authority store]
  Broker --> Journal[Accepted decisions / journal / operation intents]
  Broker --> Selection[Snapshot-based selection / atomic claim]
  Journal --> Delivery[Existing adapter seams / verified delivery]
  Delivery --> GHRemote[GitHub issues / PRs / labels]
  Delivery --> Project[Orchestrator: sole Project writer]
  Delivery --> Session[Supported session provider]
  Broker --> Views[PM / CLI / hosted versioned observations]
  GHRemote --> Reconcile[Readback / reconciliation]
  Reconcile --> Command
```

Boxes are logical modules, **not** new running services or stores. The interfaces hide
coordination complexity from CLI, browser, webhook and hosted callers. Adapters vary at
the actual seams: source retrieval, remote mutation, artifact persistence and session control.

## Version layers

| Version | Meaning | Completion evidence |
|---|---|---|
| V1 | What pinned code, tests, documented authority and separately dated live observations actually establish | Bounded census claim with source/test link; deployment not inferred |
| V2 | Reviewed identity, authority, priority, lifecycle, command, hydration and reliability contracts | Schemas/fixtures, decisions, crosswalk and independent design review |
| V3 | Dependency-ordered implementation slices over the existing modules | Each issue's tests, migration/rollback evidence, then separately authorized rollout |
| V4 | Same semantics in the hosted PM/Fleet surfaces #9079/#9081 | Identical conformance inputs/results, plus separate deployment/live proof |

Later versions add capabilities; they do not erase V1 limitations or the preserved source
mechanisms. An implementation slice can be useful before the entire program is implemented.

## Identity and authority

1. A work reference is the pair **`(authority_id, work_id)`**, serialized as strings. The
   current queue's decimal task ID remains its ID; no renumbering or new identity store.
   `fleet://work/842` in the source is a local-authority shorthand, not a globally unique
   reference across unrelated hosted authorities. A qualified display URI may use
   `fleet://<authority_id>/work/842`. Resolve a shorthand only with an unambiguous authenticated
   authority context. Cross-target parity compares the **same authority's** items, never
   independent tenant-local row numbers.
2. GitHub `owner/repo#number` is an input/display locator. Retain stable provider resource
   and repository IDs plus kind; repository renames and equal numbers in different repos
   must not merge identities. No ambiguous cross-repo mutation may use inferred defaults.
3. Document precedence is handoff → charter → proposal → recovered original; supplement
   supplies candidates. This is separate from field authority: Fleet owns work state and
   urgency; GitHub owns external issue/PR bodies/discussion; provider owns actual session
   state; the orchestrator remains the sole Project writer. Existing metadata/session
   interfaces #9073–#9076 are consumed, not re-created.
4. Current ratified `expedite=15` / `next-up=25` priority conventions remain intact.
   A **Do Now request** is a separate durable one-shot directive. It bypasses selection
   delay, not authorization, hard dependencies, live leases or applicable concurrency
   limits. Persistent `manual_only` is an eligibility property, not a one-shot directive.

## Shared command and result interface

Component examples are illustrative until resolved into this common interface. Use a
versioned, typed envelope; do not implement a collection of independent CLI backends.

```text
Command {
  schema_version, operation_id, idempotency_key, request_digest,
  work: { authority_id, work_id }, kind, arguments,
  actor_ref, source_ref, source_event_ref?, delivery_id?, source_revision?, session_ref?,
  expected_work_version?, lease_generation?, observed_snapshot_ref?,
  reason: { code, evidence_refs[], confidence? }, dry_run
}
Result {
  schema_version, operation_id, work, work_version,
  outcome, effects[], warnings[], error?, next_action?, observed_at
}
```

- Authentication supplies the principal and capabilities. A client-provided `actor_ref`
  or `authorized: true` is **not** proof of authorization. Check current field/action
  permission at acceptance and again before delayed consequential delivery. Immutable
  accepted intent records the authenticated principal, exact permitted effect/request
  digest, authority decision and policy version. Re-check revocable permission and target
  preconditions before send; revocation stops future sends, not a request already sent.
  An authenticated broker recorder can retain independently verified late-effect evidence
  for that exact operation after lease expiry/revocation; an old worker's assertion alone
  is not verification or a grant to mutate current work state.
- Parse the complete invocation before side effects. Reject unknown flags, missing values,
  ambiguous targets, invalid ranges and incompatible version/precondition combinations.
- Reusing an idempotency key with the same canonical request returns the original operation;
  a different request digest is a conflict. Separate command identity, transport delivery
  identity, per-item revision, relay lease generation and cross-item read snapshot.
- Deduplicate `delivery_id` independently from `(source_ref, source_event_ref,
  source_revision)` and normalized command identity. Broker-assigned accepted item revisions
  are monotone: older observations cannot overwrite newer accepted state; a duplicate of
  the same revision and payload is a no-op, while divergent payloads at that revision are
  a conflict. A missing accepted revision triggers reconciliation. A delayed source event
  without trustworthy causal order requires fresh source read/triage, not timestamp LWW.
  Retain uncertain or unattributable observations rather than silently suppressing them.
- Shared wire names are `snake_case` in the eventual v1 schema. Existing component examples
  using camelCase are design notation, not additional wire spellings. Contract fixtures
  must make serialization, integer/string handling and unknown-version behavior explicit.
- Integer IDs, versions, generations and rank keys cross JSON as canonical decimal strings
  wherever they may exceed JavaScript's exact integer range. Do not coerce through floating
  point. UTC instants and snapshot/policy inputs must be pinned for deterministic replay.
- Outcomes distinguish `rejected`, `accepted`, `running`, `pending_sync`, `partial`,
  `conflict`, `uncertain`, and `verified_complete`. A particular read-only operation can
  have a verified result without completing the work item. Effects name their requested,
  observed and verified states separately.
- A timeout returns a resumable operation receipt, not fictional cancellation. Dry-run
  creates no work-state, journal, outbox or remote mutation. Capability discovery advertises
  only the commands and guarantees that target actually supports.

## Ordering and distributed priority

Field precedence and accepted revisions govern commands; arrival timestamps do not.
Preserve intrinsic urgency, nullable importance, derived pressure, workflow, ownership and
directive separately. Historic charter 450/High and 1000/default are a named source profile,
not numbers to transplant into the live queue. The existing dispatch adapter's nearest-value
normalization (`15→10`, `25→20`) is a concrete compatibility gap, not authority to change
the ratified values.

For one fixed authority/lane snapshot, first require **both** the task's requirements of
the claimer **and** the claimer's explicit filter of tasks. "Union of constraints" does
not mean OR eligibility. Default predicates are both true. Then apply hard eligibility;
then compare effective urgency, explicit exact peer rank within equal urgency, creation
time and normalized canonical ID. Record policy, graph, item revisions and as-of time.
Claim revalidates current predicates; a projection or sampled candidate cannot grant a lease.
The consistency boundary is the broker's claim transaction: validate item/work version,
graph revision, active policy/profile revision, ownership and relevant eligibility inputs
against the proposed snapshot, and evaluate time-sensitive conditions at broker-authoritative
claim time. A changed input requires recomputation/reselection or a typed stale result.
If the adapter cannot provide that atomic validation, it cannot advertise this capability.
Graph revision validation and edge insertion share a transaction/serialized edit protocol;
two individually valid opposite-edge checks must not both commit.

The source explicitly requests before/after placement: retain that feature, with exact
integer peer ranks and an auditable rebalance contract. A cross-urgency move requires an
explicit urgency change or a clear conflict; it must not secretly override the urgency key.
Equal scores are legitimate. V1 claim/list final-tie divergence remains a named defect.

Cycle-safe hard-dependency pressure is effective-only. With canonical blocker→dependent
edges, inserting `u→v` creates a cycle iff `u=v` or a prior path `v→u` exists. Graph edits
must serialize or validate a graph revision atomically, including the two-opposite-inserts
race. Incomplete evaluation fails closed; a graph budget cannot hide a blocker.

Exploration is optional, feature-disabled until calibrated. Preserve the historic parameter
profile and statistical rationale; do not apply a 200-point distance blindly to live 10–90
values. Freeze eligible population/density and initial mixture masses for a draw, remove
each chosen item, and renormalize the remaining masses without replacement. Persist all
candidate dispositions before extended action; sampling creates no exclusive claims and
does not itself justify score changes. Urgent work requiring the lane suppresses exploration.

## Lifecycle and cross-system reliability

Workflow, lease ownership, execution/session receipt and external issue/PR state are distinct
dimensions. Extend #9178 and #9179; do not declare their unresolved rulings settled here.
Current worker-ID leases are not evidence of secret lease tokens or monotone fences.

An accepted mutation commits canonical state, journal facts and required remote intent in
one local transaction. Never hold that transaction over HTTP. Use effect-specific remote
preconditions and readback; a Fleet fence cannot stop a request already in flight to GitHub.
An expired worker may report the outcome of its already-authorized operation through a
restricted receipt recorder, but cannot create new mutations or overwrite a newer owner.
Unknown remote outcomes require observation before retry, not a second blind POST.

Normal completion verifies the requested PR merge, verifies explicitly designated issue
closures, and directly finalizes the canonical queue item with its receipts. No webhook
or sweep is required on that successful path. Partial effects remain recoverable; do not
undo a successful merge merely because a later issue closure failed. A final read cannot
atomically exclude a new GitHub comment; expose the freshness boundary and residual race.
Closed epics, labels and session shells are not completion or launch proof.

Completion policy names its invalidating activity classes: changed expected PR head,
required check/review/branch-policy failure, newly unresolved hard blocker, authorization
revocation or explicit hold/cancel always stop further consequential effects. An authorized
actionable requirement comment/review observed before local finalization holds the operation
for policy evaluation; routine informational activity need not. Record the final refresh
watermark and the residual read→merge interval, not an absolute "no external changes" claim.
Activity discovered after completion appends evidence and applies an explicit post-completion
triage/reopen policy; it never silently rewrites completion history or undoes a merge.

The source's managed-body feature remains traced. GitHub endpoints without documented
conditional-write support cannot guarantee prevention of human lost updates via a mere
GET-compare-PATCH sequence. Default to private journal plus explicitly mirrored, known-ID
managed comments; any body-section mutation must expose the weaker guarantee or remain
unsupported until an adequate adapter is proven. Never quietly discard the feature.
Default capability for automatic body-section writes is **unsupported without verified
conditional-write support**. A separately approved best-effort mode must disclose lost-update
risk in its preview/receipt and retain before/after evidence; it is not optimistic concurrency.

## Implementation locality and ownership

Concrete default placement is **internal modules of the existing `fleet-queue` crate**,
not an invented crate, daemon, executable or database:

```text
pkgs/fleet/crates/fleet-queue/src/work/
  contracts.rs       # identity, command/result/version semantics
  priority.rs        # intrinsic/profile/comparator/triage evaluation
  distribution.rs    # normalized intents, revisions, projection observations
  selection.rs       # eligibility, urgent gate, exploration model
  lifecycle.rs       # composed transitions; consumes primitive interfaces
  context.rs         # immutable bundles and bounded hydration
  activity.rs        # durable page ingestion, cursors, attribution
  relations.rs       # typed graph, readiness, resolution history
  journal.rs         # private notes, corrections, checkpoints, events
  authority.rs       # field permissions and managed-content policy
  operations.rs      # operation intents, relay state, verified effect receipts
  completion.rs      # direct completion/recovery state machine
```

These are **proposed files**, not existing functionality. Keep pure contracts/evaluators
independent of broker/client feature-gated adapters. Storage still belongs to existing
`turso_queue.rs`/broker authority; GitHub uses existing Fleet `gh.rs` and #9016; PM changes
stay in existing aggregator/projector/dispatch modules. One integration slice owns shared
`lib.rs`, CLI `main.rs` and registry registration so parallel implementation does not race
over those files. New module files/tests can otherwise be independent.
Placement is a concrete implementation-planning default, not allocation of storage/schema
ownership. Reconcile it with #3424/#8575/#8579 and the QTI provenance disposition before
schema-coupled work. `operations.rs` requires its own scoped mutation-outbox issue; it does
not silently enroll this program as another producer of the bounded creation-outbox pilot.

Future Rust issue done checks use the actual workspace/package, for example:

```text
cargo test --manifest-path pkgs/fleet/Cargo.toml -p fleet-queue --lib \
  --features client,queue-broker pmo_<slice_key>
```

Each builder first adds the named failing test and proves it is listed; a test filter that
runs zero tests is not acceptance. These are future test commands, **not** tests run by
this design pass. Schema/manifest/receipt fixtures, MBT state machines and PBT invariants
need independent reference oracles rather than production code comparing with itself.

## Sequencing and gates

1. Preserve sources and semantically inventory outcomes; census/crosswalk at a pinned commit.
2. Freeze shared identity/command/result/profile/authority and compatibility fixtures.
3. Develop independent pure models and read-only adapters; retain missing subcommands as
   explicit issue work. Do not block an offline fixture on completion of an entire domain epic.
4. Reconcile shared storage/schema against the original Queue Table Improvements work;
   if provenance remains unlocated, keep an explicit research issue and a concrete schema
   compatibility decision gate, not an imaginary equivalence or an indefinite wait for Drew.
5. Implement additive journal/graph/cursor/operation storage and claim guards in dependency
   order; integrate GitHub/PM/session adapters; verify crash/retry and migration/rollback.
6. Expose composed CLI/mobile paths, then separately authorize rollout and live acceptance.
   #6533 remains the existing live launch-and-bind fixture. No duplicate dispatch is launched
   merely to make this design pass look verified.

Native blocker edges point at **granular prerequisite slices**, not entire domain parents
where that would introduce cycles. Shared-file collisions are sequencing edges. Hosted
parity, calibration, deployment and live proof remain explicit future work, not reasons
to discard the larger design or mark the program done.
