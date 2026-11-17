# Direct dispatch, slash commands, and event routing

**Component:** #9203 · **Track:** A · **Status:** implementation-ready design, not implementation authorization · **Evidence baseline:** pinned commit `a038f084dbeb525494d7bb928ad91f597db2ac51`; bounded live issue/PR evidence 2026-11-13. No live agent/session is launched by this design or its acceptance plan. V1 means code/records observed at that commit; production deployment is unverified.

## 1. Requirement and decisions

An authorized actionable comment, supported slash command, PM Stack action or direct Fleet operation should promptly reach the existing work owner: either the currently bound capable session, or a directly dispatched session when policy allows. A parked item must not merely float to the head of an ordinary queue. Routing must be replay-safe and return effects in the shared `Result` contract from `architecture.md`, distinguishing request accepted, launch accepted, session created, task bound, prompt delivered, session acknowledged/meaningfully progressing, and final verified outcome. Do not invent a competing operation/receipt schema.

Source coverage: handoff H-SURVIVE-A/H-SCENARIOS/H-COMMANDS; charter C-2, C-7/C-8/C-13–C-17; full proposal P-3/P-4/P-5/P-13; recovered O-PROMPT, O-PART2, command inventory, dispatch §§11, 14, 17–20 and cross-document decision 11; supplemental S-COMMANDS/S-MOBILE. #6533's live launch-and-bind proof remains a named acceptance fixture and is not replaced by a mock or by #8187's closed code baseline.

### Non-negotiable boundaries

- Use the shared `Command`/`Result` interface from `architecture.md` and existing `fleet` operation, `fleet gh` (`pkgs/fleet/crates/fleet/src/gh.rs`), `pm-dispatch`, queue broker and provider adapters. Do not introduce a new scheduler, queue, agent system, webhook service or dispatch database.
- **One canonical queue is a compatibility goal, not an established V1 fact across every producer.** At pinned SHA, `pm-stack/dispatch/src/server.ts:50-53,79-83,214-250` wires a Supabase-backed `QueueClient` and directly enqueues through it. Open issue #9028 specifically tracks PM dispatch bypassing the packet validator and queue broker. `pkgs/fleet/crates/fleet-dispatch/src/lib.rs:1-28` is a thin client over that API. Therefore the adapter/repoint to the canonical broker is a hard integration decision; never claim the PM dispatch path is already using the Turso broker. V2 operation contracts can proceed against an adapter seam; V3 direct-dispatch changes that rely on atomic canonical task reservation gate on reconciliation of #9028 and the queue-model/QTI research slice.
- `do-now` is an independent one-shot dispatch directive. It bypasses waiting behind ordinary selection only; it cannot bypass actor authorization, hard dependencies, task eligibility, existing lease, applicable concurrency cap, provider gate, or owner/manual-only policy. It never rewrites numeric urgency or priority label. `manual_only` excludes automatic claims; an authorized explicit owner direct-start may use the #9178 manual-claim mode without clearing `manual_only`, with its override/actor/session/reason recorded. Do Now alone does not supply that authority.
- Reuse #8187's launch-on-handoff / same-lane inline-triage behavior as current baseline. This is distinct from an authorized owner direct-dispatch request. Do not reintroduce “handoff then park and hope.”
- #9178 owns claim-by-id/manual-only/takeover/resume semantics; #9179 owns park safety. The dispatcher must call their explicit validated operation, not synthesize a claim override or unpark around those policies. #9178's explicit multi-task same-session claim proposal must remain possible: no implicit redirection or extra binding, but an authorized multi-item operation keeps per-work identity, context, checkpoint and lease/fence evidence separate. Applicable caps remain policy-owned/unresolved rather than this design imposing a blanket one-session/one-task prohibition.
- Never execute shell text from a comment. Slash commands map from a strict grammar to typed operations; free text is untrusted task data and intent classification cannot grant authority.
- Don't call a Slack post, queue item, session shell, provider API 2xx, or `session_id` alone “launched and working.” Reconcile using the durable evidence levels below.

## 2. V1 — observed baseline and open seams

