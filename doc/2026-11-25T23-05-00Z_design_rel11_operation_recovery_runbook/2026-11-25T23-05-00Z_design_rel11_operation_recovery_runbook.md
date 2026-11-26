# REL-11 — Operational recovery runbook and telemetry

Issue: [#9490](https://github.com/developing-today/code/issues/9490)
(slice `REL-11` of [#9209](https://github.com/developing-today/code/issues/9209),
program [#9196](https://github.com/developing-today/code/issues/9196)).
Status: **design proposal — not an authorization to implement.**
This record elaborates the REL-11 row of the merged design package
(`doc/2026-11-13T03-20-24Z_design_pm-stack-fleet-orchestration/`,
PR #9225): "operator-visible unknown/conflict/stuck operation
inspection and safe reconcile/re-authorize procedure using existing
surfaces", first oracle
`pmo_rel_unknown_merge_outcome_has_safe_recovery_path` ("models
apply-then-timeout, revoked permission and partial issue closure"),
negatives "unsupported retry endpoint, parent epic closure,
unauthorized principal, stale operation", and the rule "no new
scheduler; any recurring cadence requires registry update same pass".
It decides which operation states an operator must be able to see,
what counts as stuck, the recovery procedure for each, what is never
allowed, the telemetry and where it goes, and what the runbook
`doc/fleet/lifecycle-recovery.md` must contain and when it is
published. It authorizes no code, schema, alert route, schedule or
deployment (§9).

Governing decisions consumed, not re-opened:

- **Component #9209 §3.2, §4:** an old operation is never silently
  dropped; a new owner may wait and reconcile, supersede _before
  send_ by CAS with actor and reason, create a compensating operation
  after observing the result, or re-authorize the same operation id
  when the intended state is still valid; once a send may have
  happened the effect is `unknown` and needs read-back; no
  cancellation fiction; revocation stops future sends, not in-flight
  ones. Retry classes: validation/auth/permission/invalid target are
  terminal pending operator; SHA or version mismatch is `conflict`;
  timeout after send is `unknown`; rate limits follow the provider's
  retry-after, with no invented count or ceiling. Reconciliation stays
  inside existing Fleet mechanisms; no new scheduler.
- **REL-03** (`doc/2026-11-25T20-47-34Z_design_rel03_idempotency_identity_receipts/`):
  receipts, `AUTH_DENIED` on re-presentation after revocation, and
  "the stored receipt stays readable through the REL-08 operator
  surface, not through replay".
- **REL-04** (`doc/2026-11-25T22-30-00Z_design_rel04_lifecycle_mutation_outbox/`):
  operation statuses `pending`, `delivering`, `confirmed`,
  `retryable_failure`, `conflict`, `terminal_failure`, `unknown`; the
  attempt log; per-target order.
- **REL-06** (`doc/2026-11-23T12-20-33Z_design_rel06-github-preconditions-readback/`):
  effect-specific read-back and resolution of `unknown`.
- **REL-07** (`doc/2026-11-25T22-26-00Z_design_rel07_completion_state_machine/`):
  effect states, derived completion status, recovery reads back
  before any resend, finalize is operation-scoped, a stale worker's
  late answer is evidence only.
- **REL-08** (`doc/2026-11-25T22-52-00Z_design_rel08_work_complete_cli/`):
  `fleet work complete` resume by key, and the read/retry façades
  `fleet sync status`, `sync outbox list|inspect|retry`,
  `sync verify`, `sync run`.
- **AUTH-03** (`doc/2026-11-25T22-10-00Z_design_auth03_managed_comment_recovery/`):
  managed-comment recovery by marker; REL-11 links to it rather than
  restating it.
- **AGENTS.md "Fleet schedules":** any created or changed schedule
  updates `doc/fleet-schedule-registry.md` in the same pass.

## Verified state on main

Read at `origin/main` on 2026-11-25:

- **No general operation store.** Operations, effects and receipts
  are design-only (REL-03/04/07 records; CEN-04 §3.9 "absent —
  gated"). `doc/fleet/lifecycle-recovery.md` does not exist.
- **The one live outbox** is the creation pilot
  (`creation_outbox.rs`, runbook `doc/fleet/creation-outbox.md`):
  `fleet queue outbox enqueue|list|metrics|claim|resolve|drain`,
  metrics "pending depth, oldest pending age, attempts, last failure
  class, circuit state", ambiguous creates go `uncertain` and
  reconcile by marker, and an expired drainer lease is claimed as
  `uncertain`. That is the pattern this record extends; it is not
  changed.
- **Existing operator surfaces:** the broker event timeline
  (`/events-append`, `/events-list`, `/events-count` in
  `turso_queue/stacks_outbox.rs`) and the needs-owner alert throat
  (`needs_drew.rs`: one alert per `(source, key)`, deduplicated by
  the PM aggregator).
- **Schedules:** `doc/fleet-schedule-registry.md` is the registry.

## 1. Intent

When an operation cannot finish on its own, an operator — a person
or an agent acting for one — can see it, understand in one read
what Fleet knows and does not know about the remote side, and take
the one safe next step. No procedure ever repeats a remote effect
whose outcome is unproven, closes something the original request did
not name, or reports success the remote does not show.

## 2. Ownership

| Concern                                     | Owner                                    | This slice                         |
| ------------------------------------------- | ---------------------------------------- | ---------------------------------- |
| Runbook `doc/fleet/lifecycle-recovery.md`   | REL-11                                   | **owns**                           |
| Recovery fixture in `work/operations.rs`    | REL-11 (fixture), REL-03/04 (module)     | **owns** the fixture and its tests |
| Operation and effect states, transitions    | REL-04, REL-07                           | consumer                           |
| Read-back and `unknown` resolution          | REL-06                                   | consumer                           |
| `fleet sync …` commands, registration, help | REL-08 (contract), CLI-13 (registration) | consumer                           |
| Event timeline, alert throat                | existing broker / PM aggregator owners   | consumer; no new route             |
| Recurring cadence                           | `doc/fleet-schedule-registry.md` owner   | none added                         |

## 3. What an operator must see

An operation needs attention when it is in one of these conditions.
Each is computed from stored state, never from a guessed timer.

| Condition             | Defined as                                                                                                 |
| --------------------- | ---------------------------------------------------------------------------------------------------------- |
| `unknown`             | an effect whose send may have left and whose read-back has not settled it                                  |
| `conflict`            | an effect or operation stopped by a precondition (head moved, version changed, gate failed, late activity) |
| `terminal_failure`    | an effect refused for validation, permission or target reasons                                             |
| `partial`             | a completion with some required effects `confirmed` and some `conflict` or `terminal_failure`              |
| `stuck_due`           | `next_attempt_at` is in the past and no attempt started within one relay pass after it                     |
| `stuck_delivering`    | status `delivering` and the relay's claim on it has expired                                                |
| `orphaned_completing` | a work in `completing` whose initiating lease has expired and which no live holder has reconciled          |
| `stale`               | an operation superseded by a newer one for the same target, still holding unsent effects                   |

"Stuck" is defined against the operation's own stored schedule and
claim, not a fixed age, so no threshold is invented here (component
§4). The relay pass interval is the existing drainer's.

## 4. Procedures

Every procedure starts the same way:

1. `fleet sync outbox inspect <operation_id>`: targets, each effect's
   status, the attempt log in order, the last read-back, and the
   operation's principal and key. Credentials are never shown.
2. If any effect is `delivering`, `pending` or `unknown`, read back
   first (`fleet sync outbox retry <op>` always reads back before
   sending; REL-07 §6). Only then choose a step below.

| Situation                                                  | Safe step                                                                                                                                                                                                 | Never                                                                      |
| ---------------------------------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | -------------------------------------------------------------------------- |
| **Merge applied, answer lost** (`unknown`)                 | read back: PR merged with the expected head and method → `confirmed`, continue the operation; open with the same head → `planned`, resend the same operation; closed unmerged or another SHA → `conflict` | merge again without read-back; infer merged from closed                    |
| **Read-back itself fails** (GitHub unavailable)            | leave `unknown`; the next relay pass or a later `retry` reads back again                                                                                                                                  | resend; mark failed                                                        |
| **Head moved / gate failed** (`conflict`)                  | the agent re-verifies, then starts a **new** operation with the new expected head and a new key; the old one is recorded `superseded` by CAS with actor and reason                                        | edit the old operation's target or head                                    |
| **Permission revoked** (`terminal_failure`, `AUTH_DENIED`) | future sends stop; read back anything that may have left; after an owner restores the grant, `retry` re-authorizes the **same** operation if its intended state is still valid                            | retry under another principal; treat revocation as undoing a landed effect |
| **Partial completion** (merge confirmed, close failed)     | the work stays `completing`; fix the cause, then `fleet work complete --idempotency-key <key> --verify` or `retry`, which resends only the unconfirmed close                                              | reopen or unmerge to simulate rollback; close the queue item by hand       |
| **`stuck_delivering`**                                     | the next claimant takes it as `unknown` and reads back, as the creation pilot does                                                                                                                        | resend on claim                                                            |
| **`stuck_due`**                                            | check the relay is running (`fleet sync status`); `fleet sync run --limit 1` drains it once                                                                                                               | add a timer or loop                                                        |
| **`orphaned_completing`**                                  | the next holder must reconcile the open operation first (REL-07 §6); finalize may commit under the operation even though the old lease expired                                                            | prepare a second completion operation                                      |
| **`stale`**                                                | reconcile the older operation first, then let the newer one proceed (per-target order)                                                                                                                    | let the newer operation send first                                         |
| **Managed comment unknown**                                | follow AUTH-03 (marker, posting identity, digest; wait out the visibility window)                                                                                                                         | post a second comment because the first answer timed out                   |

## 5. Refusals the procedures rely on (the negatives)

| Negative                   | Refusal                                                                                                                                                                                                                                                           |
| -------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Unsupported retry endpoint | an effect kind with no REL-06 read-back method cannot be retried: `retry_unsupported`; it stays `unknown` and `inspect` shows `next_action: owner_decision`. No manual "mark done" exists in v1; whether to add one with recorded evidence is asked of the owner. |
| Parent epic closure        | `retry` and `run` only act on the operation's stored targets. A parent, epic or design issue is never added; a request naming one is `completion_ref_not_linked` (REL-07 §3).                                                                                     |
| Unauthorized principal     | `inspect` needs read authority on the work; `retry` and re-authorization need the effect's own authority, re-checked at every send. A failure is `AUTH_DENIED` and changes nothing.                                                                               |
| Stale operation            | `retry` on a `superseded` operation is refused with `operation_superseded`, naming the newer one; a stale worker's resend is refused by its fence generation (REL-02).                                                                                            |

## 6. Telemetry

No new pipeline, store or scheduler. Two existing sinks carry it.

**Counts and ages** (read on demand by `fleet sync status --json`,
shaped like the creation pilot's metrics): per status, the number of
operations and the oldest age; the counts of each §3 condition;
attempts in the last relay pass; the last failure class per effect
kind.

**Events** on the broker timeline (`/events-append`), one per
transition into a §3 condition, kind `operation.<condition>`, with
the operation id, work, effect kind and reason code. Bodies, tokens
and comment text are never included.

**Alerts** through the existing needs-owner throat only for states
no automatic step can clear: `conflict`, `terminal_failure` on a
required effect, `partial`, and `unknown` with `retry_unsupported`.
The dedupe key is `operation:<operation_id>:<condition>`, so a
repeated detection raises one alert. `unknown`, `stuck_due` and
`stuck_delivering` do not alert by themselves; they are expected to
clear on the next pass and show in the counts.

**Detection cadence.** Conditions are computed inside the existing
relay drain pass and on every `fleet sync status`. Nothing new runs
on a timer. If an owner later approves a recurring check, it lands
with its row in `doc/fleet-schedule-registry.md` in the same PR.

## 7. The runbook `doc/fleet/lifecycle-recovery.md`

Published with the first implementation that writes general
operation rows, not before, so it never documents commands that do
not exist. Until then `doc/fleet/creation-outbox.md` remains the only
outbox runbook. Its required sections:

1. What an operation, effect and receipt are, in two paragraphs, with
   links to REL-03/04/07.
2. The §3 conditions table.
3. One entry per §4 row: symptom, `inspect` output to look for, the
   safe step as exact commands, and what not to do.
4. The §5 refusals and what each means for the operator.
5. Where the §6 counts, events and alerts appear.
6. Escalation: what goes to the owner and with which facts.

A consistency check runs with the runbook's PR: every `fleet …`
command line in the runbook must parse against the registered CLI
inventory (`fleet --help` / `census/commands.jsonl` status
`implemented`), so the runbook cannot drift from the binary.

## 8. Tests

In `work/operations.rs`, as a pure fixture over REL-04/REL-07 models:

| Test                                                                | Asserts                                                                               |
| ------------------------------------------------------------------- | ------------------------------------------------------------------------------------- |
| `pmo_rel_unknown_merge_outcome_has_safe_recovery_path`              | apply-then-timeout → `unknown`; read-back merged → `confirmed`; no second merge send  |
| `pmo_rel_recovery_unknown_merge_not_applied_resends_same_operation` | read-back open with same head → `planned`, resend carries the same operation id       |
| `pmo_rel_recovery_readback_unavailable_stays_unknown`               | read-back error → still `unknown`, no send                                            |
| `pmo_rel_recovery_revoked_permission_stops_future_sends`            | revocation → `terminal_failure`, in-flight read back, no new send until re-authorized |
| `pmo_rel_recovery_partial_issue_closure_resends_only_close`         | merge confirmed, close failed → retry sends only the close; work stays `completing`   |
| `pmo_rel_recovery_unsupported_retry_endpoint_refused`               | effect kind with no read-back → `retry_unsupported`, state unchanged                  |
| `pmo_rel_recovery_never_adds_parent_target`                         | retry with a parent epic named → `completion_ref_not_linked`, target set unchanged    |
| `pmo_rel_recovery_unauthorized_principal_changes_nothing`           | inspect and retry without authority → `AUTH_DENIED`, no state change                  |
| `pmo_rel_recovery_stale_operation_refused`                          | retry on superseded → `operation_superseded`; stale fence resend refused              |
| `pmo_rel_recovery_conditions_derived_without_fixed_thresholds`      | `stuck_due` and `stuck_delivering` follow stored schedule and claim only              |
| `pmo_rel_recovery_alert_dedupe_one_per_condition`                   | repeated detection yields one alert key                                               |
| `pmo_rel_recovery_events_carry_no_bodies_or_tokens`                 | event payloads contain ids and codes only                                             |

Done command (remote runner):

```sh
cargo test --manifest-path pkgs/fleet/Cargo.toml -p fleet-queue --lib --features client,queue-broker pmo_rel_unknown_merge_outcome_has_safe_recovery_path -- --list
cargo test --manifest-path pkgs/fleet/Cargo.toml -p fleet-queue --lib --features client,queue-broker pmo_rel_
```

## 9. Gates

- **No persisted field.** Conditions are derived from REL-04/REL-07
  rows. Proposed reason codes `retry_unsupported` and
  `operation_superseded` go to the `ReasonCode` registry owner.
- **Alert route** reuses the needs-owner throat; enabling it for
  operations is a runtime change that needs the aggregator owner's
  yes and its own evidence.
- **Runbook** lands with the first general-operation implementation
  (§7) and passes the consistency check.
- **Owner decisions asked:** whether a manual resolve-with-evidence
  command is wanted for effects with no read-back; which §3
  conditions page a person versus only count.
- **Rollback:** turn off the new events and alerts; operations and
  receipts stay; the creation pilot is untouched.

## 10. Requirement trace (design-level)

| IDs            | Outcome                                                                        | Where      |
| -------------- | ------------------------------------------------------------------------------ | ---------- |
| C-R142         | no pretended atomicity between GitHub and the local store; `unknown` read back | §4, §5     |
| C-R137, P-R008 | ownership, execution and GitHub state kept distinct; existing owners reused    | §2, §3, §6 |
| C-R136, P-R037 | interrupted sessions recover from stored operations, not memory                | §4         |
| P-R024         | stable identifiers, distinct statuses, no secrets in output or events          | §3, §5, §6 |
| O-R011         | durable one-shot dispatch deliveries use the same inspection and dedupe rules  | §3, §6     |

## 11. Non-goals

- No new scheduler, timer, daemon or alert route.
- No change to the creation outbox or its runbook.
- No new effect kind, status or transition (REL-04, REL-07).
- No command registration (CLI-13) and no new `sync` verb beyond
  REL-08's.
- No manual override of an unproven remote outcome.
