# DST-03 — The accepted priority intent: version, decision provenance and operation intent in one write

Issue: [#9453](https://github.com/developing-today/code/issues/9453)
(slice `DST-03` of [#9200](https://github.com/developing-today/code/issues/9200),
program [#9196](https://github.com/developing-today/code/issues/9196)).
Status: **design proposal — not an authorization to implement.**
The issue asks: "Accepted-intent write records work version + decision
provenance + required operation intent through existing broker. Owns
proposed `src/work/distribution.rs`, shared `src/work/contracts.rs`,
integration `src/turso_queue.rs`; no new table before schema gate."
This record decides what an accepted priority command writes, in which
transaction, what "decision provenance" contains, and how the intent
digest ties the acceptance to the owner policy it was checked against.
It authorizes no code, schema, remote write or deployment.

Governing decisions consumed, not re-opened:

- **DST-01** (#9451, `work/contracts.rs`): `Command` and its intent DTO.
  `request_digest` already covers `owner_policy_ref`,
  `expected_work_version`, `lease_generation`, `observed_snapshot_ref`,
  kind, arguments and reason. It excludes `operation_id`, the
  idempotency key, `delivery_id` and `dry_run`.
- **REL-03** (`doc/2026-11-25T20-47-34Z_design_rel03_idempotency_identity_receipts/`):
  the idempotency scope, the stored receipt and its immutable identity
  block, and `IDEMPOTENCY_KEY_REUSED` for the same key with a different
  digest.
- **REL-04** (`doc/2026-11-25T22-30-00Z_design_rel04_lifecycle_mutation_outbox/`):
  one local mutation holds the state CAS on a version, the operation
  row, immutable intent rows, the timeline row and guarded side writes.
  "Atomic" means that a failure injected at any statement leaves the
  store byte-identical; the version column is a CEN-04 admission.
- **PRI-01** (`work/priority.rs`): `check_priority_write`, the ratified
  profile with `expedite = 15` and `next-up = 25`, and the protected
  set as an owner-only write.
- **AUTH-01** (`doc/2026-11-25T22-13-03Z_design_auth01_field_ownership_scoped_authorization/`):
  priority writes are routed to PRI-01. Project fields belong to the
  sole writer, and no intent is written for them here.
- **DST-04** (`doc/2026-11-25T22-17-05Z_design_dst04_priority_label_intake/`):
  label events arrive as priority commands, and Fleet's own label writes
  are echoes.

## Verified state on main

Read at `origin/main` on 2026-11-25:

- **The live write.** `turso_queue/lifecycle.rs` `verb_reprioritize`
  sends one `exec_batch`:
  - `BEGIN IMMEDIATE`;
  - an `UPDATE tasks` guarded by `priority = <from>` and a live status;
  - an `events` row of kind `reprioritized` (detail: from, to, category,
    issue, actor, reason, sync), gated on `changes() = 1`;
  - the history row;
  - a `reprioritize_sync_outbox` row with the rendered
    `priority/<category>` label;
  - `COMMIT`.
- **What it lacks.**
  - The guard compares a field value, not a version.
  - No operation row or receipt exists.
  - No profile version, owner policy reference or protected-membership
    check is recorded.
  - The actor is the presented worker id.
  - The pipeline's atomicity under a failing middle statement is
    unproven (REL-04 §3.2).
- **Contracts.** `Command::intent_dto` and `compute_request_digest`
  exist under the domain `fleet.command-intent/v1`. `ReasonCode` has the
  label, ruling and pressure codes, with permitted effects.
- **`work/distribution.rs`** does not exist. DST-06 (#9456) also plans
  code in it (the order DTO and fixture builder).

## 1. Intent

Once Fleet accepts "set #842 to `next-up`", three facts must exist
together or not at all:
- the item's new value at a new version;
- the record of why it was allowed;
- whatever must later be sent out because of it.

If any of the three can exist without the others, a crash leaves a
value nobody can explain, an explanation for a value that never
changed, or a label sent for a decision that was rolled back.

## 2. Ownership

| Concern                                              | Owner                               | This slice                   |
| ---------------------------------------------------- | ----------------------------------- | ---------------------------- |
| Plan of an accepted priority command (pure)          | `work/distribution.rs`              | **owns** (`plan_acceptance`) |
| Decision provenance DTO                              | `work/distribution.rs`              | **owns**                     |
| Broker integration of the plan                       | `turso_queue` (`verb_reprioritize`) | **owns** the priority path   |
| Command, intent DTO and digest                       | `work/contracts.rs` (DST-01)        | read-only consumer           |
| Policy decision                                      | `work/priority.rs` (PRI-01)         | consumer                     |
| Transaction mechanism, version guard and intent rows | REL-04                              | consumer                     |
| Operation row, receipt and replay                    | REL-03                              | consumer                     |
| Version column and any persisted field               | CEN-04 §5 and the store owner       | waits on it                  |
| Order DTO and fixtures in the same file              | DST-06                              | shares the file; §9          |

No new table, queue, service or client. The plan is pure and the
integration reuses the existing verb.

## 3. What one acceptance writes

An accepted priority command is one REL-04 local mutation. For a
command that changes a value, it contains, in order:

1. **State CAS** on `tasks` keyed on the work version the command
   expects (`expected_work_version`). It sets the new priority, the new
   version (the old one plus 1) and `last_operation_id`. It does not
   compare `priority = <from>`.
2. **Operation row** (REL-03 §5.1), with the identity block, including
   the `request_digest`, and status `pending`, or `confirmed` when there
   is no remote effect.
3. **Decision provenance** (§4) as the timeline row: an `events` row of
   kind `priority.accepted` whose detail is the typed provenance DTO.
   `events` is append-only, so the row is immutable without a new table.
4. **History row**, as today.
5. **Operation intent rows**, one per remote effect, fully rendered at
   acceptance. For priority, that means at most one label mirror: today
   a `reprioritize_sync_outbox` row, later a REL-04 intent row carrying
   the same payload. The label is rendered once, from the new value,
   inside this transaction.

Writes 2 to 5 are guarded on the new version and this
`operation_id` (REL-04 §4.2), so a lost CAS writes nothing.

| Decision from PRI-01 and the reason code                   | Writes            | Outcome     |
| ---------------------------------------------------------- | ----------------- | ----------- |
| base change allowed                                        | 1–5               | `accepted`  |
| hint only (`label_removed_needs_triage`, `label_conflict`) | 1 (hint), 2, 3, 4 | `accepted`  |
| value already held                                         | 2, 3              | `no_change` |
| refused (protected, stale policy, unauthorized)            | none              | refused     |
| `VERSION_CONFLICT`                                         | none              | `conflict`  |

A hint is a CAS too: it bumps the version and sets the item's triage
marker, so the per-item order holds (REL-04 §3.1). A refusal writes
nothing, not even the operation row, because REL-03 stores no receipt
for a refused command. The caller gets the typed refusal.

**No Project intent.** The Project position is the sole writer's
projection of the accepted value (AUTH-01). Nothing here asks the sole
writer to move.

## 4. Decision provenance

`fleet.priority-decision/v1` is the `events.detail` of the
`priority.accepted` row:

| Field                   | Meaning                                                              |
| ----------------------- | -------------------------------------------------------------------- |
| `operation_id`          | REL-03 identity                                                      |
| `work`                  | canonical `WorkRef`                                                  |
| `before`, `after`       | `{value, band, work_version}`                                        |
| `effect`                | `base`, `derived` or `hint` (the `FieldEffectKind` that was allowed) |
| `reason`                | the command's reason code and evidence refs                          |
| `profile`               | profile id, profile version, taxonomy version, label-mapping version |
| `owner_policy`          | `{ref_supplied, ref_verified, protected, membership_version, check}` |
| `principal_ref`         | the authenticated principal that was authorized                      |
| `actor_ref`             | the command's audit field, verbatim; never used for authority        |
| `source`                | `source_ref`, `source_event_ref`, first `delivery_id`                |
| `observed_snapshot_ref` | what the producer read; may be stale                                 |
| `request_digest`        | the command's digest, copied for joining                             |

`owner_policy.check` is `in_transaction` or `pre_transaction` (§5). It
records how the check was actually done, so a reader is never told
more than happened.

No free text from the command goes into the DTO except the evidence
refs, which are references. A private note is never copied in (AUTH-01
§6).

## 5. Binding the owner policy

The issue's second oracle asks that the accepted intent bind
`owner_policy_ref`. Two separate things must hold.

**The digest binds it.** `owner_policy_ref` is in the intent DTO, so two
commands that differ only in it have different digests. The same key
under the same principal with a different `owner_policy_ref` is
REL-03's `IDEMPOTENCY_KEY_REUSED`. It is never replayed as the earlier
receipt, and it is never accepted as a new act under the old key.

**The acceptance checks it.** The plan calls PRI-01 with the supplied
ref and the current policy and membership. A stale ref is
`OWNER_POLICY_VERSION_CONFLICT`, and a missing policy fails closed. For
the check to be atomic with the CAS, the policy version and the
membership version must be read in the same transaction. That needs a
stored policy version, and there is none on main. Until there is:

- the check runs immediately before `BEGIN`, and the DTO records
  `check: "pre_transaction"`;
- the CAS still fails on a moved work version, so a membership change
  that bumps the item's version is caught;
- a policy change that does not touch the item is the disclosed gap.
  The receipt does not claim atomic policy binding.

When the policy store has a version, the CAS gains
`AND EXISTS (SELECT 1 FROM <policy> WHERE version = ?)`, and the DTO
records `in_transaction`. That is a change to the store owner's schema,
behind CEN-04.

## 6. The pure plan

`work/distribution.rs` gains one pure function that the broker calls
before it opens the transaction:

```text
plan_acceptance(
  command:   &Command,                   // validated; digest verified
  principal: &AuthenticatedPrincipal,
  row:       &WorkRow,                   // value, band, version, status
  profile:   &PriorityProfile,
  policy:    Option<&OwnerPriorityPolicy>,
  membership: Option<&ProtectedMembership>,
) -> Result<AcceptancePlan, DistributionError>

AcceptancePlan {
  cas:        { expected_version, new_version, new_value | hint }
  decision:   PriorityDecisionV1          // §4
  intents:    Vec<RenderedIntent>         // §3 step 5; empty if no change
  outcome:    Accepted | NoChange
}
```

It reads nothing and writes nothing. The broker turns the plan into the
§3 statements and runs them through REL-04's transaction mechanism. A
`dry_run` command returns the plan's would-be result and runs no
statement.

## 7. Errors

| Code (as spelled on main)                    | From                 | Writes |
| -------------------------------------------- | -------------------- | ------ |
| `invalid_schema`, `unsupported_version`      | DST-01 validation    | none   |
| `unauthorized_input`                         | DST-01 / DSP-01      | none   |
| `OWNER_POLICY_PROTECTED`                     | PRI-01               | none   |
| `OWNER_POLICY_VERSION_CONFLICT`              | PRI-01               | none   |
| `owner_policy_unavailable`                   | PRI-01, fail closed  | none   |
| `owner_policy_ref_missing`                   | DST-01               | none   |
| `off_profile_value`, `unknown_priority_name` | PRI-01 profile       | none   |
| `effect_not_permitted`                       | reason-code registry | none   |
| `VERSION_CONFLICT`                           | REL-04 CAS           | none   |
| `IDEMPOTENCY_KEY_REUSED`                     | REL-03               | none   |
| `work_terminal` (proposed)                   | row status           | none   |

Only `work_terminal` is new; the legacy verb returns that case as a
plain error string today. Every refusal is decided before or by the CAS,
so it leaves nothing behind.

## 8. Compatibility

`verb_reprioritize` keeps its wire shape. Until the version column is
admitted, the legacy path stays as it is, field-value guard included.
The DST-03 path is selected per call by the presence of
`expected_work_version` and an operation id, and refuses when the
version column does not exist, rather than falling back to the
field-value guard. Rollback disables the DST-03 path. Rows it wrote
stay, and the legacy path is untouched. The two paths never run on the
same call.

The `reprioritized` events the legacy path writes are not relabelled as
`priority.accepted`. Old acceptances stay legacy, without fabricated
provenance (REL-03 §8).

## 9. Tests and gates

First failing oracles (from the issue), in `work/distribution.rs`:

1. `pmo_distribution_atomic_intent_revision`. On a fake transaction
   store:
   - a failure injected at each statement position leaves the store
     byte-identical;
   - success leaves exactly one version step, one operation row, one
     `priority.accepted` row whose DTO matches the plan, and the
     rendered label intent;
   - a stale `expected_work_version` writes nothing and returns
     `VERSION_CONFLICT`.
2. `pmo_distribution_intent_digest_binds_owner_policy_ref`:
   - two commands that differ only in `owner_policy_ref` have different
     digests;
   - commands that differ only in `delivery_id`, `operation_id` or
     `dry_run` have the same digest;
   - reusing a key with a different ref is `IDEMPOTENCY_KEY_REUSED`;
   - the accepted DTO records the supplied and the verified ref.

Further oracles:

3. `pmo_distribution_refusal_writes_nothing`, for each §7 code.
4. `pmo_distribution_no_change_writes_receipt_and_decision_only`.
5. `pmo_distribution_hint_bumps_version_without_value_change`.
6. `pmo_distribution_label_intent_rendered_once_at_acceptance`.
7. `pmo_distribution_pre_transaction_policy_check_is_disclosed`.
8. `pmo_distribution_dry_run_runs_no_statement`.

**Done command:** list both issue names with `cargo test --manifest-path
pkgs/fleet/Cargo.toml -p fleet-queue --lib --features
client,queue-broker -- --list`, then run the `pmo_distribution_`
filter. Both run remotely.

**Gates.**
- Oracles 2, 4–8 and the pure plan can land first, because they are
  pure.
- The broker integration waits on two things: the CEN-04 admission of
  the work-version column, and REL-04's §10 atomicity proof against the
  real store.
- **Shared file:** DST-06 adds its order DTO to `distribution.rs`.
  The two slices own separate items in it. Whichever lands second
  rebases onto the first, and neither edits the other's items.

## 10. Requirement trace (design-level)

| Issue requirement                   | Where                                           |
| ----------------------------------- | ----------------------------------------------- |
| Records work version                | §3 step 1                                       |
| Records decision provenance         | §3 step 3, §4                                   |
| Records required operation intent   | §3 step 5                                       |
| Through the existing broker         | §6, §8                                          |
| No new table before the schema gate | §3, §5, §8                                      |
| Negative: stale input               | §3, §7                                          |
| Negative: duplicate / replay        | §5                                              |
| Negative: unauthorized actor        | §7                                              |
| Negative: unknown version           | §7                                              |
| Negative: partial remote effect     | §3 (intent rendered once; REL-04 owns delivery) |
| No false verified result            | §4 `check`, §5                                  |

## 11. Non-goals

- No version column, policy store or other schema change in this slice.
- No change to label delivery (REL-04, REL-06) or to the Project
  writer.
- No priority policy; PRI-01 decides.
- No migration of legacy `reprioritized` events.