- `pkgs/fleet/crates/fleet-dispatch/src/lib.rs:1-28,576-608` exposes Fleet CLI verbs `dispatch status`, `dispatch launch` (POST `/v1/sessions`, launch now), and `dispatch enqueue` (POST `/v1/tasks`, enqueue only). It resolves service auth through named Fleet secrets; it does not own PM dispatch state.
- `pm-stack/dispatch/src/server.ts:1-20,50-53,197-250` documents service authentication and route shape; checks caller repo/allowlist and directory before side effects; launch path may create tracking issue, enqueue task, start session, assign task, and annotate. Enqueue occurs through the Supabase-backed QueueClient in this version. This sequence spans external systems and is not an atomic transaction.
- `pm-stack/dispatch/src/validate.ts:11-60` maps the existing 10/20/30/40/50/70/90 lower-is-sooner scale. This is urgency/queue priority, not `do-now` authorization or a direct-dispatch request identity.
- `pm-stack/aggregator/src/actions.ts:76-139,168-239` has a typed UI action vocabulary, `clientToken`, actor, target/session references and action states. `pm-stack/aggregator/src/server.ts:1818-1858` validates Access identity and persists actions, while `src/actions.ts:9-44` says the dashboard does not itself write GitHub, Project or session services; consumers act and return results. This pattern is reusable; its existing `new/applied/rejected/superseded` lifecycle is not complete launch proof.
- **Existing projector intake (verified at pinned SHA; deployment liveness unverified):** `pm-stack/projector/src/webhook.ts:7-57` already bounds raw-body reads and verifies GitHub HMAC-SHA256 with constant-time comparison; `src/server.ts:146-201` uses it at existing `POST /github/webhook`. `pm-stack/projector/test/webhook-delivery-id.test.ts:90-184` covers invalid-signature/no-effect, missing/empty delivery IDs, same-GUID duplicate behavior and retry after freshness sink failure. README lines 23-30 specify supported issues/issue_comment/PR/review/review-comment/push events and reconciliation cursors; lines 145-179 describe the disposable hot ledger with delivery/subevent IDs and server-issued high-water ACK; `test/hot.test.ts:65-118,246-314` covers replay, page/high-water and monotone ACK. The `/queue-view` input is externally exported task JSON (`README:113-133`), not a live broker fetch. Reuse this signed ingress/fanout seam; do not build another HMAC receiver. It is still only projection ingestion: `pmp_deliveries=applied/ignored` and hot-feed consumer ACK do **not** mean a Fleet command was parsed, authorized, durably accepted or launched. The projector is disposable/read-only glue (`README:3-8`) and its tables cannot become command truth.
- The ratified priority-label convention `expedite=15` / `next-up=25` remains distinct from a one-shot `do-now` directive. Do not normalize `do-now` into priority 15 or treat `manual_only` as a directive; it is an eligibility gate.
- #8187 CLOSED is the current launch-on-handoff/inline-triage baseline. #2668 CLOSED documents a staged launch pipeline and should be consumed, not re-designed without a new demonstrated gap. Neither implies that all external events, parked direct starts or current deployments satisfy this component.
- #6533 OPEN: comments report #8066/#8367 code fixes merged; the re-scoped acceptance remains a fresh live handoff-sink fire with task `executed_lane` and `session_id` bound or a precise blocker. The issue's existence and merged fixes are not proof. The task session must produce an end-to-end acceptance packet and obtain owner-approved/live environment authorization separately.
- Bounded HMAC webhook verification, delivery dedup and disposable projection hot ledger exist at pin, but not the complete signed GitHub webhook → strict Fleet parser → per-operation authorization → canonical durable operation acceptance → active-session/direct-launch → task bind/prompt/ack pipeline. No current direct launch from an authenticated parked-item comment was established. No production integration/route or endpoint was exercised in this census.

## 3. V2 — versioned routing contract and state machines

### 3.1 Inbound sources and strict command surface

Normalize these three inputs to the shared `Command`/`Result` interface in `architecture.md`: (1) verified GitHub webhook comment event; (2) PM Stack action via #9201; (3) existing authenticated Fleet CLI/service call. Use `(authority_id,work_id)` strings as identity; `fleet://work/842` is a local shorthand only when authenticated authority context is unambiguous. GitHub `owner/repo#N` is a locator, retaining stable provider/repository IDs and resource kind. Caller/source-specific authorization stays server-side; client `actor_ref` is not proof. Wire names are snake_case and potentially unsafe integers/versions/generations serialize as canonical decimal strings. Never infer a cross-repo target from a bare number.

The grammar below is a **V2 proposed slash protocol, revision 1**, not a claim about V1/currently supported commands. Implementation requires parser/fixture and adapter acceptance before advertising a command:

```text
/fleet status
/fleet triage [now]
/fleet start
/fleet resume
/fleet retry
/fleet park [--reason <quoted text>]
/fleet unblock
/fleet priority high | /fleet priority <integer>
/fleet do-now
/fleet merge
/fleet agent status
```

Parsing rules: one command per comment unless the parser has a separately reviewed multi-command/transaction model; no `;`, shell expansions, arbitrary flags, URLs-as-executable-payload, or unrecognized trailing text. Only documented subcommands/options are accepted. Numeric priority uses the canonical #9200 policy and allowed score range, never direct SQL. `/fleet merge` always requires merge authority, current clean preview, checks/reviews/head-SHA readback and explicit confirmation; command syntax alone is not consent. `/fleet start`/resume on blocked/parked work invokes hard gates. `/fleet retry` is a typed retry request, not unlimited attempt reset. `/fleet park` requires reason/term under #9179, not a free-form status write.

Informal `/retry`, `/queue`, `/start`, `/merge` spellings found in source 05 are **alias candidates only**. Accept an alias only after an explicit mapping to one canonical `/fleet` verb is approved, documented and tested; reject ambiguous forms. Never alias an arbitrary string to shell or runner invocation. Plain-language requests are classified into candidate intents with confidence/explanation; they remain untrusted data, are separately authorized by actor policy, and ambiguous/low-confidence text is captured for triage rather than acted on. A classifier cannot decide that an author has permission.

