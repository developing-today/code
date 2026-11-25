# REL-10M — Migration, backfill, and rollback validation

Design record for
[#9489](https://github.com/developing-today/code/issues/9489)
(`pmo-9196:REL-10M`), the migration half of slice REL-10 in
`components/9209-leases-fencing-outbox-direct-completion.md`. The
hosted half is REL-10H (#9488) and is not covered here.

The slice's first failing oracle is
`pmo_rel_migration_rollback_preserves_remote_effect_receipt`, which
"simulates remote success then disable/rollback/restart". Its
negatives are mixed binaries, pending legacy intent, remote success
during rollback, and partial schema. This record fixes the model that
oracle runs, what each negative must do, and which evidence belongs
to which stage.

This is a design record. It authorizes no code, schema, migration,
rollout or live run. The slice says the "actual DB migration owner
must also supply/run its named upgrade/rollback integration command
before release". This record names no such command.

## Governing decisions consumed

- **Slice text and split refinement**, quoted: "Validate additive
  migration, unknown legacy state, mixed-version behavior, rollback
  after remote effect, and retained pending receipts only after #9198
  names the actual migration owner/command." Rollback: "Disable only
  the new capability; preserve pending intents, artifacts, existing
  state, and remote effects."
- **CEN-04** (merged): §3.9 "No persisted operation or receipt table
  exists on main", disposition "absent — gated"; `creation_outbox` is
  a scoped creation producer, "generalizing it by implication is
  refused"; §5 field-admission gate; §7 move shape "expand → backfill
  → dual-read with the old reader authoritative until parity is
  evidenced → cutover → contract" and "Rollback disables the new
  capability only; it does not reset state, delete history, or unwind
  remote effects that already landed."
- **REL-02** (merged): fencing is switched on per queue with a
  recorded `fence_enabled_at`; claims from before it are legacy
  claims; rollback disables fencing and keeps the column; re-enable
  bumps every generation so pre-rollback tokens are dead.
- **REL-03** (merged): the receipt's identity block is immutable and
  byte-identical on every read; a replay writes nothing and never
  sends; legacy records are `legacy_unknown`, never backfilled, and a
  matching presentation returns `uncertain` with `LEGACY_OPERATION`.
- **REL-04** (merged): the intent block is written once; `unknown` is
  resolved only by REL-06 read-back, never by retrying; "No transition
  leaves `confirmed`"; a lapsed send lease turns `delivering` into
  `unknown`.
- **REL-06** (merged): read-back adapter and preconditions.
- **REL-07** (merged): completion phases and recovery; rollback
  "stop preparing new completions. Open operations stay in
  `completing` with every receipt and remain inspectable and
  recoverable; nothing is reset or deleted."
- **REL-11** (merged): operator procedures, including "reopen or
  unmerge to simulate rollback" listed as forbidden.

## Verified state on `origin/main` `63e741c1`

- `work/operations.rs` has `AcceptanceLedger`: an in-memory map from
  idempotency key to `Receipt` plus an effect list. Nothing persists
  it, so no operation or receipt store exists to migrate today
  (CEN-04 §3.9).
- `work/completion.rs` has the completion receipt and REL-09's
  activity classes (merged in #15308).
- The broker's schema path is `TursoQueue::ensure_schema`
  (`turso_queue/core.rs:868`): it runs `SCHEMA`'s
  `CREATE TABLE IF NOT EXISTS` statements, then adds missing columns
  one by one after reading `pragma_table_info`, with "No backfill,
  nothing destructive". That is the existing additive path. Whether a
  REL store lands through it, and under which command, is the
  migration owner's call.
- Three landed tables already hold pending remote intent outside any
  REL design:
  - `creation_outbox` (scoped content creation, `doc/fleet/creation-outbox.md`);
  - `reprioritize_sync_outbox` (label sync drained by the broker);
  - `merge_intents` (`merge_intent.rs`: a replay pass that pushes,
    opens and **merges** stacked PRs).
- Claim SQL selects named statuses (`turso_queue/claim.rs`, for
  example `status IN ('queued', 'parked', 'handed_off')`), so a task
  in a status it does not name is not claimed by today's binaries.
  REL-07's `completing` status is not on main.

## 1. Intent

REL-02 to REL-07 each say "rollback is disable-only". REL-10M checks
that the rule still holds once a remote effect has happened, the
process restarts, and two binaries run at once. The question the
oracle answers is: after a remote merge succeeded and the capability
was turned off, does Fleet still know that the merge happened, and
does it never do it twice?

## 2. Ownership

| Concern                                                                                           | Owner                                                                                                     |
| ------------------------------------------------------------------------------------------------- | --------------------------------------------------------------------------------------------------------- |
| Migration and rollback model and its oracles, in `operations.rs` and `completion.rs` test modules | **REL-10M (this)**                                                                                        |
| Operation, receipt, intent and attempt-log storage; the schema step and its command               | #9198 store owner (the queue broker), through CEN-04 §5                                                   |
| Receipt identity, legacy rule                                                                     | REL-03                                                                                                    |
| Intent rows, transitions, send lease                                                              | REL-04                                                                                                    |
| Read-back                                                                                         | REL-06                                                                                                    |
| Completion phases, `completing` status                                                            | REL-07, with #9178 and the store owner for the status                                                     |
| Fence enable point and re-enable bump                                                             | REL-02                                                                                                    |
| Hosted parity                                                                                     | REL-10H (#9488)                                                                                           |
| The on/off switch's home                                                                          | the implementation issue, in the broker's existing config surface (as REL-02 does for `fence_enabled_at`) |

`operations.rs` and `completion.rs` are shared files. The issue's
edges put REL-10M after REL-05 (`operations.rs`) and REL-09
(`completion.rs`).

## 3. States and the switch

### 3.1 Store states

| State      | Holds                                                                   | Sends?                                     |
| ---------- | ----------------------------------------------------------------------- | ------------------------------------------ |
| `legacy`   | no operation store; today's tables and scoped outboxes only             | only the legacy producers, in their scope  |
| `enabled`  | operation, receipt, intent and attempt-log rows per REL-03/REL-04       | the REL-04 sender and the legacy producers |
| `disabled` | exactly what `enabled` left                                             | legacy producers only                      |
| `partial`  | some of the operation store's tables or columns exist and others do not | legacy producers only                      |

### 3.2 What "off" does

Checked once at command start and once before every send attempt:

- no new operation is accepted, so no new receipt or intent is
  written;
- no intent is sent, retried or moved to `delivering`;
- every receipt, intent, attempt-log row and `remote_ref` stays as it
  is, byte for byte;
- no status is reset: `unknown` stays `unknown`, `delivering` stays
  `delivering`, `completing` stays `completing`;
- reads still work: `inspect` and a same-key replay render from the
  receipt (REL-03 §5.2) and send nothing;
- REL-06 read-back may still run on demand to record evidence,
  because recording what already happened on the remote is not a new
  effect. It moves `unknown` only to `confirmed`, `conflict` or
  `retryable_failure` as REL-04 allows, and a `retryable_failure`
  is not resent while off.

A command that would accept or send while off is refused before any
remote request (proposed code `OPERATIONS_DISABLED`, uppercase as
REL-03 §7 sets for these codes).

### 3.3 Re-enable

- Every intent in `delivering` whose send lease has lapsed goes to
  `unknown` (REL-04 §6), and every `unknown` goes to read-back before
  any send. Re-enable never resends on the strength of a stored
  status alone.
- A work in `completing` is reconciled before anything else is
  prepared for it (REL-07 §6).
- If fencing was also rolled back, REL-02's re-enable bump applies
  first, so a token from before the rollback cannot send.

## 4. The first oracle, step by step

`pmo_rel_migration_rollback_preserves_remote_effect_receipt`, in
`operations.rs`'s test module, over the REL-03/REL-04 in-memory store
fakes and REL-06's fake remote:

1. **Enabled.** Accept a completion command whose required effects
   are a squash merge of PR 7 at head `H` and a close of issue 3.
   Assert one receipt, two `pending` intents, the work `completing`.
2. **Remote success, lost response.** The sender sends the merge; the
   fake remote merges PR 7 and drops the response. Assert the merge
   intent is `unknown` and the fake's merge count is 1.
3. **Disable.** Snapshot every stored byte. Run the sender. Assert
   it sends nothing and the bytes are unchanged.
4. **Restart.** Drop every in-memory value and reload from the fake
   store. Assert the receipt's identity block is byte-identical to
   step 1, the merge intent is `unknown`, the close intent `pending`,
   the work `completing`.
5. **Replay while off.** Present the same key and intent. Assert
   outcome `uncertain`, the stored `operation_id`, zero sends, zero
   writes.
6. **Re-enable.** Run recovery. Assert read-back finds PR 7 merged at
   the expected commit, the merge intent becomes `confirmed`, the
   close is sent once, the operation becomes `confirmed`, and the
   fake's merge count is still 1.

The test is listed with `-- --list` before it runs and must fail
before implementation. A zero-test filter passing is not acceptance.

## 5. The four negatives

### 5.1 Mixed binaries

During expand and during a rollback window, a binary that does not
know the operation store may run against the same database.

- The new tables and columns are additive (CEN-04 §7); an old binary
  ignores them, as `ensure_schema`'s added columns are ignored today.
- An old binary must never act on a work with an open operation. Two
  facts make that hold, and both are gates, not assumptions:
  - today's claim SQL selects named statuses, so a `completing` task
    is not claimed by an old binary (verified on main for the claim
    paths above);
  - the old one-step completion path (`verb_complete` through
    `gate_completion`) must refuse a task whose status it does not
    know. This is **not verified** here. Before `completing` is
    stored, the implementation shows it with a test against the
    oldest binary allowed to run, or the store owner adds a guard.
- A new binary never infers anything from an old binary's silence.
  An operation the old binary cannot see is still the new binary's
  to reconcile at re-enable.

### 5.2 Pending legacy intent

At upgrade, `creation_outbox`, `reprioritize_sync_outbox` and
`merge_intents` may hold pending rows.

- **No backfill.** No legacy row is converted into an operation,
  receipt or intent, and no digest, principal or status is guessed
  (REL-03 §8). Each legacy drainer keeps running in its own scope.
- **No double effect on one target.** Preparing an operation whose
  effect kind and target match a pending legacy row is refused
  (proposed `LEGACY_INTENT_PENDING`) until that row is resolved by its
  own drainer. The case on main is a `merge_intents` replay and a
  REL-07 merge of the same PR. Label sync rows and creation rows are
  checked the same way for any effect kind that overlaps them.
- A presentation that matches a legacy key returns `uncertain` with
  `LEGACY_OPERATION`, as REL-03 §8 already says.

### 5.3 Remote success during rollback

An attempt is in flight when the switch flips.

- If its response arrives, it is recorded (`confirmed` after
  read-back, or `unknown`). Recording it is not a new send.
- If the process stops first, the intent stays `delivering` with its
  send lease. No sender runs while off, so nothing touches it. At
  re-enable the lapsed lease makes it `unknown`, and read-back
  decides (§3.3).
- In either case the fake's effect count is checked: one remote
  merge, never two.

### 5.4 Partial schema

Some of the operation store exists (for example the receipt table
landed and the intent table did not, or the attempt-log column is
missing).

- The store reports `partial` and the capability stays off for that
  store: no acceptance, no send (proposed
  `OPERATION_STORE_PARTIAL`). It does not create the missing parts
  itself, outside the owner's migration step.
- Existing receipts and intents, if any, stay readable; nothing is
  dropped to "restore consistency".
- The legacy path stays authoritative, per CEN-04 §7's "old reader
  authoritative".

## 6. Proposed codes

None of these exist on main. Spelling follows REL-03 §7's uppercase
error codes; the names go to whoever owns that list.

| Code                      | When                                                         |
| ------------------------- | ------------------------------------------------------------ |
| `OPERATIONS_DISABLED`     | accept or send while the capability is off                   |
| `LEGACY_INTENT_PENDING`   | a pending legacy row targets the same effect kind and target |
| `OPERATION_STORE_PARTIAL` | the store is `partial`                                       |

## 7. Acceptance stages, reported separately

| Stage              | Evidence                                                                                   | Who                      |
| ------------------ | ------------------------------------------------------------------------------------------ | ------------------------ |
| code               | the §8 oracles listed, then run with the done command, on a recorded commit                | implementation issue     |
| DB migration       | the owner's named upgrade and rollback integration command and its output; nothing guessed | #9198 store owner        |
| staging            | §4's timeline against a store copy, with the old binary present for §5.1                   | implementation and owner |
| deployed telemetry | REL-11 §6 events for `unknown`, `conflict` and refusals during a canary queue              | deployment owner         |
| live readback      | one authenticated read-back of a real effect, separately authorized                        | separately authorized    |
| hosted parity      | REL-10H                                                                                    | REL-10H                  |

Each record states the stage, commit, environment, command as cited,
result, and what it does not claim. None implies another.

## 8. Oracles

`pmo_rel_` prefix, in the `operations.rs` and `completion.rs` test
modules, run with
`cargo test --manifest-path pkgs/fleet/Cargo.toml -p fleet-queue --lib --features client,queue-broker <name>`
after `-- --list` shows them.

| Test                                                         | Asserts                                                                           |
| ------------------------------------------------------------ | --------------------------------------------------------------------------------- |
| `pmo_rel_migration_rollback_preserves_remote_effect_receipt` | §4, the slice's first oracle                                                      |
| `pmo_rel_disable_writes_and_sends_nothing`                   | while off: refusals before any request; store bytes unchanged                     |
| `pmo_rel_disable_never_resets_status`                        | `unknown`, `delivering` and `completing` survive disable and restart as they were |
| `pmo_rel_reenable_reads_back_before_any_send`                | §3.3; a lapsed `delivering` becomes `unknown` and is read back                    |
| `pmo_rel_inflight_success_during_rollback_counts_once`       | §5.3, both branches; remote effect count 1                                        |
| `pmo_rel_legacy_rows_are_never_backfilled`                   | §5.2; legacy tables unchanged, no operation rows created from them                |
| `pmo_rel_legacy_merge_intent_blocks_same_pr_merge`           | §5.2; `LEGACY_INTENT_PENDING` for a pending `merge_intents` row on the same PR    |
| `pmo_rel_partial_store_stays_disabled`                       | §5.4 for a missing table and a missing column                                     |
| `pmo_rel_old_reader_sees_no_new_claimable_status`            | §5.1 at model level: a `completing` row is outside every claim status set         |

The old-binary completion check of §5.1 is a staging-stage test, not
a unit oracle, because it needs the old binary.

## 9. Gates and sequencing

- REL-05 and REL-09 land in the shared files first (the issue's
  edges). REL-02, REL-03, REL-04 and REL-07 code must exist for the
  oracles to have something to drive.
- The store owner names the storage step and its command, and admits
  each table and column through CEN-04 §5.
- `completing` is stored only after §5.1's old-binary check passes.
- QTI stays unlocated (#9241); no overlapping persisted field moves.
- No spelling of queue status, identity, Project writer or `fleet gh`
  changes; `expedite=15` and `next-up=25` are untouched.

## 10. Requirement trace (design-level)

| Requirement | Where                                                                                                                      |
| ----------- | -------------------------------------------------------------------------------------------------------------------------- |
| C-R136      | §4 steps 4 and 6: restart reloads the stored receipt and recovery resumes from it                                          |
| C-R137      | §3.1 and §5.1: the operation store, the claim and the remote state stay separate facts                                     |
| C-R142      | §5.3 and §4 step 2: a remote success is never assumed from, or folded into, a local commit                                 |
| O-R011      | §3.2 and §4 step 4 only: an accepted request stays durable through disable and restart; do-now dispatch itself is DSP-08's |
| P-R008      | §2: no new store, service or scheduler; the broker owns storage                                                            |
| P-R024      | §6 and §3.2: stable error codes; replay while off renders `uncertain`                                                      |
| P-R037      | §4: interrupted work resumes from its stored receipt without a second effect                                               |

Design-level only. Each row still needs the implementation and stage
evidence of §7.

## 11. Non-goals

- No schema, table, column or migration command.
- No conversion of legacy outbox or merge-intent rows.
- No reopen, unmerge or other remote undo as part of rollback.
- No hosted run (REL-10H).
- No change to an earlier REL record's contract; where this record
  restates one, the earlier record governs.
