# #9206 design: work-item lifecycle and Fleet subcommands

**Status:** proposed V2 contract and V3 issue-slice design; not an implementation report, ADR, or
schema authorization. **Pinned code baseline:** `a038f084dbeb525494d7bb928ad91f597db2ac51`.
**Inputs:** source 01 §“Command/features inventory” and §“Requirements that must survive refinement”;
source 02 §§10–16; source 03 §§2–6, 8–16 and command catalog §§4.1–4.8; recovered source 04 line 13
(flattened original; preserve all surfaced examples); `research/track-b-state-of-art.md`; #9178
claim baseline; #9179 capability limits; #9180 FDR process; #9016/#8887 existing `fleet gh` path.

**Implementation location constraint:** eventual code belongs inside the existing
`pkgs/fleet/crates/fleet-queue` package, using the exact parent architecture's proposed
`src/work/{contracts,priority,distribution,selection,lifecycle,context,activity,relations,journal,authority,operations,completion}.rs`
internal modules. These files do not exist yet. `CLI-02` owns the contract and strict parser in
`work/contracts.rs` plus new `work/cli.rs` (parent architecture may add this proposed file).
`CLI-13` alone owns `fleet-queue/src/lib.rs`, existing `client.rs`, `fleet/src/main.rs`, registry,
capability registration and approved aliases. Other slices expose typed handlers and must not edit
that shared glue. No new crate or binary. The Cargo package already has
`client` and `queue-broker` features and property tests. Intended command for targeted crate
verification is `cargo test --manifest-path pkgs/fleet/Cargo.toml -p fleet-queue --lib
--features client,queue-broker <filter>` once each named test is implemented; a filter named in an
issue below is a future test function, not an assertion that it exists today.
This worker does not edit the shared `architecture.md`; the parent owns integrating the proposed
`work/cli.rs` seam there.

**Future-test guard:** every named issue test below must be unique. Before treating its filtered
`cargo test` as acceptance, run the same package/features with `-- --list`, exact-match the function
name and assert exactly one match; a zero-test filter may otherwise exit successfully. Then run the
filter and record both commands/results. The `fleet` binary registration test uses the matching
`-p fleet --bin fleet --features full` target and the same exact-one-list check.

## 1. Purpose and non-goals

Add a coherent agent-facing lifecycle interface to the **existing Fleet multicall binary and
queue/PM authorities**. This component defines contracts and implementation-sized work slices for
the proposed `fleet work` namespace, its shared invocation/result contract, and how it composes
existing queue, hydration, activity, dependency, GitHub and session capabilities. It does not
replace those owners, ratify new queue semantics, or authorize production changes.

Hard bounds: no new scheduler, database, resolver, binary, alternate GitHub adapter, or general
creation-outbox producer. Use existing Fleet queue, `fleet gh` rotation-aware GitHub path and their
owners. The creation outbox is a bounded pilot with exactly one producer, `new-issue-creator-code-a`
(`doc/fleet/creation-outbox.md:54–60`); do not widen it. #9178 claim semantics and #9179 capability
limits remain upstream prerequisites, not silently changed by this design. Proposal spellings are
compatibility candidates: introducing an exact spelling or alias requires an explicit command
contract and must not reinterpret an existing verb.

## 2. Version boundaries

| Version                  | Contents                                                                                                                                                                                                                                                                                                                   | Exit condition                                                                                                                   |
| ------------------------ | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | -------------------------------------------------------------------------------------------------------------------------------- |
| **V1 — pinned baseline** | Current `fleet queue` multicall and `repo-queue` alias; existing queue task states and operations; current claim/lease behavior and capability contracts only to the extent #9178 evidence establishes them; `fleet gh`; scoped creator outbox. See `census/track-b.md` and `census/commands.jsonl`.                       | Evidence says only what exact source/parser/tests establish. No deployment claim; do not infer a secret token or monotone fence. |
| **V2 — contracts**       | Versioned command envelope/result/error; canonical work/resource identity; lifecycle/readiness/ownership/execution/remote-state separation; state transition table; compatibility spellings; auth and lease/fence preconditions; operation and receipt lifecycle; hydration, cursor, dependency, journal and outbox seams. | Reviewed component contracts and FDR(s), upstream issue checks, conformance examples. No schema migration implied.               |
| **V3 — staged slices**   | Add/test capability units behind existing Fleet seams, lowest-risk contracts first; migrate/adapt only after storage authority and #9178/#9179 review. Direct completion and recovery last among dependent stages.                                                                                                         | Each slice meets its own oracle; no broad “build lifecycle CLI” ticket.                                                          |
| **V4 — hosted parity**   | Reuse contracts/conformance suite in hosted editions #9079/#9081; implementation may differ while semantics match.                                                                                                                                                                                                         | Same MBT vectors pass on each supported target; evidence distinguishes hosted behavior/deployment.                               |

## 3. Domain state: four independent dimensions

Do not make one status field answer several different questions. The proposed aggregate projection
has independent dimensions:

| Dimension           | V2 concept                             | Examples / invariant                                                                                                                                                                                                                                             |
| ------------------- | -------------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Workflow            | lifecycle phase of canonical work item | `open`, `active`, `parked`, `blocked`, `completing`, `completed`, `cancelled`, `failed`; transitions are validated, not arbitrary assignment.                                                                                                                    |
| Readiness           | eligibility result derived from policy | `eligible`, `ineligible`, `unknown` plus blockers/reasons; task `requires` and claimant filter are both predicates, not OR alternatives; readiness is not lifecycle status. Persistent `manual_only` is eligibility, not the separate one-shot Do Now directive. |
| Ownership/execution | current claim and execution evidence   | `unclaimed` or owner/session + lease generation/expiry; execution receipt `not_started`, `requested`, `delivered`, `acknowledged`, `started`, `checkpointed`, `terminal`. A lease is not proof a session started.                                                |
| Remote resource     | provider-native state per typed link   | issue open/closed, PR draft/open/merged, review/check snapshots; never inferred from Fleet lifecycle.                                                                                                                                                            |

Keep current queue vocabulary stable while mapping deliberately. Current queue DB source defines
`queued` default (`turso_queue.rs:888–898`) and code handles `parked`, `blocked`, `done`,
`cancelled`, `failed`; source references include `turso_queue.rs:1117–1442` and dependent release
logic around :291–304. These are current task/queue states. Proposal's `available`, `claimed`,
`in_progress`, `completing`, `completed` are not a ratified replacement. V2 adapter must expose a
loss-aware mapping (e.g. queue `queued` can project workflow `open`, queue `done` may support
workflow completed only with verified completion receipt); never silently rename stored values.
Claimed is principally ownership, not necessarily workflow. `blocked` due to a hard dependency and
manually blocked workflow need source/reason distinction. Existing queue lease records use worker
identity/expiry; a secret token or monotone fence must not be inferred without #9178 evidence.

### Pinned executable baseline and its boundaries

The direct client dispatch is `pkgs/fleet/crates/fleet-queue/src/client.rs:5301–5345`:
`validate`, `enqueue`, `list`, `claim`, `release`, `lane`, `bind`, `heartbeat`, `checkpoint`,
`park`, `wake`, `block`, `unblock`, `dep-recheck`, `cancel`, `revive`, `fail`, `complete`,
`timeline`, `health`, `signal`, `fallback`, `dispatch`, `session`, `surfaces`, `outbox`, `stack`.
Exact function anchors include `cmd_list` :1650, `cmd_claim` :2230, `cmd_park` :2914,
`cmd_complete` :3367; parser names/options must be checked per function. This is why the requested
`fleet queue park` is implemented and `fleet queue search` is absent despite the adjacent list
operation. `repo-queue` resolves to this same CLI; `fleet queue` is the Fleet multicall domain
wrapper.

Completion-related work is not wholly absent. `fleet-queue/src/merge_intent.rs` contains the pure
core state table and validators; `client.rs:4237+` validates/records stack intent and
`:4615–5193` executes resumable replay including merge and land actions. Queue `complete` is
dispatched at :3367. These are real current primitives with specific stack/queue semantics; none
proves the proposal's complete `fleet work complete` saga (linked issue closures, cross-resource
verification, operation receipt, direct lifecycle transition and crash recovery). Treat this as
**partial substrate, exact work command absent**, not “completion absent”.

The `fleet gh` target is registered through `pkgs/fleet/crates/fleet/src/main.rs:832–837` and
delegates to the real `gh` binary under token rotation. There is no separate `fleet-gh` crate.
Metadata-aware create/query behavior is scoped by #8887/#9016 and the local PM genealogy design;
exact Metadata verbs need parser audit. The integration target is this path, not a new GitHub
adapter. Current `repo-queue outbox` is `enqueue|list|metrics|claim|resolve|drain`
(`client.rs:5281–5298`), not proposed `fleet sync outbox inspect|retry`. Creation outbox's
creator-code-a one-producer constraint remains separate and bounded.

### Selection and peer-order boundary (consumes #9204/#9200)

This lifecycle component consumes selection policy; it does not define a competing ranker. A
candidate must satisfy **both** the task's declared requirements against claimer capabilities and
the claimer's explicit task filter; the empty defaults are true. Apply hard eligibility and
revalidate it inside atomic claim. Persisted `manual_only` is an eligibility condition, not a Do
Now request. Direct dispatch reduces waiting only; it does not bypass authorization, hard
dependencies, existing live lease or applicable concurrency limits.

Preserve original before/after placement requests and the exact peer-rank model. Relative placement
is an explicit rank edit within equal urgency; moving across urgency requires an explicit urgency
change or returns a clear conflict, never silently overriding the urgency comparator. Exact
integer rank values, stable tie-breaks and rebalance are owned by #9200/#9204 and the shared
architecture. Selection sampling has no automatic exclusive claims; disposition persistence and
exploration policy belong to #9204.

### Candidate transition matrix (contract for V2 review)