### 3.2 Signed event intake, authorization and resolution order

For GitHub webhooks, read and retain the exact raw request bytes; validate the configured signature algorithm and constant-time signature comparison before parsing JSON; reject missing/invalid signature, oversized body, malformed event and unsupported event type before effects. Verify repo/install allowlist and delivery metadata. Event data and comment body are untrusted instructions. Recheck actor permission **per operation and target** at intake, then reauthorize at effect time (roles/repo access may change during a queued request). Require additional confirmation for merge/close/stop/priority/direct start according to policy.

Exact processing order: raw-body bounds → signature → event/schema validation → derive delivery identity → authenticate source principal (not yet action authority) → strict command parse or untrusted-text intent classification → canonical target resolve (issue vs PR, repo, linked Fleet work, parent vs blocker relations) → authorize this principal for this verb/resource and current policy → idempotency lookup scoped to that authorization (never disclose a prior receipt before auth) → fresh eligibility/lease/dependency/capacity checks → durable operation reservation → dispatch/send effect → bind/acknowledgment receipts → verified readback → notify original surface. A resolution failure or ambiguous link returns shared `Result` as `rejected` with `ambiguous_target` reason and candidate refs; do not choose nearest issue. No UI/webhook result acknowledges command acceptance until the Fleet request and idempotency record are durable.

The source webhook route/HMAC helper is the existing Projector ingress. Keep two idempotency domains: (1) projector projection delivery (`pmp_deliveries` / hot subevent IDs) and (2) Fleet operation idempotency (`operation_id`/`idempotency_key` plus request digest). For any comment event that might carry an actionable command, do not let a prior projection `applied` marker early-return before the Fleet operation adapter can return/accept its own receipt. If projector projection succeeds but Fleet command acceptance is unavailable, return retryable failure; on redelivery the projection may dedup/reuse its applied result, but forwarding is retried/reconciled against the same Fleet operation key. Only the Fleet authority's durable receipt means command accepted; projection row/ACK never does.

### 3.3 Deduplication and edit semantics

Use provider delivery ID (e.g. GitHub `X-GitHub-Delivery`) as the webhook-redelivery key, namespaced by provider/app/repository. Also define semantic action key from canonical `(authority_id,work_id)` + stable GitHub resource ID + comment node id + normalized command-span identity/version. This covers two deliveries of the same event and overlapping edit/redelivery routes. Persist the raw event digest, comment version/update timestamp, parse result, authenticated principal ref, operation IDs and eventual result in the existing authorized operation/journal home selected by the `fleet-queue/src/work/operations.rs` contract. Do not create a dispatch database or assume provider delivery ID dedupes a comment retried through another event path.

Editing a comment never silently reruns an already-consumed command. An edited command is a new versioned candidate: reauthorize, reparse, show new preview/confirmation for consequential operations, and link/supersede the prior request without erasing its receipt. Separate `comment_received` from `command_accepted`. Duplicate requests return the prior receipt, not a new direct session. Event delivery is acknowledged only after durable intake; external send/launch is asynchronous and recoverable.

### 3.4 Direct-session routing and launch reservation

For an actionable instruction on an active item:

1. Resolve the canonical work item and authoritative current queue task.
2. Query the current session adapter for the session bound to that task; check task/session/worktree relationship, session health and actual supported `send` capability. Never use only issue labels, recently seen session name, or a free text session id.
3. Reauthorize the actor and instruction for that exact target; create the operation idempotency/reservation record, then send through the adapter with operation id as correlation id.
4. Record each adapter result separately: accepted by adapter, delivery ID, delivered/posted (where knowable), session ack, meaningful progress/effect, error. For OpenCode, use its real acknowledgment/status semantics. For Slack/Muse, “message posted” is only transport delivery unless the session system returns an authenticated ack.
5. If no capable live session exists, evaluate state: direct start only if authorized, eligible, not hard-blocked, no conflicting live lease/session, capacity available and policy allows manual/parked dispatch. A hard-blocked item may route only to inspection/triage, not pretend execution started. An already-active different session is not stolen; follow #9178's explicit takeover rule if permitted.
6. Reserve the direct operation against canonical queue/task identity using the existing queue claim/bind protocol and a stable operation key. Recheck both task-declared requirements and the claim's explicit filter (AND), plus hard constraints, at the atomic claim point. If current `pm-dispatch` cannot write through that canonical owner, reconcile its Supabase/PostgREST bypass (#9028) before shipping a change that claims single-queue semantics.
7. Launch through an existing Fleet dispatch adapter; bind resulting session to the task; verify task/session binding, route/provider, worktree/directory and prompt delivery. If any stage has ambiguous timeout, query/reconcile by operation/task/session key before retry; never issue a blind second `start`.

### 3.5 No distributed transaction: reservation and compensation state

