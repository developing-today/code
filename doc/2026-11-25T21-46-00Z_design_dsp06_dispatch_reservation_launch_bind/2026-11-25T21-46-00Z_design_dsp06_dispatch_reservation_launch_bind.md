# DSP-06 — Canonical dispatch reservation, direct launch and bind

Issue: [#9506](https://github.com/developing-today/code/issues/9506)
(slice `DSP-06` of [#9203](https://github.com/developing-today/code/issues/9203),
program [#9196](https://github.com/developing-today/code/issues/9196)).
Status: **design proposal — not an authorization to implement.**
This record elaborates the DSP-06 row of the merged design package
(`doc/2026-11-13T03-20-24Z_design_pm-stack-fleet-orchestration/`,
PR #9225). The row says: "reserve once against canonical task, apply
#9178/#9179 gates, invoke existing `fleet dispatch launch` / service
operation, and bind actual session." It never says what "reserve"
is in the existing queue, in which order the launch steps run, what
is undone when a step fails, or how a retry is told apart from a new
request. This record _decides_ those things against the code on main
and against the four launch-path gaps the DSP-09 record handed to
this slice. It authorizes no code, schema, deployment, production
session or live acceptance; those remain separately gated stages
(see §9).

Governing decisions consumed, not re-opened:

- **Component #9203 §3.4–§3.6**
  (`components/9203-direct-dispatch-slash-commands-event-routing.md`):
  direct dispatch reserves "using the existing queue claim/bind
  protocol and a stable operation key", rechecks eligibility at the
  atomic claim, and "never sends a blind second start"; the receipt
  stages `received → … → verified_terminal` are effect stages; the
  outcomes are `rejected`, `conflict`, `partial`, `uncertain`,
  `pending_sync`, `verified_complete`; an orphan session is surfaced
  and never stopped automatically; do-now does not bypass gates, and
  `manual_only` and park follow #9178/#9179.
- **DSP-01** (`pkgs/fleet/crates/fleet-queue/src/work/operations.rs`):
  `Principal`, `Verb`, `ResourceRef`, `Grant`, `Command`, `Receipt`,
  `Effect`, `AcceptOutcome` and the in-memory `AcceptanceLedger`. A
  refusal records nothing.
- **REL-03 record**
  (`doc/2026-11-25T20-47-34Z_design_rel03_idempotency_identity_receipts/`):
  the scope tuple `(principal_ref, authority_id, idempotency_key)`,
  the `fleet.command-intent/v1` digest, the key format (1–200
  printable ASCII; `ghcmd:` reserved), the lookup order accept /
  replay / conflict / legacy, "a replay writes nothing", and "a
  timeout after a side effect is a replay that returns `uncertain`".
- **DSP-05 record**: the existing-binding lookup (§4), the send and
  its correlation marker (§5.3), and "`unknown` is never read as not
  sent" (§5.4).
- **DSP-09 record**
  (`doc/2026-11-25T20-50-05Z_design_dsp09_crash_cut_recovery_oracle/`):
  invariants I1–I9, cuts B1–B12, and gaps G1–G4 on main. DSP-06 is
  judged by that oracle; it does not restate it.
- **Program ruling, 2026-11-24, recorded on #9196**: no pretend
  safety gates on a design with no hard blocker. This slice has hard
  blockers (REL-03, #8581); they are named in §9 and nothing else is
  added.

Verified state on `origin/main` `2f4462b6d` (source facts, not
deployment proof):

- `pm-stack/dispatch/src/server.ts` `handleSessions` /
  `launchSession`: optional issue create → enqueue with
  `dedupeKey = canonicalDedupeKey(kind, repo, issue)` or
  `dispatchDedupeKey(repo)` (random when there is no issue) →
  placement guard → worktree create and registry record →
  `opencode.createSession` → `withCreatedSession(sendPrompt;
claimTask(task, session.id, caller, LEASE_SECONDS))` → guarded
  annotations. `wait=false` answers `202` and launches
  asynchronously. A failure reaches `handleLaunchFailure`, which
  cancels the task, deletes the session and retires the worktree;
  `ClaimLostError` answers `409`.
- `pm-stack/dispatch/src/queue.ts` `claimTask` POSTs `/claim-flow`
  (claim, lane, bind in one broker trip). When the response is lost,
  `reconcileClaimFlow` reads the task, checks that the claim belongs
  to this session, reapplies lane and `/bind`, and re-reads. When
  that read itself fails, the original error is rethrown and the
  launch-failure path cancels. So **G2 is narrowed on main**: an
  applied-then-lost claim is now read back; only the unreadable case
  still reaches a cancel.
- The broker already offers a by-id claim (`claim --task <id>`) that
  needs no session, a separate `/bind`, and `release` with a
  required reason (`pkgs/fleet/book/reference/queue-core.md`).
- `pkgs/fleet/crates/fleet-dispatch/src/lib.rs` is a thin client
  over pm-dispatch (`GET /health`, `POST /v1/sessions`,
  `POST /v1/tasks`). `launch` takes `repo`, `prompt`/`prompt-file`,
  `title`, `model`, `directory`, `priority`, `issue`/`no-issue` and
  `wait`/`no-wait`; it sends no request key. Exit codes: 0 ok,
  2 usage, 3 transport, 4 HTTP error.
- #9028 is closed: pm-dispatch reaches the queue only through the
  broker, so there is one producer to design against.

## 1. Intent

One accepted dispatch operation yields at most one canonical task,
at most one provider session and at most one prompt delivery, and
its receipt names exactly what exists. "Reserve" means the existing
queue claim, taken before anything external is created, so no fleet
worker can claim the task between enqueue and launch and no second
launch can start for the same operation. The launch goes through the
one existing dispatch path; DSP-06 adds no queue, store, scheduler,
service, resolver, executable or GitHub client.

## 2. Ownership

| Concern                                                                   | Owner                                                  | DSP-06 role                                       |
| ------------------------------------------------------------------------- | ------------------------------------------------------ | ------------------------------------------------- |
| Launch-step decision model, reservation contract, outcome mapping         | `pkgs/fleet/crates/fleet-queue/src/work/operations.rs` | writes (shared file; DSP-05 lands first)          |
| Client request key, `--idempotency-key`, transport-unknown next action    | `pkgs/fleet/crates/fleet-dispatch/src/lib.rs`          | writes                                            |
| Work contracts and lifecycle states                                       | `work/contracts.rs`, `work/lifecycle.rs`               | reads only                                        |
| Acceptance persistence, key scope, digest                                 | REL-03 / REL-04                                        | consumes                                          |
| Send, correlation marker, existing-binding lookup                         | DSP-05                                                 | consumes                                          |
| Crash-cut oracle, doubles, generators                                     | DSP-09                                                 | is judged by                                      |
| Claim, `/bind`, `release`, cancel, dedupe-key enqueue                     | queue broker (`fleet-queue`)                           | uses as is                                        |
| Service launch path (`/v1/sessions`)                                      | `pm-stack/dispatch`                                    | changes specified here, implemented by a later PR |
| `manual_only`, takeover, resume; park and resume                          | #9178, #9179                                           | consumes semantics; runtime gates                 |
| CLI registration of `agent.dispatch`, `agent.slash.start`, `work.handoff` | CLI-13                                                 | supplies semantics only                           |
| Live acceptance of the dispatch path                                      | #6533 / DSP-07 (#9447)                                 | not duplicated                                    |

## 3. Acceptance before any effect

1. The request is accepted through REL-03 before any external call,
   keyed by `(principal_ref, authority_id, idempotency_key)` with the
   `fleet.command-intent/v1` digest over the normalized launch
   intent (repo, issue or none, prompt digest, model, directory,
   priority, wait).
2. **Replay** (same key, same digest) returns the stored receipt
   with its current stage and writes nothing. It never launches
   again, whatever stage the first attempt reached.
3. **Conflict** (same key, different digest) is `conflict` with the
   digest mismatch named; nothing is created.
4. **Legacy** (no key): the launch still runs, and its receipt says
   `idempotency: unsupported` so no caller reads replay safety into
   it. A retry without a key is a new request.
5. The accepted operation carries `operation_id`, which every later
   marker in §5 uses.

**Until REL-03/REL-04 persistence lands (G4)**, acceptance is the
in-memory DSP-01 ledger. The service then answers every launch with
`idempotency: unsupported`, even when a key was sent, and claims no
replay safety across a restart. This is the honest interim, not a
fallback.

## 4. Existing binding and gates, at reservation time

The checks run in this order; the first that decides ends the
operation with its receipt. All are re-read at the atomic claim in
§5 step 3, never trusted from an earlier read.

| Order | Check                                                         | Outcome                                                                                                                                                           |
| ----- | ------------------------------------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| 1     | A live, capable session already bound to the task (DSP-05 §4) | route to a send on that session (DSP-05), no launch. A bound session that cannot take the instruction is reported with the reason; it is never replaced silently. |
| 2     | Hard dependency open                                          | `rejected`, reason `ineligible_blocker`, blockers named.                                                                                                          |
| 3     | `manual_only` (#9178)                                         | allowed only through the explicit manual-claim mode the request names; the flag stays on the task. Without that mode: `rejected`, reason `manual_only`.           |
| 4     | Parked (#9179)                                                | an explicit, authorized resume only; ordinary dispatch is `rejected`, reason `parked`. A resume claim does not consume an attempt (broker behaviour, kept).       |
| 5     | Another live lease or a different bound session               | `conflict`, holder named. Never stolen; takeover is #9178's explicit verb, not a dispatch side effect.                                                            |
| 6     | Capacity or bundle cap                                        | the owner's policy decides queue-and-wait or refuse; the receipt names the cap and the decision. Never a silent downgrade to ordinary queue work (O-R065).        |
| 7     | `do_now`                                                      | consumed once by this operation; a replay does not consume it again. It removes waiting only (H-R006), never a gate above.                                        |

## 5. The launch sequence

Each step names its stage (DSP-09 §3.1) and what it writes. The
commit point moves from "claim after prompt" (main) to "claim
first": the reservation exists before any external node.

| Step | Stage              | Action                                                                                                                                                                                                                                                                        |
| ---- | ------------------ | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| 1    | `resolved`         | Issue: use the named issue; or, when the dispatch creates one, search for the operation marker first and create only when the search read is complete and empty (B4).                                                                                                         |
| 2    | `reserved`         | Task: an existing canonical task for the issue is used as is; a task this operation creates is enqueued with dedupe key `dispatch-op:<operation_id>` (closes G3: a retry under the same accepted operation finds the same row; the random key is retired for keyed launches). |
| 3    | `reserved`         | **Reservation = by-id claim** with the dispatch worker identity, no session, and a launch lease. The claim's conditional update re-checks claimability (§4 rows 2–5). Lost claim race → `conflict`, nothing external exists yet.                                              |
| 4    | `launching`        | Worktree create and registry record (B6).                                                                                                                                                                                                                                     |
| 5    | `session_created`  | `createSession` with title and directory carrying `operation_id` as the marker the recovery read searches for (B7).                                                                                                                                                           |
| 6    | `task_bound`       | `/bind` the claimed task to the actual session id returned by step 5, under the same worker and lease.                                                                                                                                                                        |
| 7    | `prompt_delivered` | Send through DSP-05 with its correlation marker. `unknown` stays `unknown` until the marker is read back (DSP-05 §5.4).                                                                                                                                                       |
| 8    | annotations        | Comment and labels after the commit point, best effort (#2668); a retried comment finds the first by its marker (REL-06).                                                                                                                                                     |
| 9    | `acknowledged`…    | DSP-05 acknowledgement levels; an expired wait is `pending_sync` with the level reached (B11).                                                                                                                                                                                |

`wait=false` answers `202` after step 3 with the receipt at
`reserved` and the inspection handle; it never answers before the
reservation is durable.

**Gate on the design itself:** the implementation must show, against
the broker's own tests, that a by-id claim with no session followed
by a separate `/bind` under the same worker is accepted and leaves
the same row state `/claim-flow` leaves today. If it is not, the
fallback is to keep `/claim-flow` at step 6 and run step 3 as a
lease-only reservation on the existing queue; a new reservation store
is not an option.

## 6. Failure and compensation

The rule is DSP-09 I5 applied per step: undo only what is known to
exist and known to be this operation's; read before acting on
anything `unknown`.

| Where it fails                          | Known state                  | Action                                                                                                                                                                                               | Outcome                       |
| --------------------------------------- | ---------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ----------------------------- |
| Steps 1–3, definite                     | no session, no worktree      | release the reservation; a task this operation created is cancelled with the reason (as #2901 does); an existing task is released back to `queued` with the reason.                                  | `rejected` or `conflict`      |
| Step 4, definite                        | reservation held             | as above; a created-but-unrecorded tree is recorded or reported as an orphan (B6).                                                                                                                   | `rejected`, orphan named      |
| Step 5, definite failure                | no session                   | release as above, retire the worktree.                                                                                                                                                               | `rejected`                    |
| Step 5, `unknown` (G1)                  | session may exist            | hold the reservation; list provider sessions by marker and directory. Found → continue at step 6. Complete read, none → one create under the same operation. Unreadable → stop.                      | `uncertain` until resolved    |
| Step 6, definite failure                | confirmed session, no prompt | the session may be deleted, with the evidence (session id, bind error) recorded; then release.                                                                                                       | `rejected`, evidence recorded |
| Step 6, `unknown`                       | bind may have landed         | read the task: bound to this session → continue; bound elsewhere → `conflict`, this session reported as an orphan (I6), never deleted; unreadable → stop. This is `reconcileClaimFlow`'s read, kept. | `uncertain` until resolved    |
| Step 7, `unknown`                       | prompt may be delivered      | DSP-05 marker read-back; never resent without it.                                                                                                                                                    | `uncertain` until resolved    |
| Reservation lost (lease expired, taken) | confirmed session            | report the session as an orphan with the holder named; never deleted automatically (#9203 §3.5).                                                                                                     | `partial`                     |

In every `unknown` row the task is **never** cancelled and the
session **never** deleted (I1, I5). This removes the remaining G2
case on main, where an unreadable claim result still reaches
`handleLaunchFailure`. The receipt in an `uncertain` state carries
the inspection handle and the exact read that would resolve it.

## 7. The client: `fleet dispatch launch`

- Mints an idempotency key when the caller gives none, and accepts
  `--idempotency-key <key>` (REL-03 format) to reuse one.
- Sends it as the `Idempotency-Key` header on `POST /v1/sessions`.
- On exit 3 (transport) after the request may have left, prints the
  key and the next action "retry with `--idempotency-key <key>`",
  with the key visible on stderr and in the JSON error record.
- Never retries by itself with a new key. A retry it does make keeps
  the key.
- Reports `idempotency: unsupported` from the service verbatim; it
  does not upgrade it.
- Exit codes are kept (0, 2, 3, 4); a `conflict` is an HTTP error
  (4) with the conflict body printed.

## 8. Commands (semantics for CLI-13)

| Command             | Surface                | DSP-06 semantics                                                                                                                                         |
| ------------------- | ---------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `agent.dispatch`    | `fleet agent dispatch` | §3–§6. Success requires the bind of step 6 and the prompt receipt of step 7; a second dispatch for the same task while one is active is `conflict`.      |
| `agent.slash.start` | `/fleet start` event   | The event's delivery id feeds the REL-03 key, so a redelivered event replays (C-R093). Success requires the same bind and prompt proof.                  |
| `work.handoff`      | `fleet work handoff`   | Reports `launched` only with a prompt-delivery receipt and the session bind; a handed-off row with no bound session is `pending_sync`, never `launched`. |

## 9. Persisted fields, acceptance and gates

Persisted fields: none new. The reservation is the existing claim
and lease; the binding is the existing `/bind`; the operation record
is REL-03/REL-04's. The only new value written to an existing field
is the dedupe-key form `dispatch-op:<operation_id>`, which is
additive (existing keys keep their forms).

Tests (named; first red is the slice's starting test):

| Test                                                                    | Target              | Proves                                                                                      |
| ----------------------------------------------------------------------- | ------------------- | ------------------------------------------------------------------------------------------- |
| `pmo_dsp_06_direct_launch_timeout_reconciles_without_duplicate` (first) | `fleet-queue`       | B7 apply-then-lose: the read finds the session by marker and binds it; exactly one create.  |
| `pmo_dsp_06_concurrent_operation_one_task_one_session`                  | `fleet-queue`       | two accepted operations racing for one task: one reservation wins, the other is `conflict`. |
| `pmo_dsp_06_duplicate_event_reuses_ids`                                 | `fleet-queue`       | a redelivered `/fleet start` replays the receipt; no new task, session or prompt.           |
| `pmo_dsp_06_hard_gate_rejects_before_effect`                            | `fleet-queue`       | blocker, `manual_only`, park, cap and foreign lease each stop before step 4.                |
| `pmo_dsp_06_claim_first_blocks_worker_steal`                            | `fleet-queue`       | a fleet worker claim between enqueue and launch matches no row.                             |
| `pmo_dsp_06_unknown_never_cancels`                                      | `fleet-queue`       | every `unknown` row in §6 leaves the task and session in place.                             |
| `pmo_dsp_06_definite_failure_compensates_with_evidence`                 | `fleet-queue`       | every definite row in §6 releases or cancels exactly what this operation owns.              |
| `pmo_dsp_06_lost_reservation_reports_orphan`                            | `fleet-queue`       | expired lease with a confirmed session: `partial`, orphan named, no delete.                 |
| `pmo_dsp_06_unsupported_idempotency_is_reported`                        | `fleet-queue`       | with in-memory acceptance the receipt says `unsupported`, keyed or not.                     |
| `pmo_dsp_06_client_prints_key_on_transport_unknown`                     | `fleet-dispatch`    | exit 3 prints the key and the retry action; no automatic retry under a new key.             |
| `direct-launch-reservation.test.ts`                                     | `pm-stack/dispatch` | the service seam: claim-first order, dedupe key form, `202` only after reservation.         |

The Rust tests are a pure launch-step decision model over the DSP-09
failure doubles; they talk to no broker and no provider. The command
is the one the issue names:
`cargo test --manifest-path pkgs/fleet/Cargo.toml -p fleet-queue --lib --features client,queue-broker pmo_dsp_06_direct_launch_timeout_reconciles_without_duplicate`,
listed first with `-- --list` (a zero-test filter is not a pass).
Service seam: `cd pm-stack/dispatch && bun test test/direct-launch-reservation.test.ts`.

Gates:

- **Hard blockers:** REL-03 (acceptance, key scope, persistence of
  the replay record) and #8581 (verb contract v2 mode line, including
  the degraded modes the dispatch verbs report).
- **Shared-file predecessor:** DSP-05 on `work/operations.rs`.
- **Runtime gates:** #9178 and #9179 semantics verified before any
  mutation path ships; offline models proceed now. #9028 is closed.
- **Evidence gate:** #9241 for any schema-coupled change; none is
  proposed here.
- **Stages each needing their own evidence and approval:** the
  pm-dispatch change, deployment, and live acceptance (DSP-07 /
  #6533).
- **Rollback:** disable the claim-first path and return to the
  current order; pending operations, reservations, sessions and
  history are kept. No broad reset or cleanup.

## 10. Requirement trace (design-level)

| ID                                     | Outcome (crosswalk)                                                                                                 | Where               |
| -------------------------------------- | ------------------------------------------------------------------------------------------------------------------- | ------------------- |
| C-R005, C-R107, P-R009, H-R017, O-R023 | Cohesive Fleet CLI across the command families, names reconciled, no parallel dispatch mechanism                    | §7, §8              |
| C-R009                                 | Agents discover, claim safely, execute, report                                                                      | §4, §5              |
| C-R087, C-R088                         | Small authenticated slash grammar; authorized plain-language requests where policy permits                          | §8                  |
| C-R090                                 | An authorized actionable parked-item comment may launch directly, not merely rise in the queue                      | §4 rows 4 and 7     |
| C-R091                                 | Verify → authorize → resolve → dedupe → find live session → send or dispatch → bind and record                      | §3, §4 row 1, §5    |
| C-R093                                 | Repeated deliveries never double-launch                                                                             | §3, §8              |
| C-R152                                 | Scenario C: a live comment routes to the bound session                                                              | §4 row 1            |
| H-R001                                 | Two coordinated tracks preserved                                                                                    | §2                  |
| H-R006                                 | Revalidate atomic claims; direct dispatch bypasses waiting only                                                     | §4 row 7, §5 step 3 |
| H-R012                                 | Authenticated commands, live-session routing, authorized direct dispatch, durable receipts, idempotent webhooks     | §3–§6               |
| H-R016                                 | Fresh work, parked resume, crash after remote mutation, duplicate webhook, stale lease, conflict, launch/bind proof | §6, §9              |
| H-R021                                 | Sanctioned paths; verify ids, route, prompt delivery, session identity, lease; reuse an active compatible session   | §4 row 1, §5        |
| H-R023, H-R045, H-R058                 | Committed design, traceability, staged plan, explicit gaps                                                          | this record         |
| H-R024, H-R031                         | Original commands, options, errors and failures stay in compatibility coverage                                      | §7, §8              |
| H-R034, H-R035                         | Current dispatch authority; verify binding; a queued issue or unaccepted request is not execution                   | §5, §8              |
| H-R041, H-R054                         | Bind the canonical task and directly launch the authorized session with the prompt                                  | §5 steps 2–7        |
| H-R043, H-R056                         | A label, queue position, bare session or pending outbox record is not a launch receipt                              | §8 `work.handoff`   |
| H-R049, H-R062                         | Launch receipts with issue, task and session ids, route and first stage                                             | §5, §6              |
| H-R060                                 | Children stay open until their own tests and evidence pass                                                          | §9                  |
| O-R011                                 | do-now is a durable one-shot request; route or dispatch, bind, preserve blockers, dedupe                            | §4 row 7, §3        |
| O-R014                                 | Authenticated commands, honest delivery and ack, replay and idempotency controls, visible failures                  | §3, §6              |
| O-R038                                 | Cross-document decisions treated as recommendations                                                                 | governing decisions |
| O-R040, O-R041, O-R053                 | Historic request preserved; current dispatch authority; verified dispatch, not labels or queue position             | §5, §8              |
| O-R047                                 | Invoke actual authorized scheduling; bind issue, task and session; avoid double-claim; report failure honestly      | §5 step 3, §6       |
| O-R063, O-R075                         | Route actionable comments on active work to the bound session; parked work may resume or explain why not            | §4 rows 1 and 4     |
| O-R064, O-R076                         | Capability-aware session adapters; never treat a chat post as an agent receipt                                      | §4 row 1, §5 step 7 |
| O-R065, O-R077                         | Dedupe, bind and report receipts; preserve retry intent; never silently downgrade an immediate request              | §4 row 6, §6, §7    |
| S-R002                                 | A do-something-now label affects dispatch, not stored priority                                                      | §4 row 7            |
| S-R004                                 | Authorized bot, webhook and phone controls; comments route to a live session                                        | §4 row 1, §8        |

## 11. Non-goals

- No new queue, store, scheduler, service, resolver, executable,
  GitHub client or creation-outbox producer.
- No CLI registration (CLI-13) and no live acceptance (DSP-07,
  #6533).
- No change to priority values, `expedite=15` / `next-up=25`, or
  `do_now` storage.
- No automatic stop or delete of an orphan session.
- No schema change; no migration.