| From → To                          | Required actor/preconditions                                                                                | Atomic Fleet effects                                                                                  | Remote/outbox effect                                                          | Failure/result                                                                                                                     |
| ---------------------------------- | ----------------------------------------------------------------------------------------------------------- | ----------------------------------------------------------------------------------------------------- | ----------------------------------------------------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------- |
| open → active                      | authorized worker; readiness eligible; claim path accepts exact #9178 policy; expected version              | claim/fence generation + journal event; session binding only if requested and verified                | none required                                                                 | conflict/blocked returns current state; partial claim/bind preserves committed claim receipt, never tells caller to reclaim        |
| active → parked                    | current lease generation; reason; checkpoint optional but durable if requested                              | journal checkpoint, lifecycle projection, ownership release ordered under domain contract             | optional sync intent in existing mutation/outbox owner                        | if checkpoint fails, do not report parked; lease release failure yields explicit partial state                                     |
| parked → active                    | authorized resume; blockers rechecked; activity refresh per request                                         | new claim gets strictly greater fence; resume event; checkpoint remains                               | hydration refresh may follow                                                  | blocked remains parked/blocked with all reasons; stale version conflict                                                            |
| open/active → blocked              | validated blocker or explicit block reason; current owner required for active                               | append blocker/journal evidence; do not erase ownership unless policy explicitly transitions/releases | project sync through existing owner if requested                              | a blocking-edge cycle/authorization/version failure leaves prior projection unchanged; whole-domain cycle rejection is not default |
| blocked → open/active              | all hard blockers resolved or authorized explicit override; readiness re-evaluated                          | append resolution event; preserve resolved dependency history                                         | none                                                                          | unresolved blockers remain; no blanket unblock                                                                                     |
| active → completing                | current lease/fence, expected version, all policy preconditions; completion request has stable operation ID | record completion intent and `completing`; journal exact target/effects                               | issue/PR operation intents through existing `fleet gh`/mutation-outbox owners | conflict/partial/unknown is durable; timeout does not cancel                                                                       |
| completing → completed             | all required remote effects authoritatively verified; operation valid; direct Fleet queue close succeeds    | one local transition+receipt; release ownership; final journal event                                  | no webhook wait; duplicate later webhook no-op                                | failed/unknown remote effect cannot yield completed                                                                                |
| any nonterminal → cancelled/failed | authorized policy actor and explicit reason; no unsafe pending effect                                       | append terminal event; retain history and operation state                                             | pending external effects reconciled, not discarded                            | uncertain remote effect remains visible; no false terminal proof                                                                   |

This is a proposed interface model, not a new table or mutation policy. The V2 contract must map
each transition to present #9178 behavior and upstream issue comment/FDR evidence before any
implementation. Manual overrides and admin transitions require separate permission and audit.

## 4. Command interface

The interface is a **strictly parsed command envelope into one validated Fleet service seam**;
shortcuts are compositions over that same envelope/service. No client-side policy or independent
resolver. Unknown flags/arguments fail before side effects (repo operator-script argument contract
is a local precedent, not automatic CLI conformance).

```json
{
  "schema_version": "fleet.command/v1",
  "operation_id": "op_<stable-id>",
  "idempotency_key": "client-stable-key",
  "request_digest": "sha256:…",
  "work": { "authority_id": "local-fleet", "work_id": "842" },
  "kind": "work.park",
  "arguments": { "checkpoint": true, "release_lease": true },
  "actor_ref": "worker-07",
  "source_ref": "cli",
  "session_ref": "ses_…",
  "expected_work_version": "12",
  "lease_generation": "4",
  "observed_snapshot_ref": "snapshot-<pinned-revision>",
  "owner_policy_ref": null,
  "reason": { "code": "awaiting_maintainer", "evidence_refs": [] },
  "dry_run": false
}
```

Normative constraints for V2 review:

- This follows the shared interface in `architecture.md`; wire names are `snake_case`. Integer
  IDs, versions, generations and rank keys are canonical decimal strings wherever they may exceed
  JavaScript's exact range. `(authority_id, work_id)` is canonical identity; `fleet://work/842` is
  local-authority shorthand only. `actor_ref` is attribution, never proof of authorization:
  authenticated principal/capabilities come from trusted request context.
- `operation_id` is assigned before effects and returned even on pending/partial result. Idempotency
  scope is actor + key with canonical request digest. Same key/same digest resolves to original
  receipt; same key/different digest is conflict. Do not expose lease token in serialized output,
  logs, error details, or persisted human artifacts.
- Canonical Fleet work identity is primary. GitHub owner/repo/resource-kind/remote-ID is a typed
  locator/link, not global Fleet identity. Inference is permitted only when exactly one match
  exists; cross-repo ambiguous mutations reject with `AMBIGUOUS_RESOURCE`.
- Parse whole argv strictly before opening services or doing remote I/O. Each command declares its
  exact accepted flags, aliases, values and mutual exclusions. `--repo`, `--issue`, `--pr`,
  `--work`, actor/session, lease, time/cursor, file and output context are not globally accepted by
  implication.
- Expected Fleet version and lease generation are separate preconditions. The local mutation
  checks both atomically. A request already authorized as a durable remote effect remains
  recoverable after lease expiry; expiry cannot revoke GitHub work already in flight.
- `dry_run` returns the exact planned local/remote effects and checks authorization/readiness but
  makes no work-state, journal, outbox or remote mutation. Operational telemetry that is explicitly
  outside the domain transaction may be emitted separately; it must not be represented as a
  committed command/audit effect or alter the command result.
- Time budget bounds caller wait, not operation lifetime. Timeout yields a receipt identifying
  whether the intent was accepted, running, pending sync, verified, partial, conflict or uncertain.
  It does not claim remote cancellation or that the operation failed. Resume uses operation ID.

### Result and error envelopes