An external process/session API and the queue/GitHub store cannot share a transaction. Use one local authoritative Fleet transaction for accepted work decision, journal fact and required remote-operation intent after the storage owner and contract are decided; do not claim the existing GitHub-creation outbox is already a general lifecycle outbox or expand it in this design pass. Then call the remote adapter. Suggested receipt effect stages (not a second lifecycle/outcome enum):

```text
received → verified → authorized → resolved → reserved
  → launching → session_created → task_bound → prompt_delivered
  → acknowledged → progressing → verified_terminal
```

Map effects to shared Result outcomes: policy/authz gate is `rejected` with reason code; stale precondition is `conflict`; known partial remote effects are `partial`; ambiguous timeout is `uncertain`; known intent awaiting readback is `pending_sync`; verified requested effect is `verified_complete`. Keep a detailed effect-level reason such as `blocked`, `send_failed`, `launch_failed`, `bind_failed`, or `ack_timeout`; these are not new top-level outcomes. Maintain operation outcome separately from underlying task lifecycle; do not mutate task status to “in progress” until verified claim/session binding under lifecycle policy. `accepted` means only durable acceptance into Fleet. If launch times out after remote creation, return a resumable receipt as `uncertain` and reconcile provider by request metadata/work identity; if a session is found, bind/attach effect receipt through authorized recovery; if not found, policy may permit a retry. Never create a second live session to “fix” a missing response. Orphan session is surfaced as an unresolved binding and handled by existing ownership/recovery rules, not automatically stopped.

The shared `Result` effect receipt includes operation/idempotency key, `(authority_id,work_id)`, event delivery/source/comment references, authenticated principal/capabilities, policy version, timestamped stages, expected/observed work versions, source-before/source-after, effect diff, dispatch directive separately from urgency, route/provider/adapter, actual model if returned, lease owner/expiry and generation only where the chosen contract provides it, actual session reference, worktree/directory reference, prompt-delivery receipt, session ack/progress evidence, final error/reason code, verification readback and original issue/comment locator. Use strings for potentially unsafe identifiers/counters. V1 worker-ID lease records do not prove secret lease tokens or monotone fencing. Never record credentials or token values. Keep task/session binding evidence in the existing authority, not only a GitHub note. A Fleet fence cannot stop an already-in-flight remote GitHub operation; expired-owner receipt reporting may record that operation's outcome but cannot initiate another mutation or overwrite newer ownership.

### 3.6 Paused/blocked/fairness and one-shot directive

- `do-now` bypasses ordinary wait/selection, but neither hard requirements nor lease policy. A request against hard-blocked work returns `rejected` with an `ineligible_blocker` reason plus blocker refs, or a permitted inspection/triage operation. It cannot make a dependency ready or clear `manual_only`; explicit authorized direct start may use #9178's manual-claim mode while leaving that flag intact and preserving the override audit.
- A parked task requires explicit authorized start/resume operation and #9178/#9179 policy; preserve park reason, owner, checkpoint and next action in result. Never reset attempt history implicitly. If a matching active binding exists, route to that work item; otherwise do not silently change session binding or worktree. #9178's multi-item same-session operation remains available only through its explicit validated semantics, with contexts/checkpoints/ownership separately identified.
- No implicit redirection/binding of another item onto a live session. Explicit authorized multi-task claims remain possible under #9178 and must keep per-work context, checkpoint and ownership/fence evidence independent. Do not impose a blanket one-session/one-task ban in this component; batch/cap semantics remain with the #9178/owner policy.
- Applicable capacity/capacity leases are checked at reservation and remote start according to the owning policy. No ad hoc lane bypass or arbitrary dispatch count per comment. Any unresolved/provider-specific limit remains explicit, names its source or `unknown`, and produces a reason in the shared result rather than a newly invented cap.
- One-shot directive transitions `requested→reserved→consumed|expired|rejected`, carries item/actor/operation and expiry/reason; it never becomes a persistent “critical” score or an unlimited priority boost. A repeated delivery references the same directive/operation.

### 3.7 Security and fail-safe matrix

| Input/failure                                                 | Required behavior                                                       | No-go behavior                                                                 |
| ------------------------------------------------------------- | ----------------------------------------------------------------------- | ------------------------------------------------------------------------------ |
| Bad/missing signature, spoofed repo, unsupported webhook      | Reject before queue/session/Project/GitHub effect; audit bounded denial | Parsing body then performing effects                                           |
| Authorized actor posts unknown command                        | Store/return invalid-command result or ordinary comment classification  | Shell, URL fetch, model tool execution or silently enqueueing arbitrary prompt |
| Plain-language classifier uncertain                           | Surface candidate for triage; no direct effect                          | Treat classifier output as auth or exact executable command                    |
| Actor authorized at receipt but revoked before dispatch       | Reauthorize, reject/expire and record                                   | Apply stale authorization from webhook time                                    |
| Duplicate event/command or retried browser request            | Return existing receipt from stable idempotency key                     | Double session, comment, merge or queue item                                   |
| Ambiguous issue/task mapping                                  | Block with canonical candidates and request resolution                  | Guess by numeric id or title similarity                                        |
| Task blocked/parked/manual-only/lease active                  | Apply #9178/#9179 gates; return blocked or explicit permitted recovery  | Do-now as gate bypass, hidden park clearing, implicit takeover                 |
| Provider unavailable or request times out after remote effect | Record ambiguous result, reconcile existing session and task by key     | Blind launch retry or claim success from HTTP acceptance                       |
| Session supports no send/resume                               | Show actual unsupported capability and documented alternate             | Fake button, Slack post presented as session ack                               |
| Queue/dispatch producer uses wrong store                      | Fail closed on canonical route or surface integration mismatch          | Write directly to a second store or report canonical queue state               |

