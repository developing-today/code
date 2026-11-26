# DSP-07 — Auditable launch receipt and #6533 acceptance fixture

Issue: [#9447](https://github.com/developing-today/code/issues/9447)
(slice `DSP-07` of [#9203](https://github.com/developing-today/code/issues/9203),
program [#9196](https://github.com/developing-today/code/issues/9196)).
Status: **design proposal — not an authorization to implement.**
This record elaborates the DSP-07 row of the merged design package
(`doc/2026-11-13T03-20-24Z_design_pm-stack-fleet-orchestration/`,
PR #9225). The row lists what a launch receipt "must include" and
says a validator rejects each omitted field against a golden corpus.
It never says where those fields live in the shared result envelope,
which read source stands behind each stage, what a provider that
cannot read a prompt back can ever prove, or how the existing #6533
live fixture is used without a second live-proof issue. This record
_decides_ those things. It authorizes no code, schema, deployment,
production session or live acceptance; those remain separately
gated stages (see §9).

Governing decisions consumed, not re-opened:

- **Shared result envelope** (`work/contracts.rs`, read-only here):
  `CommandResult` (`fleet.result/v1`) with `operation_id`, `work`,
  `work_version`, `outcome`, `effects[]`, `warnings`, `error`,
  `next_action`, `observed_at`; the eight-value `Outcome`; `Effect`
  with `requested` / `observed` / `verified` kept apart. "Do not
  invent a second receipt state machine."
- **DSP-05 record** §3.3–§3.4 and §7: the delivery ladder
  `reserved → adapter_accepted → delivered → acknowledged →
progressing`; `progressing` is never promoted to `acknowledged`;
  `unknown` is never read as "not sent"; the effects
  `session_binding`, `message_delivery`, `session_ack`; and §7.3:
  "placing [per-stage timestamps and evidence references] is
  DSP-07's receipt contract." On OpenCode, `delivered` and
  `acknowledged` are **unverified** today (no message-read call).
- **DSP-06 record**: claim-first reservation, the launch sequence
  steps 1–9 and their stages, the compensation table, and
  `idempotency: unsupported` until REL-03/REL-04 persistence lands.
- **DSP-09 record**: invariants I1–I9, in particular I3 ("a stage is
  `verified` only when its evidence came from that stage's named read
  source after the effect") and I6 (every external node is bound,
  compensated with evidence, or listed as an orphan).
- **Component #9203 §3.5**: effect stages are not a second outcome
  enum; an orphan is surfaced, never stopped automatically.
- **#6533** stays the one live launch-and-bind acceptance. Its
  acceptance criteria are: a handed-off task gains a non-null
  `executed_lane` and `session_id`, the bound session posts its first
  checkpoint, a failed launch surfaces with a named reason, handoff
  cycles stop burning attempts, and one previously stuck task is
  observed launched end to end. DSP-07 does not edit it and does not
  file another live-proof issue.

## 1. Intent

A launch is "done" only when one receipt shows, stage by stage and
from the source that can actually see each stage, that the task is
reserved, a session exists, that session is bound to that task, the
prompt reached that session, and that session did meaningful work on
that task. Everything weaker (a session id, a queued issue, a label,
a queue position, a `handed_off` row, a 2xx from the provider, a
pending outbox row) is named, kept in the receipt for what it is,
and never accepted as proof. The validator is a pure function over
the receipt; it is what turns that rule into a test.

## 2. Ownership

| Concern                                                | Owner                                                  | DSP-07 role                                |
| ------------------------------------------------------ | ------------------------------------------------------ | ------------------------------------------ |
| Launch receipt type, stage evidence, validator, corpus | `pkgs/fleet/crates/fleet-queue/src/work/operations.rs` | writes (after DSP-06 on the shared file)   |
| `CommandResult`, `Effect`, `Outcome`, `WorkRef`        | `work/contracts.rs`                                    | reads only; unchanged                      |
| Delivery ladder and send receipts                      | DSP-05                                                 | consumes                                   |
| Launch sequence, reservation, compensation             | DSP-06                                                 | consumes                                   |
| Crash-cut oracle                                       | DSP-09                                                 | its doubles produce receipts DSP-07 checks |
| Stable receipt storage and replay                      | REL-03 / REL-04                                        | stores the receipt; DSP-07 defines it      |
| Live launch evidence                                   | #6533 owner process                                    | supplies evidence; DSP-07 validates it     |

## 3. Where the receipt lives

**Decision:** the launch receipt is a wrapper, `fleet.launch-receipt/v1`,
that carries the shared `CommandResult` unchanged as `result` and adds
the launch-specific parts beside it. `contracts.rs` is not edited:
`Effect` has no slot for a source or a time, and widening the shared
envelope for one verb family would be a second state machine by the
back door.

```text
LaunchReceipt {
  schema_version: "fleet.launch-receipt/v1",
  result:     CommandResult,        // outcome, effects[], error, next_action, observed_at
  refs:       LaunchRefs,           // §3.1
  stages:     [StageEvidence],      // §3.2, one or more per effect field
  orphans:    [ExternalRef],        // DSP-09 I6
  idempotency: "supported" | "unsupported",
}
```

A consumer that only knows `fleet.result/v1` reads `result` and gets
a correct, if less detailed, answer. The wrapper is serialized whole
wherever a launch answers (`fleet dispatch launch`, `/v1/sessions`,
`fleet work handoff`, the `/fleet start` event receipt).

### 3.1 `LaunchRefs` (identity, never proof)

| Field                               | Content                                                                                        | Required when          |
| ----------------------------------- | ---------------------------------------------------------------------------------------------- | ---------------------- |
| `operation_id`                      | the accepted operation (`result.operation_id` repeated for search)                             | always                 |
| `idempotency_key`                   | the REL-03 key, or null with `idempotency: unsupported`                                        | always (nullable)      |
| `dedupe_key`                        | the queue dedupe key used, e.g. `dispatch-op:<operation_id>` (DSP-06 §5)                       | once `reserved`        |
| `event_ref`, `comment_ref`          | delivery id and comment id when an event or comment started it                                 | when event-started     |
| `principal`, `capabilities`, `auth` | authenticated login, the capability set used, the decision and policy version                  | always                 |
| `work`                              | `(authority_id, work_id)` as the shared `WorkRef`                                              | once `resolved`        |
| `issue_url`                         | external locator                                                                               | when an issue exists   |
| `queue_ref`                         | queue name and task id                                                                         | once `reserved`        |
| `directive`                         | `do_now` consumed or not, kept apart from the priority value; never rewritten to `expedite=15` | always                 |
| `route`, `model`                    | the route taken and the model stamp                                                            | once `session_created` |
| `lease`                             | holder worker and lane, expiry; `generation` only when the broker contract provides one        | once `reserved`        |
| `session_ref`, `provider`           | the actual session id and adapter                                                              | once `session_created` |
| `worktree`                          | path and registry record id                                                                    | once `launching`       |

A V1 worker-id claim is recorded as such; the receipt never presents
it as a secret lease token or a monotone fence (#9203 §3.4).

### 3.2 `StageEvidence` (the proof)

```text
StageEvidence {
  stage:          "reserved" | "launching" | "session_created" | "task_bound"
                | "prompt_delivered" | "acknowledged" | "progressing" | "verified_terminal",
  effect_field:   string,           // the result.effects[].field this backs
  value:          string,           // the state shown, e.g. "bound", "delivered"
  source:         ReadSource,       // §3.3; a closed list
  source_before:  string | null,    // what the source showed before the effect, when read
  source_after:   string,           // what the source showed after the effect
  effect_at:      timestamp,        // when the effect was attempted
  read_at:        timestamp,        // when source_after was read
  evidence_ref:   string,           // event id, timeline row, provider message id, checkpoint id
}
```

Effect fields used by a launch, each backed by at least one stage:

| `effect.field`     | Stage               | `verified` may be set only from                                                                                                               |
| ------------------ | ------------------- | --------------------------------------------------------------------------------------------------------------------------------------------- |
| `task_reservation` | `reserved`          | `queue_task_read`: the task row read after the claim shows this worker and lease                                                              |
| `worktree`         | `launching`         | `worktree_registry_read`                                                                                                                      |
| `session`          | `session_created`   | `provider_session_read`: the provider lists or gets the session by id                                                                         |
| `session_binding`  | `task_bound`        | `queue_task_read` or `queue_timeline_read` after the bind: the task names this `session_ref` (`session_bound` event or `progress.session_id`) |
| `message_delivery` | `prompt_delivered`  | `provider_message_read` (DSP-05 `delivered`), **or** `session_checkpoint_marker` (§4)                                                         |
| `session_ack`      | `acknowledged`      | an authenticated provider signal; `unsupported` is a legal value                                                                              |
| `session_progress` | `progressing`       | `queue_timeline_read`: a `checkpoint` event for this task written under this session after the bind                                           |
| `launch_terminal`  | `verified_terminal` | the three reads above repeated at verification time                                                                                           |

### 3.3 `ReadSource` (closed)

`queue_task_read`, `queue_timeline_read`, `worktree_registry_read`,
`provider_session_read`, `provider_message_read`,
`session_checkpoint_marker`, `provider_ack_signal`,
`adapter_response`, `github_issue_read`, `self_report`.

`adapter_response` (a 2xx) and `self_report` (the launcher's own
memory of what it did) may fill `observed`; neither ever fills
`verified`. An unknown source string is a schema failure, not a
guessed source.

## 4. Proving prompt delivery on a provider that cannot read back

OpenCode exposes no message read today (DSP-05 §6), so DSP-05's
`delivered` cannot be reached there. Requiring it would make every
OpenCode launch unprovable; accepting the 2xx would accept a false
receipt. **Decision:** the launch prompt instructs the session to
write its first queue checkpoint with the operation's correlation
marker (`operation_id`). A `checkpoint` event that (a) is for this
task, (b) is written by the holder under this `session_ref`, (c) is
later than the bind, and (d) carries the marker is recorded as
`message_delivery` evidence with source `session_checkpoint_marker`.

- It proves the session received this prompt: only the prompt
  carried the marker.
- It does **not** set DSP-05's `delivered` level and is never shown
  as `acknowledged`; the two ladders stay separate, and the receipt
  names which source was used.
- The same event also backs `session_progress`, so on OpenCode one
  checkpoint carries both. A checkpoint **without** the marker backs
  `session_progress` only; delivery then stays at `adapter_accepted`
  and the launch is `pending_sync`, not verified.
- When a provider later offers a message read, `provider_message_read`
  is used first and the checkpoint rule stays as a fallback.

## 5. Outcome rules the validator enforces

The validator is `verify_launch_receipt(&LaunchReceipt) -> Vec<Violation>`.
An empty list means the receipt is internally consistent; it never
means the launch succeeded unless `result.outcome` says so.

| `result.outcome`     | Must hold                                                                                                                                                                                                                                                                                                                |
| -------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| `verified_complete`  | `task_reservation`, `session`, `session_binding`, `message_delivery` and `session_progress` each `verified` from an allowed source; every `read_at` ≥ its `effect_at`; the binding and progress name the same `session_ref` as `refs.session_ref`; no orphans; `idempotency` stated. `session_ack` may be `unsupported`. |
| `pending_sync`       | at least `session_binding` verified; one or more of delivery or progress only `observed`; `next_action` names the read that would finish it.                                                                                                                                                                             |
| `partial`            | some external effect exists (stage with `source_after`) and the launch did not complete; each landed effect and each orphan is named. Never `rejected` while an effect exists (DSP-09 I9).                                                                                                                               |
| `uncertain`          | at least one stage `unknown`; `next_action: reconcile` with the read named; no stage after the unknown one is `verified`.                                                                                                                                                                                                |
| `conflict`           | the holder or the bound session is someone else's, named in `error`; this operation's session, if any, is an orphan.                                                                                                                                                                                                     |
| `rejected`           | no stage carries a `source_after` showing an external effect; `error.code` is a registry reason.                                                                                                                                                                                                                         |
| `accepted`/`running` | no `verified` field claims a stage later than the last evidence.                                                                                                                                                                                                                                                         |

Every violation has a code. The fixed list:
`missing_field:<name>`, `missing_stage:<effect>`, `unverifiable_source`,
`stale_read` (`read_at` < `effect_at`), `session_mismatch`,
`progress_before_bind`, `progress_other_session`,
`delivery_without_marker`, `verified_after_unknown`,
`rejected_with_effect`, `orphan_unlisted`, `fence_overclaimed`,
`directive_rewritten`, `secret_present`, `unknown_outcome`.

## 6. The golden corpus

The corpus lives in the `#[cfg(test)]` module of `operations.rs`
(the slice owns no other file): one golden receipt for a verified
OpenCode launch, built from DSP-09's doubles, and a generator that
derives the negatives from it.

1. **One-field deletions.** For every required field in §3.1 and
   every stage in §3.2, the receipt with that one item removed must
   produce exactly the matching `missing_*` violation. This is how
   "rejects each omitted field" is proved without hand-writing a
   case per field.
2. **Named substitutes**, each a separate case that must be rejected
   as `verified_complete`:
   - a session id alone (no binding, delivery or progress stage);
   - a queued issue or a queue position (`reserved` with no session);
   - a `handed_off` row with a null `session_id` (the #6533 defect);
   - a label, or a pending outbox row, offered as a stage source;
   - a provider 2xx as the only delivery evidence;
   - a bind with no read-back (`self_report` only);
   - a checkpoint from another session, or before the bind;
   - a checkpoint for another task;
   - progress present but delivery absent;
   - any stage `unknown` with outcome `verified_complete`;
   - a worker-id claim presented with a fence generation;
   - a Slack or chat post offered as acknowledgement.
3. **Consistency cases**: `rejected` while a session exists;
   `partial` with an unlisted orphan; two receipts for one operation
   with different `session_ref`s.
4. **Redaction**: a receipt containing a prompt body, an
   `Authorization` value or a token-shaped string fails
   `secret_present`. Prompt content is referenced, never included.

## 7. #6533 as the live fixture

DSP-07 maps #6533's acceptance to receipt fields so its owner can
judge a live run with the validator instead of by reading logs:

| #6533 criterion                                                 | Receipt evidence                                                                            |
| --------------------------------------------------------------- | ------------------------------------------------------------------------------------------- |
| handed-off task gains non-null `executed_lane` and `session_id` | `session_binding` verified from `queue_task_read` / `queue_timeline_read`                   |
| the bound session posts its first checkpoint                    | `session_progress` verified (and, with the marker, `message_delivery`)                      |
| a launch that cannot happen surfaces with a named reason        | `rejected` / `partial` / `uncertain` with `error.code` and the stage reached                |
| handoff cycles stop burning attempts                            | outside the receipt; stays #6533's own check on the task's attempt counter                  |
| one previously stuck task observed launched end to end          | one live receipt that validates as `verified_complete`, attached by the #6533 owner process |

The live run is separately authorized and is not part of this
slice. Passing `pmo_dsp_07` unit tests does not close #6533 and does
not count as live acceptance; a live run that fails the validator
records its violations on #6533 as a blocker.

## 8. Commands

| Command             | Receipt use                                                                                                                                        |
| ------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------- |
| `agent.dispatch`    | answers with the launch receipt; exits non-zero unless the outcome is `verified_complete`, `pending_sync` or `running` (exact codes are CLI-13's). |
| `agent.slash.start` | the event receipt is the launch receipt with `event_ref` and `comment_ref` set.                                                                    |
| `work.handoff`      | may say `launched` only when the receipt validates as `verified_complete`; otherwise prints the outcome and next action.                           |

## 9. Persisted fields, acceptance and gates

Persisted fields: none new in this slice. REL-03/REL-04 store the
receipt as an opaque versioned document under the operation; the
queue's existing timeline and task rows are the read sources.

Tests (first red is the slice's starting test):

| Test                                                            | Proves                                                                               |
| --------------------------------------------------------------- | ------------------------------------------------------------------------------------ |
| `pmo_dsp_07_receipt_rejects_missing_prompt_ack_or_bind` (first) | removing the delivery, progress or binding stage from the golden receipt is rejected |
| `pmo_dsp_07_golden_receipt_validates`                           | the golden OpenCode receipt has no violations                                        |
| `pmo_dsp_07_each_omitted_field_rejected`                        | the one-field deletion generator over §3.1 and §3.2                                  |
| `pmo_dsp_07_session_id_alone_rejected`                          | substitute: bare session                                                             |
| `pmo_dsp_07_queued_issue_rejected`                              | substitute: queued issue or position                                                 |
| `pmo_dsp_07_handed_off_unbound_rejected`                        | substitute: the #6533 defect shape                                                   |
| `pmo_dsp_07_adapter_2xx_not_delivery`                           | substitute: 2xx alone                                                                |
| `pmo_dsp_07_checkpoint_marker_proves_delivery`                  | §4 accepted path, and its four conditions each rejected when broken                  |
| `pmo_dsp_07_stale_read_rejected`                                | `read_at` before `effect_at`                                                         |
| `pmo_dsp_07_unknown_never_verified`                             | DSP-09 I3                                                                            |
| `pmo_dsp_07_rejected_with_effect_flagged`                       | DSP-09 I9                                                                            |
| `pmo_dsp_07_orphan_must_be_listed`                              | DSP-09 I6                                                                            |
| `pmo_dsp_07_fence_not_overclaimed`                              | V1 worker-id claims                                                                  |
| `pmo_dsp_07_secrets_rejected`                                   | redaction                                                                            |
| `pmo_dsp_07_result_view_is_shared_envelope`                     | `receipt.result` validates under `CommandResult::validate` unchanged                 |

Command:
`cargo test --manifest-path pkgs/fleet/Cargo.toml -p fleet-queue --lib --features client,queue-broker pmo_dsp_07`,
after proving with `-- --list` that the `pmo_dsp_07` tests are listed
(a zero-test filter is not a pass).

Gates:

- **Hard blocker:** DSP-05 (its record is on main; its implementation
  lands the effects and ladder this receipt checks).
- **Shared-file predecessor:** DSP-06 on `work/operations.rs`.
- **Evidence gates:** #6533 (live run, its owner's process), #8187
  (closed launch-on-handoff baseline, not new live acceptance),
  #2668 (staged-launch evidence).
- **Stages each needing their own evidence and approval:** the
  prompt change that asks the session for a marked first checkpoint,
  deployment, and the live #6533 run.
- **Rollback:** stop emitting the wrapper; `result` alone remains the
  answer. Stored receipts and history are kept.

## 10. Requirement trace (design-level)

| ID                                     | Outcome (crosswalk, abridged)                                                                     | Where                  |
| -------------------------------------- | ------------------------------------------------------------------------------------------------- | ---------------------- |
| C-R005, C-R107, P-R009, H-R017, O-R023 | Cohesive Fleet CLI; names reconciled; no parallel dispatch mechanism                              | §8                     |
| C-R009                                 | Agents claim safely, execute and report                                                           | §3, §5                 |
| C-R087                                 | Small authenticated slash grammar                                                                 | §8 `agent.slash.start` |
| C-R091                                 | Verify → authorize → resolve → dedupe → send or dispatch → bind and record receipt                | §3.1, §3.2             |
| C-R152                                 | Scenario C: a live comment routes to the bound session with acknowledgement recorded              | §3.2 `session_ack`     |
| H-R001                                 | Two coordinated tracks preserved                                                                  | §2                     |
| H-R012                                 | Durable receipts, live-session routing, authorized direct dispatch                                | §3                     |
| H-R016                                 | Crash after remote mutation, duplicate webhook, stale lease, conflict, live launch and bind proof | §5, §6, §7             |
| H-R021                                 | Verify created ids, route, prompt delivery, session identity, lease, meaningful start             | §3.1, §3.2, §4         |
| H-R022                                 | Require acknowledgement or meaningful resumed work before claiming recovery                       | §3.2, §4, §5           |
| H-R023, H-R045, H-R058                 | Committed design, traceability, explicit gaps                                                     | this record            |
| H-R024, H-R031                         | Original errors, receipts and failures stay in acceptance coverage                                | §6                     |
| H-R035                                 | A queued issue or unaccepted dispatch request is not proof of execution                           | §6 item 2              |
| H-R041, H-R054                         | Bind the canonical task and launch the authorized session with the prompt                         | §3.2                   |
| H-R043, H-R056                         | A label, queue position, bare session or pending outbox record is not a launch receipt            | §1, §6 item 2          |
| H-R049, H-R062                         | Launch receipts with issue, task and session ids, route and first stage                           | §3.1                   |
| H-R060                                 | Children stay open until their own tests and evidence pass                                        | §7, §9                 |
| O-R014                                 | Honest delivery and ack, replay and idempotency controls, visible failures                        | §3.3, §5               |
| O-R041                                 | Verified dispatch, not urgency labels or queue position                                           | §6 item 2              |
| O-R063, O-R075                         | Route actionable comments to the bound session with delivery and acknowledgement                  | §3.2, §4               |
| O-R064, O-R076                         | Capability-aware adapters; never treat a chat post as an agent receipt                            | §3.3, §6 item 2        |
| O-R065, O-R077                         | Bind and report receipts; on failure preserve retry intent                                        | §5                     |

## 11. Non-goals

- No change to `contracts.rs`, the shared `Effect` or the outcome
  vocabulary.
- No second live-proof issue and no edit to #6533; no production
  launch from this slice.
- No new store; REL-03/REL-04 own receipt storage.
- No promotion of progress to acknowledgement, and no use of a chat
  post as either.
- No prompt content, credential or private note in any receipt.