The common wire identifiers and exact intent hashing DTO/framing are fixed in
[`architecture.md`](../architecture.md#digest-and-serialization-contract):
`fleet.command/v1`, `fleet.result/v1`, and the separate `fleet.command-intent/v1`
digest purpose. CLI-02 must test fixed default/null semantics, excluded delivery/wait/output
metadata, included state/fence/snapshot/owner-policy preconditions, RFC 8785 canonical bytes,
and `sha256:` lowercase-hex output. Hash agreement never authorizes an actor or an effect.

```json
{
  "schema_version": "fleet.result/v1",
  "operation_id": "op_<stable-id>",
  "work": { "authority_id": "local-fleet", "work_id": "842" },
  "work_version": "13",
  "outcome": "pending_sync",
  "effects": [{ "kind": "release_lease", "status": "verified" }],
  "warnings": [],
  "next_action": "work inspect --work work_… --operation op_<stable-id>",
  "error": null
}
```

Use architecture outcomes: `rejected`, `accepted`, `running`, `pending_sync`, `partial`, `conflict`,
`uncertain`, `verified_complete`. A read-only operation may return verified observations without
marking work complete; effects name requested, observed and verified state separately. A stable
error object includes `code`, `retryability` (`never`,
`after_refresh`, `same_operation`, `operator`), safe message, relevant current version/resource,
operation ID, and remediation. At minimum preserve research's candidates: `NOT_FOUND`,
`AMBIGUOUS_RESOURCE`, `INVALID_TRANSITION`, `LEASE_LOST`, `FENCE_STALE`, `VERSION_CONFLICT`,
`DEPENDENCY_BLOCKED`, `DEPENDENCY_CYCLE`, `REMOTE_CONFLICT`, `REMOTE_UNAVAILABLE`,
`HYDRATION_INCOMPLETE`, `COMPLETION_PARTIAL`, `IDEMPOTENCY_KEY_REUSED`. These are contract
candidates, not claimed existing error codes. Human output renders the same envelope; exit status
signals invocation/domain result only and never substitutes for the durable receipt. Exact exit
mapping requires a V2 ADR/conformance matrix.

## 5. Composed operations

Keep primitive service operations and compatibility sugar separately named, same seam beneath:

| Source spellings                                                                  | Composition (V2 proposal)                                                                                                             | Important boundary                                                                                                                                                                             |
| --------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `fleet work next [--ready] [--claim] [--hydrate full                              | incremental]`                                                                                                                         | candidate selection → policy ranking → eligibility revalidation → atomic existing claim → optional bind → hydration → journal/receipt                                                          | Selection is not claim. Never claim three candidates just because exploration sampled three. |
| `fleet work open <ref> [--claim] [--hydrate …]`                                   | resolve explicit canonical item → inspect → optional claim/hydrate                                                                    | ambiguous ref fails before writes                                                                                                                                                              |
| `fleet work inspect/status`                                                       | read aggregate projection + journal/lease/effects/remote freshness                                                                    | no mutation; separate state dimensions in output                                                                                                                                               |
| `fleet work link`                                                                 | authorize a typed external-resource link to canonical `(authority_id, work_id)`                                                       | not read-only; require auth, expected version, idempotency/operation receipt and journal provenance; do not invent subcommands/options or unlink/update semantics absent a reviewed V2 grammar |
| `fleet work checkpoint`                                                           | journal checkpoint including cursor and artifact refs                                                                                 | checkpoint durable before any release that depends on it                                                                                                                                       |
| `fleet work park --reason … [--checkpoint] [--release-lease]`                     | checkpoint → transition → release lease under one versioned service operation; sync intent if applicable                              | outcomes represent partial step completion; no blind retry                                                                                                                                     |
| `fleet work resume [--claim] [--hydrate incremental]`                             | refresh remote/activity and blockers → re-evaluate readiness → claim new generation → incremental bundle                              | resumed item does not inherit stale lease; retain historical checkpoint                                                                                                                        |
| `fleet work handoff`                                                              | authorized dispatch/session adapter operation plus binding/proof                                                                      | queue position, label, or shell is not a launch receipt                                                                                                                                        |
| `fleet work complete --merge-pr … --close-issues … --close-queue --verify --wait` | persist completion intent → precondition/activity check → verified individual remote effects → direct canonical queue close + receipt | no webhook wait; partial/async/unknown effects are recoverable; policy must not blind-batch irreversible effects                                                                               |

Original source variants `--merge-linked-prs`, `--close-linked-issues` and proposed explicit
`--merge-pr`, `--close-issues` are separately recorded in JSONL. V2 must decide if both families
survive as aliases or if explicit refs are canonical; never drop either silently.

## 6. Security, authority and interruption

1. Existing Fleet authorization/queue policy is authoritative; this design adds no identity or
   bypass. A command includes actor, session, capability/purpose and resource scope in its audit
   receipt. `--agent-id` is not a username; session identity is not proof of authorization.
2. External comments and plain language are untrusted input. Only an authenticated, authorized
   request normalized into an allowlisted typed action may enter the shared service. The `/fleet`
   grammar is one accepted form; an authorized plain-language request on a live/parked item may be
   classified under policy and then normalized, but text itself is neither authentication,
   authorization nor executable shell. `/fleet start` on a parked item is direct dispatch only
   after policy, eligibility, duplicate-live session and hard-blocker checks, and returns actual
   delivery/bind acknowledgment status.
3. Every local write checks expected version and current lease generation/fence within its owning
   transaction. Reacquisition monotonically raises fence. GitHub does not validate Fleet fences;
   remote operations require operation-scoped authorization, endpoint preconditions where available
   (e.g. PR head SHA), then read-after-write reconciliation.
4. Checkpoints retain durable progress, pending effects, cursors, artifact paths/hashes and last
   verified step. A budget approaching timeout prompts checkpoint/handoff/park. Process timeout
   cannot be interpreted as cancellation; a later command resolves by operation ID.
5. Private journal notes are private by default. Mirror is a distinct, explicit effect with a
   separate permission and receipt. `fleet gh` owns GitHub interactions; #8887/#9016 path is
   extended, not replaced. Do not generalize the creation-only pilot outbox.

## 7. Test/acceptance design

Tests name executable oracles: lifecycle reference model, exact CLI golden/parser corpus, invariants,
or deterministic fake remote call ledger. Source requirements include fresh work; assigned work;
blocked/parked/resume; late external comment; near-timeout checkpoint; crash after remote mutation;
duplicate webhook; stale lease; conflicting managed section; partial merge/close; and actual
dispatch/bind proof. Proposed test portfolio:

- **MBT:** generate legal/illegal operation sequences over lifecycle × readiness × lease × remote
  effect state; compare returned envelopes and projections with a reference model. Oracle: same
  transition/effect result for every generated sequence.
- **PBT invariants:** at most one current lease generation; reacquire strictly increases fence;
  stale fence cannot write; completed implies all required effects verified; unknown effect never
  implies completion; same idempotency digest returns one operation ID; changed digest conflicts;
  pending operations survive a process stop; private notes never mirror without explicit consent.
- **CLI conformance:** every JSONL command and option has one strict parser test for accepted spelling,
  value, unknown-flag rejection and exit/result contract. Explicitly assert rejected aliases don't
  reach network/service calls. Every implemented current spelling remains covered by its existing
  tests; do not repurpose queue `park` while adding work `park`.
- **Concurrency/crash cuts:** two claims; stale/new worker; same key parallel calls; crash before and
  after local intent commit; remote applies then response lost; verifier unavailable; async merge
  accepted but not merged; partial issue close; cursor page committed/not committed. Oracle:
  reference state plus fake remote ledger proves no false success and a retry converges.
- **Negative/security:** ambiguous cross-repo mutations; wrong actor/capability; unauthorized
  comment; fabricated lease generation; malformed/unknown args; arbitrary slash text; private note
  in public projection; `--dry-run` mutating; timeout described as cancellation. Oracle: zero side
  effects plus exact stable error code.
- **Migration/rollback:** contract/adaptor test against current queue status vocabulary and #9178
  behavior first. No schema migration assumed. If later required, schema owner supplies expand/read
  compatibility/backfill/rollback plan and proves old clients still function; rollback must preserve
  operation receipts and avoid converting ambiguous `done` to verified completion.

Source 03 §21's `CLAIM_CONFLICT`, `LEASE_EXPIRED`, `DEPENDENCY_BLOCKED`,
`CONTEXT_INCOMPLETE`, `EXTERNAL_STATE_CHANGED`, `OUTBOX_PENDING`,
`REMOTE_MUTATION_FAILED` and `COMPLETION_INCOMPLETE` remain explicit error-registry
entries or documented versioned compatibility mappings. New names do not erase those
source contracts; test the distinct conditions rather than treating every failure as one alias.

### Performance and evidence limits

No source evidence establishes a latency/SLO budget for lifecycle commands. Set no invented hard
targets in implementation acceptance. V2 baseline measurement should separate local command parse,
queue operation, GitHub API round trips, hydration bytes/pages and synchronous completion end-to-end;
report p50/p95, sample count, fixture/live-vs-mock, rate limits, and worst-case pagination. Candidate
budgets (e.g. local inspect <100ms, no more than one remote round trip for a purely local command)
are discussion prompts only and require maintainer approval. Cost/scale risks: hydration fanout,
large diffs/comments, polling/reconciliation cadence, repeated retries, outbox contention and hosted
concurrency. Use existing scheduling and DB authorities; do not add a new worker just for CLI.

## 8. Candidate granular implementation/design issues (not filed)

Each record is a candidate actionable issue, with explicit ownership, seam, blocked-by prerequisites,
failing-test oracle, and scope. Local dependency keys are within this proposal; external IDs must be
confirmed during #9196 issue reconciliation. `REQ` labels below are source trace hooks, pending the
parent's final atomic semantic IDs. No new schema/implementation is authorized by this document.

| Key / proposed title                                                             | Owner                                                                    | Deliverable and command/option coverage                                                                                                               | File/API seam; boundaries                                                                                                                                                                                                                                | Depends on                                                                                                     | Starting failing test / measurable done oracle                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                             |
| -------------------------------------------------------------------------------- | ------------------------------------------------------------------------ | ----------------------------------------------------------------------------------------------------------------------------------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | -------------------------------------------------------------------------------------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `CLI-01` V2 lifecycle state and compatibility crosswalk                          | #9206                                                                    | Transition table and explicit loss-aware map to existing queue states; `queue.*`/`work.*`; REQ H-COMMANDS, C-10, P-6/P-13, O-PART2                    | Design-only contract in this component; implementation model is `fleet-queue/src/work/lifecycle.rs`. Must reconcile #9178/#9179; no status schema rename.                                                                                                | #9178, #9179                                                                                                   | **Failing test:** `lifecycle_queue_state_mapping_preserves_source_state`. **Done:** unique test-list check and `cargo test --manifest-path pkgs/fleet/Cargo.toml -p fleet-queue --lib --features client,queue-broker lifecycle_queue_state_mapping_preserves_source_state` pass; each source state maps loss-aware or explicitly unmapped.                                                                                                                                                                                                                                                                 |
| `CLI-02` Shared command contract and strict parser                               | #9206                                                                    | `fleet work` root, per-operation argv grammar, shared identity/output/safety flags; REQ C-10, P-2/P-4, O-PART2                                        | `pkgs/fleet/crates/fleet-queue/src/work/contracts.rs` and proposed parser `src/work/cli.rs`; no registry, `client.rs`, `lib.rs`, main or resolver ownership. Shared envelope follows `architecture.md`.                                                  | `CLI-01`, `DST-01`; FDR-PMO-001 namespace check                                                                | **Failing test:** `work_cli_rejects_unknown_args_before_side_effects`. **Done:** unique test-list check and `cargo test --manifest-path pkgs/fleet/Cargo.toml -p fleet-queue --lib --features client,queue-broker work_cli_rejects_unknown_args_before_side_effects` pass, alongside golden result/error schema cases.                                                                                                                                                                                                                                                                                     |
| `CLI-03` Read-only work inspect/status and identity resolution                   | #9206                                                                    | `work.inspect`, `work.status`, `--work/--repo/--issue/--pr`; REQ P-2/P-6, O-PART2                                                                     | `work/contracts.rs` and `work/lifecycle.rs` read projection; no independent resolver/backend and no CLI registration.                                                                                                                                    | `CLI-02`, Queue Table Improvements crosswalk                                                                   | **Failing tests:** `work_inspect_rejects_ambiguous_cross_repo_reference` and `work_inspect_by_id_rejects_nonmatching_result`. **Done:** list-check each exact name occurs once; run `cargo test --manifest-path pkgs/fleet/Cargo.toml -p fleet-queue --lib --features client,queue-broker work_inspect_rejects_ambiguous_cross_repo_reference` and `cargo test --manifest-path pkgs/fleet/Cargo.toml -p fleet-queue --lib --features client,queue-broker work_inspect_by_id_rejects_nonmatching_result`; four state dimensions/provenance represented and explicit-ID mismatch cannot report another item. |
| `CLI-12` Typed work-resource link command semantics                              | #9206, with #9208 authority and #9207 read adapter                       | `work.link` only, exact source spelling; REQ P-4 §4.3 line 405, P-6 and O-PART2                                                                       | `work/contracts.rs` resource-ref types + `work/authority.rs` authorization + `work/journal.rs` provenance; no resolver, schema, add/remove subcommands or alias invented here. Link mutation does not belong to read-only inspect/status.                | `CLI-02/03`, #9208 AUTH-01, #9207 source-ref adapter; canonical identity contract                              | **Failing test:** `work_link_requires_version_auth_and_idempotent_receipt`. **Done:** test is listed exactly once, then `cargo test --manifest-path pkgs/fleet/Cargo.toml -p fleet-queue --lib --features client,queue-broker work_link_requires_version_auth_and_idempotent_receipt` passes; ambiguous provider/repo/resource refs fail without mutation and accepted link is journaled.                                                                                                                                                                                                                  |
| `CLI-04` Shared service composition for work next/open                           | #9206 with #9204/#9209/#9207 seams                                       | `work.next/open`, `--ready`, `--claim`, `--hydrate full                                                                                               | incremental`; REQ C-9/C-10, P-5/P-7/P-8                                                                                                                                                                                                                  | Adapter calls existing selection (#9204), claim (#9209/#9178), hydrate (#9207); no duplicate rank/eligibility. | `CLI-02`, `CLI-03`, upstream claim/capability review                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                       | **Failing test:** future `work_next_revalidates_eligibility_through_shared_claim_path`. **Done:** `cargo test --manifest-path pkgs/fleet/Cargo.toml -p fleet-queue --lib --features client,queue-broker work_next_revalidates_eligibility_through_shared_claim_path` passes; selection and sugar use same service, partial receipt preserved. |
| `CLI-05` Lifecycle transition + checkpoint/park/resume/handoff receipts          | #9206 for command contract; #9207 checkpoint; #9209 fence; #9203 handoff | `work.block/unblock/park/resume/release/handoff/checkpoint`, `--reason/--checkpoint/--release-lease`, agent receipt; REQ H-SURVIVE-B, C-10/C-15, P-13 | Shared lifecycle service and existing queue/agent APIs; no new scheduler.                                                                                                                                                                                | `CLI-01/02`, #9178/#9179, #9203/#9207/#9209                                                                    | **Failing test:** future `work_park_requires_durable_checkpoint_before_release`. **Done:** `cargo test --manifest-path pkgs/fleet/Cargo.toml -p fleet-queue --lib --features client,queue-broker work_park_requires_durable_checkpoint_before_release` passes; every transition defines partial recovery and receipts separate delivery/ack/start.                                                                                                                                                                                                                                                         |
| `CLI-06` Resource-link and GitHub adapter service contracts                      | #9208 owner, #9207 read paths, #9209 mutations                           | `github.issue.*`, `github.pr.*`, `github.comment.*`, `github.events.since`; matching JSONL keys; REQ C-8/C-10/C-12, P-4/P-10/P-11, O-PART2            | `work/authority.rs` typed adapters consume existing `fleet gh`/#9016. No parser registration, alias, `client.rs`, `main.rs` or second backend in this slice.                                                                                             | `CLI-02`, #9016/#8887 parser audit; #9207/#9208/#9209 contracts                                                | **Failing test:** `github_adapter_preserves_rotation_and_remote_receipt_contract` in the fleet-queue adapter mock. **Done:** exact test-list check and `cargo test --manifest-path pkgs/fleet/Cargo.toml -p fleet-queue --lib --features client,queue-broker github_adapter_preserves_rotation_and_remote_receipt_contract` pass; read/mutation adapter behavior and effect receipts conform without exposing a second gh route.                                                                                                                                                                           |
| `CLI-07` Metadata fields, managed content and note privacy contract              | #9208                                                                    | metadata get/patch/title/summary/priority/labels/fields; note add/list/edit; matching JSONL keys; REQ C-6/C-12, P-10, O-PART2                         | `work/authority.rs` + `work/journal.rs`; consumes existing `fleet gh` Metadata owner. No parser registration or generalized creation-outbox scope.                                                                                                       | `CLI-02`, #9016/#8887, #9208 authority design                                                                  | **Failing test:** `work_private_note_never_mirrors_without_explicit_option`. **Done:** exact test-list check and `cargo test --manifest-path pkgs/fleet/Cargo.toml -p fleet-queue --lib --features client,queue-broker work_private_note_never_mirrors_without_explicit_option` pass; managed-content conflict and ownership model tests pass.                                                                                                                                                                                                                                                             |
| `CLI-08` Typed dependencies and readiness CLI contract                           | #9208; #9204 consumes readiness                                          | dependency list/add/remove/graph/blockers/ready; `--blocked-by/--blocks/--hard/--depth`; REQ C-5/C-12, P-12, O-PART2                                  | Existing #9178/#9179 dependency and capability authorities first; no second graph schema.                                                                                                                                                                | #9178/#9179, `CLI-01/02`                                                                                       | **Failing test:** future `dependency_add_rejects_hard_cycle_without_partial_edge`. **Done:** `cargo test --manifest-path pkgs/fleet/Cargo.toml -p fleet-queue --lib --features client,queue-broker dependency_add_rejects_hard_cycle_without_partial_edge` passes; generated DAGs agree with typed-edge model.                                                                                                                                                                                                                                                                                             |
| `CLI-09` Hydration/activity/cursor CLI surface                                   | #9207                                                                    | hydrate/refresh/changes/activity list/since/reconcile/watch; matching JSONL keys; REQ C-11/C-15, P-8/P-9/P-11, O-PART2                                | `work/context.rs` + `work/activity.rs`; no database, parser or registration ownership.                                                                                                                                                                   | `CLI-02`, #9207 cursor/bundle design                                                                           | **Failing test:** `activity_cursor_does_not_advance_before_page_durability`. **Done:** exact test-list check and `cargo test --manifest-path pkgs/fleet/Cargo.toml -p fleet-queue --lib --features client,queue-broker activity_cursor_does_not_advance_before_page_durability` pass; edited-old-comment fixture also passes and partial/uncertain attribution is visible.                                                                                                                                                                                                                                 |
| `CLI-10` Completion, outbox receipts and direct verified close                   | #9209                                                                    | `work.complete`, sync status/run/verify/outbox list/inspect/retry; matching JSONL keys; REQ H-SURVIVE-B, C-13/C-14/C-15, P-3/P-13/P-14                | `work/operations.rs` + `work/completion.rs`; existing merge-intent/replay/outbox substrate consumed. No parser registration or creator-pilot expansion.                                                                                                  | `CLI-01/02`, #9178/#9180, #9016/#8887; #9207 activity                                                          | **Failing test:** `work_complete_recovers_remote_success_before_queue_close`. **Done:** exact test-list check and `cargo test --manifest-path pkgs/fleet/Cargo.toml -p fleet-queue --lib --features client,queue-broker work_complete_recovers_remote_success_before_queue_close` pass with apply-then-timeout remote oracle; operation resumes and direct closes only after verification.                                                                                                                                                                                                                 |
| `CLI-11` Authenticated slash/event router and agent session controls             | #9203                                                                    | each `agent.slash.*`, `agent.session.*`, `--mode/--lane/--purpose`; REQ H-SURVIVE-A/B, C-7/C-8/C-15, P-4/P-11, O-PART2                                | Typed action/event/session handlers behind `work/authority.rs` and `work/operations.rs` only. No `lib.rs`, `client.rs`, `fleet/src/main.rs`, registry, global capability registration or `fleet gh` alias ownership. Existing `repo-queue session status | result                                                                                                         | list` remains compatible.                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                  | `CLI-02/05`, existing session APIs and issue #6533 launch proof                                                                                                                                                                                                                                                                               | **Failing test:** `slash_command_duplicate_delivery_creates_one_receipt`. **Done:** exact test-list check and `cargo test --manifest-path pkgs/fleet/Cargo.toml -p fleet-queue --lib --features client,queue-broker slash_command_duplicate_delivery_creates_one_receipt` pass; forged prose rejected; receipt distinguishes delivery/ack/start/terminal. |
| `CLI-13` Single Fleet command registration, capability advertisement and aliases | #9206 single integration owner                                           | Registers all reviewed CLI-02/03/04/05/06/07/08/09/10/11/12 handlers and only approved `fleet gh` aliases; REQ H-COMMANDS, C-10, P-4, P-22            | Sole ownership of `fleet-queue/src/lib.rs`, `client.rs`, `pkgs/fleet/crates/fleet/src/main.rs`, fleet registry and capability registration. Does not implement domain handlers. Keep `fleet gh` registered existing path; no second binary/backend.      | `CLI-02/03/04/05/06/07/08/09/10/11/12` typed interfaces and review                                             | **Failing test:** `fleet_work_registration_matches_declared_capabilities`. **Done:** test is listed exactly once in fleet binary target, then `cargo test --manifest-path pkgs/fleet/Cargo.toml -p fleet --bin fleet --features full fleet_work_registration_matches_declared_capabilities` passes; golden help/capability output matches registered handlers and all unapproved aliases reject.                                                                                                                                                                                                           |

### Candidate issue graph and sequencing

#### Inventory key → primary slice, one row per inventory key

[`../census/command-slice-map.jsonl`](../census/command-slice-map.jsonl) is the authoritative,
machine-readable one-to-one assignment for **every** key in `census/commands.jsonl`. Its row schema
is `key`, `primary_slice`, `supporting_slices`, `starter`, and `disposition`; key sets must match
exactly (no wildcard row, implicit inheritance or omitted current/absent command). Normal
requirements map to a named local issue-slice key already present in the corresponding component;
existing V1 behavior is explicitly `V1-BASELINE` with its source evidence and support slice;
out-of-scope pilot and pending namespace decision use explicit boundary rows, not implementation
placeholders. The file preserves unfiled status: it assigns design ownership, not permission to
create issues.

The issue slices below have exact owned paths within the existing `fleet-queue` crate. All modules
are architecture paths and are proposed, not existing functionality:

| Slice    | Owned seam                                                                                   | Shared-file restriction                                                        |
| -------- | -------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------ |
| `CLI-01` | `work/lifecycle.rs` compatibility/model fixtures                                             | no schema rename                                                               |
| `CLI-02` | `work/contracts.rs` + proposed `work/cli.rs`                                                 | no registration or shared dispatcher                                           |
| `CLI-03` | `work/contracts.rs`, `work/lifecycle.rs` read view                                           | no mutations                                                                   |
| `CLI-04` | `work/selection.rs`, `work/lifecycle.rs` adapter                                             | consume selection/claim/hydration interfaces                                   |
| `CLI-05` | `work/lifecycle.rs`, `work/journal.rs` handlers                                              | no CLI registration                                                            |
| `CLI-06` | `work/authority.rs` typed GitHub adapter contract                                            | reuse `fleet gh`; no CLI aliases/backend                                       |
| `CLI-07` | `work/authority.rs`, `work/journal.rs`                                                       | no general creator-outbox expansion                                            |
| `CLI-08` | `work/relations.rs`                                                                          | no schema or CLI registration                                                  |
| `CLI-09` | `work/context.rs`, `work/activity.rs`                                                        | no DB or CLI registration                                                      |
| `CLI-10` | `work/operations.rs`, `work/completion.rs`                                                   | consume existing mutation adapters; no CLI registration                        |
| `CLI-11` | typed dispatch/event/session handlers in the existing work-module boundary                   | no registration or direct remote adapter duplication                           |
| `CLI-12` | `work/contracts.rs`, `work/authority.rs`, `work/journal.rs` resource-link semantics          | mutation, not read-only inspect; no new resolver/schema                        |
| `CLI-13` | `fleet-queue/src/lib.rs`, `client.rs`, `fleet/src/main.rs`, registry/capability registration | sole integration writer; registers reviewed handlers and approved aliases only |

The executable queue command and its `repo-queue` alias remain compatible. Shared flags are not
globally accepted by implication; every command has its own parser declaration in `CLI-02`,
registered only by `CLI-13`. `--lease` is not `--lease-seconds`, `--agent-id` is not `--worker`,
and issue/session attribution is not authentication.

Minimum order: `CLI-01` → `CLI-02` → `CLI-03`; `CLI-12` follows identity/auth contract and may proceed
independently of work lifecycle transitions. Then `CLI-04/05` follow existing #9178/#9179 review.
Independent handler seams `CLI-06/07/08/09/10/11` proceed only within their named owners; `CLI-13`
is last among the CLI slices and blocks on every CLI handler interface it registers
(CLI-02 through CLI-12). Domain handler slices owned outside the CLI family (for
example HYD-08 with work.refresh / activity.reconcile / activity.watch) are not
CLI-13 prerequisites: they register afterwards through the mechanism CLI-13
delivers, so HYD-08 is blocked by CLI-13 and not the reverse — the registration
graph stays acyclic (post-review correction, task 15381). All issues remain unfiled pending #9196 review.

## 9. Open decisions for parent integration

1. Is `fleet work` the canonical added namespace, while original `fleet github` and source 04's
   spellings become compatibility aliases onto existing `fleet gh`? Default: yes, no new backend;
   preserve every original spelling in inventory until a reviewed alias/defer decision.
2. Which lifecycle state is owned by existing queue columns vs Fleet work projection? Default:
   preserve present queue values (`queued`, `claimed`, `parked`, `blocked`, `done`, `cancelled`,
   `failed`); map loss-aware at API boundary, no silent schema rename.
3. Which source 04 “priority set/move/adjust/explain” exact operation grammar is normative? Default:
   retain every recovered spelling as candidate work assigned to #9200, avoid premature #9206
   command implementation.
4. How should `--merge-linked-prs/--close-linked-issues` coexist with explicit
   `--merge-pr/--close-issues`? Default: preserve both candidates; explicit references preferred
   for auditable safety until ratified.
5. Exact command exit codes, auth capability names, operation retention, latency budgets and
   hosted capability constraints remain to be settled by owners; no values are invented here.

## 10. Change boundary

This is design-only. It adds no code, live queue semantics, schema, dispatcher, resolver, DB,
binary, FDR ratification, GitHub issue, or queue record. Parent coordinator performs final cross-
component integration and live issue/implementation checks.