## 4. V3 — implementation slices (issue specs, not filed)

Each spec is independently reviewable and must be filed as one child under #9203 if accepted. Sequence shared operation/id contracts before adapters; production proof has a separate approval gate. Commands below are suggested test entrypoints to confirm with owning package scripts; no code test was run during this design-only pass.

### DSP-01 — Signed webhook intake and actor authorization boundary

- **Requirements:** C-8, H-SURVIVE-A, O §§14.2/14.6. **Goal:** accept only signed, bounded, supported GitHub events and derive actor identity from verified provider context.
- **Owned files:** reuse/extend `pm-stack/projector/src/webhook.ts`, its route in `src/server.ts`, and existing delivery-dedup/fanout only as ingress to the canonical Fleet operation interface; Fleet actor/action authorization and durable acceptance remain at that authority. Extend existing projector webhook tests and the Fleet owner tests as needed. Do not add a second webhook/HMAC service, make projector persistence canonical, or place policy/remote writes in projection glue. This slice proves full authorization and durable Fleet operation acceptance, not just projection apply.
- **Failing test to start:** `operation_forward_retries_after_projector_delivery_is_applied` in proposed `pm-stack/projector/test/webhook-operation-forwarding.test.ts`: first event projects successfully but Fleet command acceptance fails; on redelivery the existing `deliveryApplied` shortcut must not suppress the separately idempotent operation forwarder.
- **Owned adapter files:** reuse `pm-stack/projector/src/server.ts` route and `src/webhook.ts` HMAC helper; add only in-process `src/operation-forwarder.ts` to send a signed-source event envelope into the existing authoritative Fleet operation interface. On the Fleet side use the existing `fleet-queue/src/work/operations.rs` + `authority.rs` contracts integrated into the existing queue-broker handler; no second service/database. `projector` forwards event context only and never stores canonical command acceptance.
- **Done/oracle:** current signature vectors and bounded-body tests remain green; tampered/missing signature and oversized body yield zero Fleet effects. New projector test proves the command forward is retried/reconciled even when `pmp_deliveries` says projection already applied; a second delivery invokes exactly one command effect and receives the stored Fleet receipt. Fleet tests prove authenticated principal + per-verb/resource authorization and durable command idempotency independently of projector delivery dedup. Commands: `cd pm-stack/projector && bun test test/webhook-delivery-id.test.ts test/hot.test.ts test/webhook-operation-forwarding.test.ts`; `cargo test --manifest-path pkgs/fleet/Cargo.toml -p fleet-queue --lib --features client,queue-broker pmo_dsp_01_command_acceptance_is_durable_separate_from_projection_delivery`. First prove each new named test is listed; zero-test success is not acceptance. Existing endpoint's deployed/fanout liveness stays unverified until an authorized deployment/readback check; no second ingress service and no production mutation in this design pass.
- **Dependencies:** #9016/#8887 authorized GitHub boundary; no direct production mutation until operator approval.

### DSP-02 — Strict slash grammar and command-to-operation mapping

- **Requirements:** H-COMMANDS, C-8, O command inventory, S-COMMANDS. **Goal:** typed parser for exactly V2 §3.1 grammar and separately reviewed alias candidates.
- **Owned files:** shared operation parser in the existing `fleet-queue/src/work/contracts.rs` seam (or existing ingress parser adapter), parser fixtures and compatibility docs. No shell exec. Defer CLI registration in `pkgs/fleet/crates/fleet/src/main.rs` and `fleet-registry` to one integration glue issue after domain work; do not add a `fleet-gh` crate (`fleet gh` is `pkgs/fleet/crates/fleet/src/gh.rs`).
- **Failing test to start:** `pmo_dsp_02_command_grammar_is_closed_and_typed` in `fleet-queue/src/work/contracts.rs` contract tests (first show it listed and failing).
- **Done/oracle:** golden corpus accepts every canonical command and rejects unknown command, executable syntax, duplicate command, arbitrary option, malformed score, ambiguous target, unsupported alias and merge lacking confirmation. Parser round-trip and property oracle ensures accepted AST is one closed enum verb with bounded typed arguments. Command: `cargo test --manifest-path pkgs/fleet/Cargo.toml -p fleet-queue --lib --features client,queue-broker pmo_dsp_02_command_grammar_is_closed_and_typed`; if a PM ingress adapter has its own parser, also `cd pm-stack/dispatch && bun test test/command-parser.test.ts` against the same fixture corpus.
- **Dependencies:** soft #9200 for priority schema; parser itself may proceed with a numeric candidate while application delegates validation to policy.

