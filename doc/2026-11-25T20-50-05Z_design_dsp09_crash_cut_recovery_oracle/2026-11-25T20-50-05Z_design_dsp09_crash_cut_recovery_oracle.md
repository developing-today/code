# DSP-09 — Recovery, ambiguous remote outcome, race and failure-injection oracle

Issue: [#9449](https://github.com/developing-today/code/issues/9449)
(slice `DSP-09` of [#9203](https://github.com/developing-today/code/issues/9203),
program [#9196](https://github.com/developing-today/code/issues/9196)).
Status: **design proposal — not an authorization to implement.**
This record elaborates the DSP-09 row of the merged design package
(`doc/2026-11-13T03-20-24Z_design_pm-stack-fleet-orchestration/`,
PR #9225) into the test design a separately scoped implementation
issue can be filed against. The component row says: "inject crash at
each listed boundary, replay delivery and compare effect graph to
model; at most one external launch, no lost accepted request, no
false verification. MBT + concurrency race, no arbitrary retry caps."
It never lists the boundaries, never defines the effect graph and
never says what "recoverable" has to look like. This record
_decides_ those three things, names the failure doubles and the
generated sequences, and records where the launch pipeline on main
today would fail the oracle. It authorizes no code, schema,
deployment, production session or live acceptance; those remain
separately gated stages (see §9).

Governing decisions consumed, not re-opened:

- **Component #9203 §3.4–§3.5**
  (`components/9203-direct-dispatch-slash-commands-event-routing.md:73-97`):
  the receipt stage list
  `received → verified → authorized → resolved → reserved → launching → session_created → task_bound → prompt_delivered → acknowledged → progressing → verified_terminal`
  is effect stages, not a second outcome enum; "ambiguous timeout is
  `uncertain`"; "If launch times out after remote creation, return a
  resumable receipt as `uncertain` and reconcile provider by request
  metadata/work identity"; "Never create a second live session to
  'fix' a missing response. Orphan session is surfaced as an
  unresolved binding … not automatically stopped."
- **Component #9203 §3.7 fail-safe matrix** (`:107-120`) and **§5
  launch crash model** (`:216`): "fail after each transition. Oracle:
  expected receipt/task/session graph; at most one live task-bound
  session; an ambiguous remote result is never represented as
  verified."
- **REL-03 record** (#9513,
  `doc/2026-11-25T20-47-34Z_design_rel03_idempotency_identity_receipts/`,
  merged in #15240): the scope tuple, replay versus conflict, and
  "a replay writes nothing". DSP-09's source slice names DSP-03; the
  issue normalizes that alias to REL-03.
- **DSP-05 record**
  (`doc/2026-11-25T19-35-00Z_design_dsp05_session_capability_delivery_receipts/`):
  the delivery evidence ladder (§3.3), `unknown` "is never read as
  'not sent'" (§3.4), reconcile-before-retry by correlation marker
  (§5.4), and its failure-mode table (§9).
- **#2668** (closed design, pm-dispatch launch pipeline): "the claim
  PATCH is the launch's commit point. After it, steps are
  annotations"; annotation failure never cancels a live session's
  task.
- The issue's negatives: stale input, duplicate/replay, ambiguous
  resource, unauthorized actor, partial remote effect, unknown
  version, and no false verified/completed result.

## 1. Intent

Every way a dispatch can be cut short (a crash, a lost response, a
race with another worker, a revoked grant) ends in one of two places:
the work finished exactly once with evidence, or the work is visibly
stopped in a state an operator can resume from without guessing. No
cut ever produces a second agent session for one request, silently
drops a request Fleet already accepted, or reports a launch as
verified on evidence that does not show it. DSP-09 is the test that
proves this, and the model that says what "this" is.

## 2. Ownership

| Concern                                                                                                              | Owner                                                                                             |
| -------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------- |
| Reference effect-graph model, boundary list, generators, invariants, failure doubles, recovery runbook (this record) | **DSP-09**: tests for `fleet-queue/src/work/operations.rs` and the dispatch/session adapter seams |
| Operation identity, replay and conflict                                                                              | REL-03 (record merged)                                                                            |
| Reservation, direct launch and bind; the production fixes for §6's gaps                                              | DSP-06 (#9506)                                                                                    |
| Message send, delivery ladder, send reconciliation                                                                   | DSP-05 (record merged)                                                                            |
| Receipt field verifier and the #6533 live fixture                                                                    | DSP-07 (#9447)                                                                                    |
| Remote read-back of GitHub effects                                                                                   | REL-06 (#9516)                                                                                    |
| `completion.rs`                                                                                                      | read-only consumer here                                                                           |

DSP-09 writes no production code path. Its doubles never touch a
real provider, queue, GitHub repository or worktree root, and no test
writes production data.

## 3. The effect graph

### 3.1 Nodes

The model tracks every object a dispatch can create or change,
inside Fleet and outside it.

| Node             | Where it lives                                      | Created at stage                  |
| ---------------- | --------------------------------------------------- | --------------------------------- |
| `event`          | webhook intake (delivery id, comment version)       | `received`                        |
| `operation`      | REL-03 ledger: scope tuple, digest, receipt         | `authorized` (durable acceptance) |
| `task`           | queue broker row (dedupe key, status, session id)   | `reserved`                        |
| `issue`          | GitHub, when the dispatch creates one               | before `reserved`                 |
| `worktree`       | dispatch worktree root + registry record            | `launching`                       |
| `session`        | provider (OpenCode) session                         | `session_created`                 |
| `binding`        | task ↔ session in the queue authority               | `task_bound`                      |
| `prompt`         | message inside the session, with correlation marker | `prompt_delivered`                |
| `annotation`     | issue comment and labels after the commit point     | after `task_bound`                |
| `receipt stages` | per-stage evidence with its read source             | every stage                       |

Each external node also carries **who knows about it**: `confirmed`
(Fleet holds the id from a successful response or a read-back),
`unknown` (a request left and its answer was lost), or `absent`.

### 3.2 Invariants

The oracle checks all of these after every generated step, not only
at the end.

| ID  | Invariant                                                                                                                                                                                                                                     |
| --- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| I1  | **At most one external launch per operation.** The provider holds at most one session created for a given `operation_id`, and the queue holds at most one live task-bound session per task.                                                   |
| I2  | **No lost accepted request.** Every operation that reached durable acceptance is, at every point, either terminal with a receipt or in a resumable state that an inspection read returns. A restart never forgets it.                         |
| I3  | **No false verification.** A stage is `verified` only when its evidence came from that stage's named read source after the effect. An `unknown` external node never yields `verified_complete` or `verified_terminal`; it yields `uncertain`. |
| I4  | **Replay is free.** Redelivering the same event, or retrying with the same key and digest, after any cut, creates no node and changes no stage (REL-03 §5.3).                                                                                 |
| I5  | **No blind compensation.** Fleet never cancels a task, deletes a session or removes a worktree whose state is `unknown`; it reads back first. A confirmed effect is never undone to "clean up" an ambiguity.                                  |
| I6  | **Nothing invisible.** Every external node that exists in the doubles at the end of a run is either bound to the operation, compensated with recorded evidence, or listed in the receipt as an orphan.                                        |
| I7  | **Acknowledge after durable intake.** The webhook delivery is acknowledged only after the `event` and, when accepted, the `operation` are durable.                                                                                            |
| I8  | **Authority is current.** A grant revoked, a binding changed or a work version advanced between two stages stops the operation before its next external effect, with `rejected` or `conflict`.                                                |
| I9  | **Partial is visible.** When some effects landed and others did not, the receipt says `partial` and names each landed effect; it never says `rejected` while an effect exists.                                                                |

## 4. Boundaries

### 4.1 The cut list

Each boundary is cut three ways where an external call is involved:
**before** the call leaves, **apply-then-lose** (the remote side
applied the effect and the response was lost), and **after** the
response was received but before Fleet recorded it. Internal-only
boundaries are cut before and after the local commit.

| ID  | Boundary (stage)                                            | Cuts                                 | Required outcome after restart and redelivery                                                                                                                                                                                                                                              |
| --- | ----------------------------------------------------------- | ------------------------------------ | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| B1  | event intake (`received`)                                   | before / after commit                | before: provider redelivers, processed once. After: redelivery is a duplicate; I7 holds.                                                                                                                                                                                                   |
| B2  | parse, authorize, resolve (`verified`→`resolved`)           | before / after                       | nothing durable before acceptance; a refusal records nothing (DSP-01); ambiguous resource blocks with candidates.                                                                                                                                                                          |
| B3  | operation acceptance (`authorized`)                         | before / after commit                | before: redelivery accepts once. After: redelivery replays (I4).                                                                                                                                                                                                                           |
| B4  | issue create, when the dispatch creates one                 | before / apply-then-lose / after     | apply-then-lose: recovery finds the issue by the operation's marker before creating; never two issues for one operation.                                                                                                                                                                   |
| B5  | task enqueue / reservation (`reserved`)                     | before / apply-then-lose / after     | the queue dedupes on the operation's dedupe key; apply-then-lose returns the existing row, never a second task.                                                                                                                                                                            |
| B6  | worktree create + registry record (`launching`)             | before / after create / after record | a tree with no record is found by path and recorded, or reported as an orphan; never silently leaked.                                                                                                                                                                                      |
| B7  | session create (`session_created`)                          | before / apply-then-lose / after     | apply-then-lose: the session id is `unknown`; recovery lists provider sessions by the operation's title/marker and directory; found → bind; not found after a complete read → one create under the same operation; not readable → `uncertain`. Never a second create while `unknown` (I1). |
| B8  | prompt send (`prompt_delivered`)                            | before / apply-then-lose / after     | DSP-05 §5.4: `unknown` until the correlation marker is read back; never resent without that read.                                                                                                                                                                                          |
| B9  | claim and bind (`task_bound`, the commit point)             | before / apply-then-lose / after     | apply-then-lose: read the task by id; claimed for this session → bound, continue; claimed by another worker → `conflict`, this session is an orphan reported under I6; still queued → claim once more. Never cancel without the read (I5).                                                 |
| B10 | annotations (comment, labels)                               | before / apply-then-lose / after     | best-effort after the commit point (#2668); a lost annotation is a warning, never a cancel; a retried comment finds the first by its marker before posting (REL-06).                                                                                                                       |
| B11 | acknowledgment and progress (`acknowledged`, `progressing`) | wait expires                         | `pending_sync` with the levels reached; never promoted to `acknowledged` without its evidence (DSP-05 §3.3).                                                                                                                                                                               |
| B12 | final verification (`verified_terminal`)                    | before / after                       | `verified_terminal` only with read-back of task, binding and session; otherwise the last proven stage stands.                                                                                                                                                                              |

### 4.2 Races

Generated alongside the cuts, not instead of them:

- two deliveries of one event processed by two workers at once
  (I1, I4);
- two different operations targeting the same task (one reservation
  wins; the other is `conflict` or routed to the existing binding);
- a fleet worker claiming the task between B5 and B9 (the launch backs
  out as a lost claim race, the session it created is reported under
  I6);
- a grant revoked, or the comment edited, between any two stages
  (I8; an edit is a new candidate and conflicts under REL-03);
- the work version advanced by another writer between `resolved` and
  `reserved` (stale input, `conflict`).

### 4.3 No arbitrary retry caps

The model never terminates because a counter ran out. Every
recovery step is a read that settles the node (`confirmed` or
`absent`) or leaves it `unknown`, and an `unknown` node blocks the
next external effect instead of being retried. So the generated
sequences terminate by their own length, and the invariants hold for
any number of restarts and redeliveries. A provider retry budget, if
one is ever configured, is REL-05's policy and is not a correctness
input here.

## 5. Failure doubles and generators

- **Fake provider** (session API): modes `ok`, `refuse`,
  `apply_then_timeout`, `timeout_without_apply`, `5xx_after_body`,
  and `eventually_visible` (a created session that a list call shows
  only after N reads). It exposes create, get, list, send, list
  messages and delete, so recovery reads exist in the double exactly
  where they exist in the provider.
- **Fake queue broker**: enqueue with dedupe, claim-by-id with
  `ClaimLost`, claim `apply_then_timeout`, get-by-id, find-by-dedupe-key
  and find-by-session-id (the reads `pm-stack/dispatch/src/queue.ts`
  already uses).
- **Fake GitHub**: issue create, comment, labels, each with the same
  modes, plus a marker search.
- **Fake worktree root**: create, registry add, registry lookup, with
  a crash between create and record.
- **Deterministic clock and scheduler**: the generator chooses the
  interleaving; nothing depends on wall time.
- **Generator**: sequences over
  `deliver | redeliver | edit | revoke | crash@Bk(cut) | restart | reconcile | competing_operation | worker_claims_task | advance_version`,
  with every B1–B12 cut reached at least once per run. Failing seeds
  are kept as fixed regression cases.
- **Model**: a pure reference of §3 that predicts nodes and stages for
  the same sequence. The test compares predicted and observed graphs
  node by node, then checks I1–I9.

## 6. Where the pipeline on main stands against the oracle

Source facts from `pm-stack/dispatch/src` at `origin/main` `1c6c75384`,
not deployment proof. The pipeline is: optional issue create, enqueue,
worktree create and record, `createSession`, `sendPrompt`, then
`claimTask` (claim-by-id, lane ledger and session bind in one broker
trip), then guarded annotations. These are the cases the generated
sweep would report today. They are inputs to DSP-06, not fixes made
here.

| Gap | Boundary | What the code does                                                                                                                                                                                                                                                                                                                                                                                                                                                                          | Invariant |
| --- | -------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | --------- |
| G1  | B7       | `withCreatedSession` records the session id only after `createSession` returns (`server.ts:328-344`, `launch_failure.ts`). If the create is applied and the response lost, the failure path cancels the task with no session in hand, so the note names no session and nothing searches for it.                                                                                                                                                                                             | I5, I6    |
| G2  | B9       | `claimTask` runs inside `withCreatedSession`; any error other than `ClaimLostError` reaches `handleLaunchFailure`, which cancels the task and deletes the session (`server.ts:489-505`). A claim applied by the broker whose response was lost is therefore undone without a read-back.                                                                                                                                                                                                     | I5        |
| G3  | B3–B5    | `POST /v1/sessions` carries no request key. A client that retries after a lost `201` gets a new launch: a newly created issue yields a fresh canonical dedupe key, and with no issue the key is random (`packet.ts:17-34`), so the broker enqueues a second task and a second session is created. `POST /v1/tasks` does hash a caller-supplied key into its dedupe key when attaching to an existing issue (`packet.ts:38-52`, `server.ts:679`); the session launch path has no equivalent. | I1, I4    |
| G4  | B3       | The DSP-01 acceptance ledger is an in-memory model (`work/operations.rs:270-329`); its persistence is not on main, so a restart after acceptance would forget it.                                                                                                                                                                                                                                                                                                                           | I2        |

What already holds: a lost claim race (`ClaimLostError`) backs out and
answers `409` rather than `502` (`server.ts:503-505`); annotations
after the claim are individually guarded and never cancel the launch
(`server.ts:362-377`, #2668 phase 2); the enqueue dedupes on its key
against all tasks (`queue.ts:178-187`).

## 7. Recovery runbook

For each `uncertain` node, the read that settles it, using surfaces
that exist today. Commands allocated to this slice
(`agent.session.resume`, `agent.session.send`, `agent.session.stop`,
`agent.slash.retry`) do not exist yet; until CLI-13 registers them,
the runbook names the read, not a command.

| Unknown node    | Settle by reading                                                                                       | If found                                     | If absent after a complete read      | If unreadable               |
| --------------- | ------------------------------------------------------------------------------------------------------- | -------------------------------------------- | ------------------------------------ | --------------------------- |
| issue           | GitHub search for the operation marker in the dispatch repo                                             | adopt it                                     | create once under the same operation | stay `uncertain`            |
| task            | broker find-by-dedupe-key                                                                               | adopt the row                                | enqueue once                         | stay `uncertain`            |
| worktree        | worktree registry, then the path under the root                                                         | record it                                    | nothing to do                        | report as retained          |
| session         | provider session list filtered by directory and the operation's title/marker; broker find-by-session-id | bind if the task is free; else report orphan | create once under the same operation | stay `uncertain`; no create |
| prompt          | provider messages for the correlation marker (DSP-05 §5.3)                                              | `delivered`                                  | send once under the same operation   | stay `uncertain`; no send   |
| binding (claim) | broker get-by-id                                                                                        | continue from `task_bound`                   | claim once                           | stay `uncertain`; no cancel |

An orphan session is reported in the receipt and left running for its
owner's recovery rules (component `:95`). Stopping it is an explicit,
authorized `agent.session.stop`, never an automatic step.

## 8. Acceptance oracle

Rust tests live beside `fleet-queue` `src/work/operations.rs`; the
adapter seam tests live in `pm-stack/dispatch/test/`. Every Rust test
is first listed by exact name with `-- --list`; a filter matching
zero tests is not acceptance.

| Test                                                                | Asserts                                                                                                    |
| ------------------------------------------------------------------- | ---------------------------------------------------------------------------------------------------------- |
| `pmo_dsp_09_crash_cut_never_duplicates_or_falsely_verifies`         | the generated sweep: every B1–B12 cut and every §4.2 race, graph equal to the model, I1–I9 after each step |
| `pmo_dsp_09_session_create_lost_response_reconciles_not_relaunches` | B7 apply-then-lose: one session, bound after read-back; `unknown` blocks a second create (G1)              |
| `pmo_dsp_09_claim_ack_lost_never_cancels_bound_task`                | B9 apply-then-lose: task read back as claimed for this session; no cancel, no delete (G2)                  |
| `pmo_dsp_09_prompt_send_unknown_never_resent_without_readback`      | B8: `unknown` until marker read; resend only after a complete negative read, under the same operation      |
| `pmo_dsp_09_client_retry_after_lost_201_reuses_operation`           | B3–B5: same key and digest after a lost response → the stored receipt, no new issue, task or session (G3)  |
| `pmo_dsp_09_restart_after_acceptance_keeps_operation`               | B3 after commit, then restart: the operation is still listed and resumable (G4)                            |
| `pmo_dsp_09_concurrent_operations_one_task_session`                 | two operations on one task: one reservation, the other `conflict` or routed; one live bound session        |
| `pmo_dsp_09_revoked_between_stages_stops_before_next_effect`        | revoke between each pair of stages → `rejected` before the next external call                              |
| `pmo_dsp_09_orphan_session_reported_not_stopped`                    | lost claim race after B7: session listed as orphan in the receipt; the double shows no delete              |
| `pmo_dsp_09_ack_follows_durable_intake`                             | B1: no acknowledgment observed before the event and operation are durable                                  |
| `pmo_dsp_09_partial_effect_visible`                                 | annotations fail after B9 → receipt names the landed stages; outcome never `rejected`                      |
| `pmo_dsp_09_stale_version_and_unknown_schema_rejected`              | advanced work version → `conflict`; unknown command major version → `rejected`; nothing written            |
| `pmo_dsp_09_ambiguous_target_blocks_with_candidates`                | two candidate tasks → blocked with candidates; no reservation                                              |
| `pmo_dsp_09_terminates_without_retry_cap`                           | the sweep run with no retry limit configured terminates and passes                                         |
| `test/launch-crash-cut.test.ts` (bun, `pm-stack/dispatch`)          | B4–B10 against the real `launchSession`/`handleLaunchFailure` with fake provider, broker and GitHub        |

The issue's done command:
`cargo test --manifest-path pkgs/fleet/Cargo.toml -p fleet-queue --lib --features client,queue-broker pmo_dsp_09_crash_cut_never_duplicates_or_falsely_verifies`.
The seam test: `cd pm-stack/dispatch && bun test test/launch-crash-cut.test.ts`.
Passing either is model and seam evidence only. It is not the #6533
live acceptance and launches no production session.

Code spelling to reconcile, not decided here: the DSP-05 record names
the replay conflict `idempotency_conflict`, while the REL-03 record
uses the component #9209 envelope code `IDEMPOTENCY_KEY_REUSED`. The
oracle asserts the shared outcome `conflict` and leaves the code
spelling to REL-03, which owns identity.

## 9. Gates and sequencing

- **Hard blocker named by the issue:** REL-03 (#9513). Its record is
  merged (#15240); its implementation is still gated. DSP-09's model
  tests can be written against the REL-03 contract before that lands,
  but the sweep cannot pass until the ledger replays and conflicts as
  REL-03 §4 says.
- **Source dependencies:** DSP-06 (#9506, open, no record) owns the
  fixes for G1–G3 and the reservation the sweep exercises; DSP-07
  (#9447, open, no record) owns the receipt verifier the I3 check
  calls. Until each lands, the matching tests are written as expected
  failures with the gap ID in their name or message, never skipped
  and never passed by weakening the model.
- **File collision:** `operations.rs` is shared with DSP-01, DSP-04,
  DSP-08 and REL-03; DSP-09 adds tests only and sequences after them.
- **#2668** is consumed as evidence (closed), not as a gate.
- **Rollback:** the slice adds tests and doubles only; removing them
  changes no runtime behavior.

## 10. Requirement trace (design-level)

Full per-occurrence text for every allocated ID is in
`doc/2026-11-13T03-20-24Z_design_pm-stack-fleet-orchestration/census/requirements-crosswalk.jsonl`;
this records which part of this design speaks to each group. It is
design coverage, not implementation.

| IDs                                                            | Theme                                                                       | Where addressed                                                        |
| -------------------------------------------------------------- | --------------------------------------------------------------------------- | ---------------------------------------------------------------------- |
| H-R016                                                         | crash after remote mutation, duplicate delivery scenarios                   | §4, §8                                                                 |
| H-R021, H-R038, H-R039, H-R051, H-R052                         | verify created ids and durable task records; pending intent is not evidence | I3, B4–B9, §7                                                          |
| H-R040, H-R053                                                 | actual route and host, no invented provider                                 | B7 reads the real provider's list; doubles mirror its reads only       |
| C-R087, C-R152                                                 | authenticated slash commands, live comment routing                          | B1–B3, I7, I8                                                          |
| C-R094, O-R046                                                 | do-now is a one-shot directive, not a gate bypass                           | §4.2 revoke and stale-version races; directive semantics stay DSP-08's |
| C-R005, C-R009, C-R107, H-R017, H-R024, H-R031, O-R023, P-R009 | cohesive CLI, preserved inventory, structured receipts and failures         | §7 allocated commands, §8 receipt assertions                           |

## 11. Non-goals

- No fix to `pm-stack/dispatch` or the broker; §6 is input to DSP-06.
- No receipt field list (DSP-07), send semantics (DSP-05), identity
  rules (REL-03) or directive policy (DSP-08).
- No production session, real provider call, GitHub write or #6533
  live run.
- No new queue, database, scheduler, service, executable or GitHub
  client, and no retry budget.