### DSP-03 — Event and semantic-action idempotency ledger contract

- **Requirements:** C-8/C-13, H-SURVIVE-A/H-SCENARIOS, O §§14.3/14.7. **Goal:** dedupe delivery retries, comment edits and overlapping source submissions without a new DB.
- **Owned files:** proposed `pkgs/fleet/crates/fleet-queue/src/work/operations.rs` and contract tests, using existing broker storage only after #8581/#8575 ownership review; no new table/DB by assumption. Preserve before/after values in accepted decisions/effects.
- **Failing test to start:** `pmo_dsp_03_delivery_and_semantic_replay_has_one_effect` in `fleet-queue/src/work/operations.rs` tests.
- **Done/oracle:** duplicate delivery ID, same command via a second delivery ID, concurrent submissions, changed digest on same key and timeout replay match reference MBT; exactly one fake effect; receipt persists before ack. Command: `cargo test --manifest-path pkgs/fleet/Cargo.toml -p fleet-queue --lib --features client,queue-broker pmo_dsp_03_delivery_and_semantic_replay_has_one_effect`.
- **Dependencies:** hard DSP-01 and shared operation envelope; soft #8581/#3424/#8579 authority/migration review.

### DSP-04 — Item resolution, event router and policy decision table

- **Requirements:** H-DISTRIBUTED, C-8, O §14.3/14.4/14.6. **Goal:** resolve canonical issue/PR/task and route to status/triage/active-session/direct-start/blocked actions without confounding hierarchy and dependencies.
- **Owned files:** proposed `pkgs/fleet/crates/fleet-queue/src/work/operations.rs` and `relations.rs` contracts plus source adapters, policy decision table and tests; use native parent vs blockedBy metadata from existing GitHub/PM read model.
- **Failing oracle:** ambiguous cross-repo id, PR vs issue mismatch, no task, duplicate tasks, hard/advisory dependencies, and hidden manual-only item each produce a deterministic explainable result.
- **Failing test to start:** `pmo_dsp_04_ambiguous_work_reference_rejects_before_effect` in `fleet-queue/src/work/operations.rs` tests.
- **Done/oracle:** table-driven MBT oracle equals expected target/action/reason; no effect on ambiguous/ineligible rows and no priority mutation on `do-now`. Command: `cargo test --manifest-path pkgs/fleet/Cargo.toml -p fleet-queue --lib --features client,queue-broker pmo_dsp_04_ambiguous_work_reference_rejects_before_effect`.
- **Dependencies:** DSP-01/02/03 and granular resource-read/relations contract keys, #9070, #9178/#9179, #9200. No blocker on the whole #9206 domain starter; queue/table identity reconciliation is hard only for reservation-level writes.

### DSP-05 — Session capability, existing-binding lookup and message delivery receipts

- **Requirements:** C-7/C-8, H-SURVIVE-A, O §14.3/14.5/17. **Goal:** deliver instruction only to the task's authoritative currently-bound session and expose provider-specific capability/ack states.
- **Owned files:** existing OpenCode session adapter, proposed `pkgs/fleet/crates/fleet-queue/src/work/operations.rs` receipt contract, router adapter and fake-provider tests/docs; Muse/Slack only when an existing approved integration is identified.
- **Failing test to start:** `pmo_dsp_05_send_requires_verified_task_binding` in the existing session adapter contract test target.
- **Done/oracle:** verified matching task/session sends once; unbound shell sends never; unsupported adapter says unsupported; Slack delivered is not ack; retry after timeout reconciles. Contract corpus proves capability and receipt distinctions, operation correlation, and redaction. Command: `cargo test --manifest-path pkgs/fleet/Cargo.toml -p fleet-queue --lib --features client,queue-broker pmo_dsp_05_send_requires_verified_task_binding`; fake PM adapter: `cd pm-stack/dispatch && bun test test/session-delivery.test.ts`.
- **Dependencies:** DSP-03/04; consume #9075 and actual provider contracts. No dependency on completing the whole #9206 lifecycle/hydration domain; do not create a new provider adapter/scheduler as part of this slice.

### DSP-06 — Canonical dispatch reservation, direct launch and bind

- **Requirements:** C-2/C-8/C-13, H-SURVIVE-A/H-SCENARIOS, O §§11/14/17/20. **Goal:** reserve once against canonical task, apply #9178/#9179 gates, invoke existing `fleet dispatch launch` / service operation, and bind actual session.
- **Owned files:** `pkgs/fleet/crates/fleet-dispatch` adapter plus proposed `pkgs/fleet/crates/fleet-queue/src/work/operations.rs`/`lifecycle.rs` contract integration, dispatcher tests and operator runbook. Do not patch Supabase and Turso paths independently; route integration of any CLI verb is a separate shared `main.rs`/registry glue slice.
- **Failing test to start:** `pmo_dsp_06_direct_launch_timeout_reconciles_without_duplicate` in the queue/dispatch contract test targets.
- **Done/oracle:** generated crash-cut MBT proves concurrent operation has one task/session; duplicate event reuses IDs; hard gate/cap/lease rejects; timeout after remote create binds existing session after readback without a second launch. Command: `cargo test --manifest-path pkgs/fleet/Cargo.toml -p fleet-queue --lib --features client,queue-broker pmo_dsp_06_direct_launch_timeout_reconciles_without_duplicate`; PM service seam: `cd pm-stack/dispatch && bun test test/direct-launch-reservation.test.ts`. Gated on #9028 and #9178/#9179; no parallel queue/scheduler.
- **Dependencies:** DSP-03/04; #9028, #8581, #9178/#9179; #9241 (local alias `QTI-RECON-01`) gates schema-coupled implementation only.

### DSP-07 — Auditable launch receipt and #6533 acceptance fixture

- **Requirements:** H-SCENARIOS/H-DISTRIBUTED, C-2/C-16/C-17, O §§0.1/14/17. **Goal:** define and verify one complete launch receipt through live handoff/dispatch.
- **Owned files:** shared `Result.effects[]` receipt contract and verifier in existing operation owner; reference #6533 as the existing live fixture (no duplicate issue); no new fixture task/issue.
- **Receipt must include:** operation/dedupe/event/comment refs; authenticated principal/capabilities and auth decision; `(authority_id,work_id)`; external locator and queue ref; source-before/source-after; dispatch one-shot directive distinct from urgency and ratified `expedite=15` band; route/model stamp; lease owner/expiry and generation only where the chosen broker contract provides it; actual session ref; correct worktree; provider prompt-delivery acknowledgement; session binding readback; first meaningful session progress/checkpoint; verification timestamps; terminal or precise blocker. V1 worker-ID claims do not prove secret lease tokens or monotone fences. Every stage points to its read source. Do not require public prompt contents or leak credentials.
- **Failing test to start:** `pmo_dsp_07_receipt_rejects_missing_prompt_ack_or_bind` in existing operation-receipt tests.
- **Oracle:** synthetic success mocks missing prompt ack, bind, or meaningful progress cannot satisfy the fixture; a session ID alone and a queued issue are rejected.
- **Done check:** automated validator rejects each omitted field (`Result` outcome/reason, effect source-before/source-after, task/session binding, prompt delivery and progress proof) with the golden receipt corpus. Command: `cargo test --manifest-path pkgs/fleet/Cargo.toml -p fleet-queue --lib --features client,queue-broker pmo_dsp_07`; first prove `pmo_dsp_07` is listed with `-- --list` because zero-test success is not acceptance. A separately authorized live run against the existing #6533 fixture supplies evidence or records a blocker. This design does not launch production agents; passing unit tests does not close live acceptance.
- **Dependencies:** DSP-05/06; #6533, #8187, #2668. Use #6533's existing acceptance directly; do not create another live-proof issue. Close/accept only through #6533 owner process.

### DSP-08 — Directives, owner authority and reauthorization rules

- **Requirements:** C-3/C-5/C-8, H-SURVIVE-A, O §11.4/14.2/17. **Goal:** one-shot `do-now`, parked-direct-start, severity/action permissions, live-owner rules.
- **Owned files:** operation policy module, roles/permissions docs and tests under existing auth/policy owner.
- **Failing test to start:** `pmo_dsp_08_do_now_never_bypasses_hard_eligibility` in queue policy tests.
- **Done/oracle:** property invariant: no action turns hard-ineligible into eligible; do-now changes directive only; revoked role denied; direct start cannot steal lease/session; merge rechecks head/checks/role. Policy table covers allowed/denied/confirmation/triage and each decision is receipted. Command: `cargo test --manifest-path pkgs/fleet/Cargo.toml -p fleet-queue --lib --features client,queue-broker pmo_dsp_08_do_now_never_bypasses_hard_eligibility`.
- **Dependencies:** #9200, #9178/#9179 and granular eligibility/authz contract keys, DSP-04. `manual_only` remains an eligibility property; `do_now` remains one-shot directive. Do not invent privilege names; reconcile actual Access roles and fleet owner policy first.

### DSP-09 — Recovery, ambiguous remote outcome, race and failure-injection tests

- **Requirements:** C-13/C-16, H-SCENARIOS, O §§14.7/19. **Goal:** prove no duplicate launch and visible/recoverable partial effect under crash/race.
- **Owned files:** integration tests for proposed `fleet-queue/src/work/operations.rs` and existing dispatch/session adapters, failure test doubles and runbook recovery commands. No production data write in test.
- **Failing test to start:** `pmo_dsp_09_crash_cut_never_duplicates_or_falsely_verifies` in the cross-boundary operation test target.
- **Done/oracle:** inject crash at each listed boundary, replay delivery and compare effect graph to model; at most one external launch, no lost accepted request, no false verification. MBT + concurrency race, no arbitrary retry caps. Command: `cargo test --manifest-path pkgs/fleet/Cargo.toml -p fleet-queue --lib --features client,queue-broker pmo_dsp_09_crash_cut_never_duplicates_or_falsely_verifies`.
- **Dependencies:** DSP-03/06/07 and #2668 launch design.

### DSP-10 — Deployable source integration, observability and rollback

- **Requirements:** H-SURVIVE-A/H-SCENARIOS, C-16/C-17, O §§17/20. **Goal:** wire supported entrypoints and dashboards for operator inspection with kill switch and retention/retry policy.
- **Owned files:** existing dispatcher/webhook/orchestration configuration under exact service owner, operator docs/runbook, metrics and deploy smoke tests. Fleet schedule registry updated in same change if a schedule is introduced/changed.
- **Failing test to start:** `pmo_dsp_10_disabled_route_has_zero_effects` in `pm-stack/dispatch/test/event-route-rollback.test.ts`.
- **Done/oracle:** route disabled emits zero effects; stage smoke with signed test event and fake provider reports shared Result outcomes and receipt with secret/prompt canaries absent; rollback disables apply and preserves receipts. Command: `cd pm-stack/dispatch && bun test test/event-route-rollback.test.ts`. No production launch in this slice; the existing #6533 live proof is separately authorized/evidenced, and deploy state separately verified from source merge.
- **Dependencies:** DSP-01 through DSP-07; #9196 safety gates; change/deploy approval from service owner.

## 5. Test strategy and acceptance corpus

Every automated test names an oracle. Use:

- **Parser property tests:** arbitrary bytes/string never panic or produce a verb outside the closed enum; accepted input has exactly one normalized command and typed bounded payload. Oracle: grammar AST validator + golden command corpus.
- **Authorization/eligibility PBT:** for arbitrary operation, actor role, task lease, blocker, manual-only, deadline, capacity and directive, no emitted effect unless all policy predicates pass. Oracle: small pure reference policy model; `do-now` changes neither urgency nor blockers.
- **Event MBT:** generate duplicate, reordered, edited and retried events through state machine; compare receipts and effect count to reference model; every acknowledgement follows durable intake; same operation key has ≤1 irreversible effect.
- **Launch crash model:** fail after each transition. Oracle: expected receipt/task/session graph; at most one live task-bound session; an ambiguous remote result is never represented as verified.
- **Golden receipt corpus:** deliberately omit or mismatch each receipt field; require verifier to reject, especially missing prompt delivery or binding evidence. `#6533`'s live fixture is not satisfied by this corpus alone.
- **Redaction oracle:** receipt serialization contains no bearer/API credential, private note body without visibility permission, or raw sensitive prompt payload. Use canary strings and exact absence assertions.

## 6. Rollout, rollback and open assumptions with defaults

1. **Read-only discovery first:** produce route map and current producer/store matrix, including direct dispatch/PostgREST #9028. No endpoint/schema changes.
2. **Dry-run parser and authorization:** accept only signed test fixtures; return candidate operations without launch/send/Project writes.
3. **Shadow idempotency and receipt states:** replay captured sanitized fixture events against fake adapters, measure no duplicates, retain existing behavior untouched.
4. **Staging with fake provider/task store:** verify all crash boundaries and #6533 receipt validator on the live proof's expected field shape, but do not start a production session from this design pass.
5. **Canary rollout:** only after #9028 is reconciled, role policy approved, #9178/#9179 operations available, and owner authorizes a real live fixture. Per-verb disable switches; direct merge remains disabled until explicit approval.
6. **Rollback:** disable webhook/action apply route; keep signed event and operation receipts for audit/recovery; do not erase outbox/reservation, do not kill active sessions automatically, and route any uncertain remote operation to the owner-run reconciliation path. Existing ordinary queue and #8187 handoff remain unchanged.

Open design assumptions and defaults (no blocking questions):

- **QTI identity still unlocated.** #3424/#8579 are candidates, not aliases. Research is filed at #9241 under #9198 (local alias `QTI-RECON-01`); continue interface design, and gate only schema-dependent reservation/migration slices on its crosswalk and compatibility reviews.
- **Direct launch idempotency support may differ by provider.** Default to reconcile-before-retry; where provider offers idempotency key, send it and still read back. If no reliable reconciliation query exists, do not retry an ambiguous irreversible start automatically.
- **Command aliases unspecified.** Default aliases off until verb-by-verb mapping/authorization test is accepted.
- **Plain-language classifier unspecified.** Default classify-to-human-triage only; never direct side effect from low-confidence/unstructured text.
- **Session/provider ack semantics vary.** Default expose unsupported or delivery-only, not synthetic acknowledgment.
- **FDR IDs:** #9180 reserves FDR-0001…0010. Proposed `FDR-PMO-001-*` naming must be reconciled with the canonical global index before allocation; filing a stub is not ratification.
