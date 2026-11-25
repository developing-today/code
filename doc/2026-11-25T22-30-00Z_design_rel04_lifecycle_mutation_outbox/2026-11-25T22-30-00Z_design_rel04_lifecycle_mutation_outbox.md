# REL-04 — General lifecycle mutation outbox in the existing authority store

Issue: [#9514](https://github.com/developing-today/code/issues/9514)
(slice `REL-04` of [#9209](https://github.com/developing-today/code/issues/9209),
program [#9196](https://github.com/developing-today/code/issues/9196)).
Status: **design proposal — not an authorization to implement.**
This record elaborates the REL-04 row of the merged design package
(`doc/2026-11-13T03-20-24Z_design_pm-stack-fleet-orchestration/`,
PR #9225). The row's goal is "atomic local state+journal+intent
transaction, immutable intent payload, attempt log/status/event
order", with first oracle
`pmo_rel_state_and_intent_commit_or_rollback_together` ("inject a
failure after each local write"), and negatives "intent absent when
state commits, duplicate key, reordered per-target op, DB busy". This
record _decides_ what one local mutation transaction contains, how
it is made atomic on the store Fleet actually has, what an intent
row holds and which of its fields may never change, the operation
status transition table, the attempt log, the ordering rule per
target, and what the landed `creation_outbox` stays. It authorizes
no code, schema, migration, deployment or live acceptance; those
remain separately gated stages (§11).

Governing decisions consumed, not re-opened:

- **Component #9209 §4**
  (`components/9209-leases-fencing-outbox-direct-completion.md`):
  the suggested operation states `pending`, `delivering`,
  `confirmed`, `retryable_failure`, `conflict`, `terminal_failure`,
  `unknown`; a transition table that forbids regressing from
  `confirmed`; per-work-item causal sequence for operations on the
  same target. §6: never post a second comment solely because the
  first response timed out.
- **REL-03 record** (#9513,
  `doc/2026-11-25T20-47-34Z_design_rel03_idempotency_identity_receipts/`,
  merged in #15240): the scope tuple
  `(principal_ref, authority_id, idempotency_key)`, the lookup order,
  the receipt identity block, and the status→outcome mapping of its
  §5.2. Its §2 assigns the status transition table and outbox
  atomicity to REL-04; this record fills that slot and does not
  change the mapping.
- **CEN-04 record** (#9434,
  `doc/2026-11-23T13-51-45Z_design_cen04_schema_owner_migration_compatibility/`):
  §3.6 (`events` is the broker timeline, **not** the #9208 journal,
  and no journal table exists on main), §3.9 (operation/receipt
  persistence is `absent — gated`; `creation_outbox` is a scoped
  creation producer, not the general outbox), §5 (no persisted field
  without an admitted row) and §7 (disable-only rollback).
- **REL-06 record** (#9516,
  `doc/2026-11-23T12-20-33Z_design_rel06-github-preconditions-readback/`):
  remote preconditions, effect-specific read-back and resolution of
  `unknown` belong to REL-06. REL-04 records evidence; it never
  decides that a remote effect happened.
- **Delivery plan, storage row**: typed storage lands in the
  existing authority store, behind a "shared transaction proof",
  with the old reader authoritative on rollback.

## Verified state at `origin/main` `b483ca1c`

The REL-04 row names "a proven DB transaction contract" as a hard
blocker. Reading the broker's Turso backend shows why: the contract
is not proven today, and two of its gaps are concrete.

| Surface | What exists | What it means for REL-04 |
| --- | --- | --- |
| `turso_queue/core.rs` `pipeline` (`:705`) and `exec_batch` (`:834`) | One HTTP POST to Hrana `/v2/pipeline` carrying one `execute` request per statement plus `close`. The client walks the results and returns `Err` at the **first** non-`ok` result. | Callers build transactions as `[BEGIN IMMEDIATE, …, COMMIT]` inside one pipeline. The client learns of a failed statement only after the server has processed the whole request list, including the trailing `COMMIT`. Nothing in the request makes later statements conditional on earlier ones, so "the pipeline returned `Err`" does not prove "nothing committed". No `ROLLBACK` is ever sent on this path, and Hrana `batch` step conditions are not used anywhere in `fleet-queue`. |
| Guarded writes, e.g. `links.rs` `GUARDED_DEP_INSERT_SQL` (`:567`) | Follow-on inserts carry `WHERE EXISTS (SELECT 1 FROM tasks WHERE id = ? AND <expected post-state>)`, so a lost state CAS turns them into no-ops inside the same transaction. | This is the right in-pipeline pattern, and REL-04 adopts it (§4.2). It keys on a post-state that two writers can both produce, so it is a weak guard. |
| `turso_queue/admin.rs` `block_task` (`:87-140`) | State CAS `UPDATE tasks … WHERE id = ? AND status = 'claimed' AND worker_id = ?`, guarded dep writes, then an **unguarded** `DELETE FROM leases WHERE task_id = ?`, then `COMMIT`. Afterwards it checks `results[1].affected == 0` and returns "claim changed before the block committed". | When the CAS loses, the unguarded `DELETE` has already committed, and it deletes whatever lease the new owner holds (`leases` is keyed by `task_id` alone, `core.rs:1083`). The post-commit affected-row check reports the race but cannot undo it. This is the "guard does not stop the statements after it" gap in one place. |
| `turso_queue/core.rs` `event` (`:1684`) | Writes the `events` row (and, for a status transition, a history row) in its **own** pipeline. Callers such as `block_task` call it after their state transaction committed. | State and timeline are two transactions today. A crash or a store error between them leaves a committed state change with no event. That is exactly the "intent absent when state commits" negative, already present for the timeline. |
| `creation_outbox.rs` + `turso_queue/stacks_outbox.rs` | One scoped producer: dedupe by `(repo, dedup_key)`, `payload_hash`, states `pending/leased/uncertain/done/blocked`, a single serial drainer with a lease, a persisted rate-limit circuit, and `github_number`/`github_url` read-back. Approved as a bounded pilot. | Useful prior art for payload hashing, serial draining and `uncertain`. CEN-04 §3.9 refuses treating it as the general outbox, and the REL-04 row says the pilot stays isolated. |
| `work/operations.rs` DSP-01 `AcceptanceLedger` | In-memory receipts and effects; REL-03 §7 states its delta. | No persistence. REL-04's rows are where REL-03's receipt is stored. |
| Operation, intent or attempt tables | None (CEN-04 §3.9). | Everything in §5 is `absent — gated` until admitted through CEN-04 §5. |

## 1. Intent

A Fleet command that changes local state and asks for a remote effect
commits both facts together or neither. If the local state change is
visible, the intent to perform its remote effect is durable, its
payload can never change afterwards, and every attempt to deliver it
is recorded in order. Operations aimed at the same target are applied
in the order they were accepted. A crash, a store error or a busy
store at any point leaves either the old state with no intent, or the
new state with its intent pending. Never one without the other.

## 2. Ownership: who writes what

| Concern | Owner |
| --- | --- |
| The local mutation transaction, intent immutability, status transition table, attempt log, per-target order (this record) | **REL-04** |
| Idempotency scope, lookup order, receipt identity block, status→outcome rendering | REL-03 (#9513, record merged) |
| Remote preconditions, read-back, resolving `unknown` | REL-06 (#9516) |
| Stale relay acknowledgements that would regress an operation | REL-05, consuming §6 of this record (`pmo_rel_stale_relay_ack_cannot_regress_operation`) |
| Claim leases and fence generation | REL-01 (#9512) and #9487 |
| The #9208 journal and its home | JRN-01/02, through CEN-04 §5 |
| Persisted tables and columns, migration, rollback of storage | REL-07 + the #9198 store owner, through CEN-04 §5 |
| Operator inspection of stuck, `unknown` or `conflict` operations | REL-08 |
| `creation_outbox` and its drainer | the landed creation pilot, unchanged |

REL-04 adds no database, queue, relay, service, scheduler or GitHub
client. Its storage is rows in the existing broker store.

## 3. The transaction contract

### 3.1 What one local mutation contains

A **local mutation** is the set of writes one accepted command makes
locally. It is committed as one transaction, and it contains, in this
order:

1. **The state CAS.** One conditional write on the target row, keyed
   on the expected version the command carries
   (`Command.expected_work_version`, and the lease generation where
   REL-01 applies). It stamps the row with the new version and the
   `operation_id` that changed it.
2. **The operation row** (REL-03's receipt, §5.1 there) with status
   `pending`, inserted only if no row holds the same scope tuple.
3. **One intent row per remote effect**, immutable once written (§5).
4. **The timeline row** in `events`, plus the history row when the
   kind is a status transition (what `event()` writes today).
5. **The journal entry**, when JRN persistence exists. Until then
   the slot is empty, and nothing in `events` is relabelled as the
   journal (CEN-04 §3.6).
6. **Any guarded side writes** the verb already makes (deps,
   leases), each guarded per §4.2.

A command with no remote effect still commits 1, 2, 4 and 6
together. A command whose only effect is remote still commits a
state CAS on its work item, so the per-target order (§7) has a
version to hang on.

### 3.2 What "atomic" has to mean on this store

The transaction is atomic when, for **every** statement position
_k_ in the transaction, a failure injected at _k_ leaves the store
byte-identical, in the tables the transaction touches, to the state
before it began. "The client got `Err`" is not the test. The store's
contents after the failure are.

On a Hrana pipeline that sends `BEGIN IMMEDIATE … COMMIT` as
independent `execute` requests, that property cannot be assumed. A
failing middle statement in SQLite does not, in general, roll the
transaction back, and the trailing `COMMIT` is still in the request
list. So REL-04 requires one of these mechanisms, and requires the
§10 proof oracle to pass against the real store before either is
relied on:

- **(a) Conditional batch.** Send the transaction as one Hrana
  `batch` whose every step after `BEGIN` carries an `ok` condition on
  the step before it, whose `COMMIT` is conditional on the last write
  succeeding, and whose final `ROLLBACK` step runs when the `COMMIT`
  did not. This keeps one round trip.
- **(b) Interactive stream.** Open a stream, run `BEGIN IMMEDIATE`,
  run each statement and check its result client-side, then send
  `COMMIT` or `ROLLBACK`. This costs one round trip per statement and
  holds the write lock across them.

The record prefers (a). Picking between them is part of the proof:
whichever passes the oracle on the hosted store is the contract,
and `exec_batch` stops being used for multi-write transactions on the
REL-04 path. Fixing existing callers (`block_task` and the rest) is
not REL-04's change; §12 lists it as a follow-up the proof unblocks.

### 3.3 A lost CAS aborts the whole transaction

A state CAS that matches zero rows is not a store error, so neither
mechanism above aborts on it by itself. REL-04 closes that in SQL,
inside the transaction:

- With (a), every later write is guarded (§4.2), so a lost CAS turns
  every later write into a no-op, and the transaction commits nothing
  of substance.
- With (b), the client sees `affected == 0` on the CAS and sends
  `ROLLBACK` before any other write.

In both cases the caller gets a typed `StateConflict`, never a
partial commit followed by an error string, which is what
`block_task` returns today.

## 4. Guards

### 4.1 Expected version, not expected post-state

The state CAS compares the row's current version with the version
the command expected. It never compares field values that a
different writer could also produce. The version column is a CEN-04
§5 admission (owner: the queue broker, with REL-01's fence
generation where leases apply). Until it exists, REL-04 is not
implementable, and this record does not substitute a field-value
guard for it.

### 4.2 Guarding follow-on writes

Every write after the CAS carries
`WHERE EXISTS (SELECT 1 FROM <target> WHERE id = ? AND version = ?
AND last_operation_id = ?)`, binding the **new** version and this
operation's id. Only the transaction that won the CAS can satisfy it,
so a lost CAS makes every later write a no-op even under mechanism
(a). This is `GUARDED_DEP_INSERT_SQL`'s pattern with a guard no other
writer can reproduce. An unguarded follow-on write, such as
`block_task`'s `DELETE FROM leases`, is not allowed on the REL-04
path.

## 5. The intent row

### 5.1 Fields

| Field | Set when | Mutable? |
| --- | --- | --- |
| `intent_id` | the local mutation | never |
| `operation_id` (REL-03 §3.4) | the local mutation | never |
| `target_ref` (e.g. `github:<owner>/<repo>#<n>`, the work item the effect lands on) | the local mutation | never |
| `target_seq` (per-target sequence, §7) | the local mutation | never |
| `effect_kind` (a registered `contracts::Effect` kind) | the local mutation | never |
| `payload` (canonical JSON of what will be sent) | the local mutation | never |
| `payload_hash` (`sha256:` over the canonical payload) | the local mutation | never |
| `preconditions` (REL-06's expected remote state, e.g. head SHA, body hash) | the local mutation | never |
| `status` | the local mutation (`pending`) | only by §6's table |
| `attempts` (count) and `next_attempt_at` | each attempt | yes |
| `remote_ref` (comment id, PR number, URL) and read-back evidence | REL-06 read-back | fill once; never cleared |
| `status_changed_at` | each transition | yes, with `status` |

### 5.2 Immutability

The payload, its hash, the preconditions and the identity fields are
the **intent block**. They are written once in the local mutation and
never updated. A sender recomputes `payload_hash` before every
attempt and refuses to send on a mismatch, moving the intent to
`terminal_failure` with code `INTENT_PAYLOAD_TAMPERED`. A changed
intent is a new command with a new operation, never an edit.

The payload is what will be sent, fully rendered at acceptance.
Rendering it at send time from current state would make the same
intent send different bytes on a retry, which is the duplicate-effect
bug REL-03 and REL-06 exist to prevent.

## 6. Status transition table

The operation's status is derived from its intents: `confirmed` when
every required intent is `confirmed`, otherwise the least advanced
intent status in the order below. REL-03 §5.2 renders outcomes from
it unchanged. The table below is for one intent.

| From | To | Who | Condition |
| --- | --- | --- | --- |
| (none) | `pending` | the local mutation | in the same transaction as the state CAS |
| `pending` | `delivering` | the sender | it holds the target's send lease and this is the lowest unconfirmed `target_seq` (§7) |
| `delivering` | `confirmed` | REL-06 read-back | the effect is observed and verified |
| `delivering` | `retryable_failure` | the sender | a failure that provably happened before the remote accepted anything (connection refused, 429, 5xx with no side effect per REL-06) |
| `delivering` | `unknown` | the sender | a timeout or lost response after the request may have reached the remote |
| `delivering` | `conflict` | the sender | a REL-06 precondition failed |
| `delivering` | `terminal_failure` | the sender | a non-retryable error, attempts exhausted, or `INTENT_PAYLOAD_TAMPERED` |
| `retryable_failure` | `delivering` | the sender | `next_attempt_at` reached, same lease and order rules as `pending` |
| `unknown` | `confirmed` / `retryable_failure` / `conflict` | REL-06 read-back only | read-back shows the effect present, absent and safe to retry, or a precondition now false |
| `conflict`, `terminal_failure` | none | — | terminal. An operator can only start a new operation (REL-08) |
| `confirmed` | none | — | terminal. **No transition leaves `confirmed`**, whatever a late or replayed acknowledgement says |

Rules the table encodes:

- `unknown` is never resolved by retrying. A sender that would retry
  an `unknown` intent stops, because §6 of #9209 forbids a second
  POST on a timeout. Only read-back moves it.
- Every transition is itself a CAS on `(intent_id, status,
  attempts)`, so two senders, or a sender and a stale relay ack,
  cannot both apply. A transition that loses its CAS is dropped, and
  REL-05's stale-ack oracle checks that it never regresses an
  operation.
- Each transition writes its attempt-log row (§8) and its timeline
  row in the same transaction as the status change, under §3's
  mechanism.

## 7. Per-target order

`target_seq` is allocated in the local mutation as one more than the
highest `target_seq` for that `target_ref`, under the same write lock,
and is unique per `target_ref`. A sender may move an intent to
`delivering` only when every lower `target_seq` for that target is
`confirmed`, `conflict` or `terminal_failure`. An intent behind an
`unknown` or `retryable_failure` waits.

That is the "reordered per-target op" negative: a label change
accepted after a comment edit on the same issue is never sent before
it, even when the comment edit is retrying. Different targets are
independent and can be sent concurrently.

A blocked head is visible, not silent: REL-08's inspection lists
targets whose head intent has waited past a threshold, and the
timeline row for each waiting transition names the blocking
`target_seq`.

## 8. Attempt log and event order

Each delivery attempt appends one immutable attempt row:
`(intent_id, attempt_no, started_at, finished_at, from_status,
to_status, error_code, remote_status, remote_ref)`. `attempt_no` is
dense from 1 and unique per intent. The row is written in the same
transaction as the status transition it reports (§6), so the log and
the status can never disagree.

For one operation, the timeline order is fixed: `accepted` (in the
local mutation), then for each attempt `delivering` and its outcome,
then `confirmed`/`conflict`/`terminal_failure`. Every timeline row
carries `operation_id`, `intent_id` and `attempt_no` in `detail`, so
`timeline` can show an operation's history without joining on time.

## 9. Busy store, duplicates and crash points

- **Duplicate key.** The operation row's uniqueness is REL-03's
  scope tuple. A second local mutation with the same scope finds the
  row and follows REL-03's lookup order (replay or conflict). It
  never inserts a second operation or a second set of intents, and
  the state CAS is not attempted.
- **DB busy.** `SQLITE_BUSY` or a store `5xx` before the transaction
  committed is a `retryable` local error. The command is not
  accepted, nothing is visible, and the client retries with the
  **same** idempotency key. Because the retry is a fresh attempt at
  the whole local mutation, a busy store can never leave half of it.
  A busy store during a sender transition leaves the intent where it
  was.
- **Crash between commit and response.** The command is accepted.
  The client's retry with the same key replays the receipt (REL-03
  §4), and the intent is already pending.
- **Crash during delivering.** The sender's lease on the target
  expires, and the next sender treats a `delivering` intent whose
  lease lapsed as `unknown`, never as `pending`. Only read-back moves
  it on.

## 10. Acceptance oracle

Tests a separately scoped implementation must add. Names follow the
`pmo_rel_` convention.

1. **The transaction proof** (blocks everything else):
   `pmo_rel_store_transaction_contract_rolls_back_on_any_failed_step`
   runs against the hosted store, not a mock. For a transaction of
   _n_ writes and each _k_ in 1.._n_, it makes statement _k_ fail
   (constraint violation, then a forced error) and asserts every
   touched table is unchanged afterwards. It runs for the chosen
   §3.2 mechanism and, as a recorded negative, for today's
   `exec_batch` pipeline.
2. `pmo_rel_state_and_intent_commit_or_rollback_together` — the
   row's first oracle: inject a failure after each local write of
   §3.1; afterwards either all of state, operation, intents and
   timeline are present, or none.
3. `pmo_rel_lost_cas_writes_nothing` — two local mutations race on
   one version; the loser leaves no operation, intent, timeline or
   side-write row, and gets `StateConflict`.
4. `pmo_rel_intent_payload_never_changes` — an update to any
   intent-block field is refused; a tampered payload stops at
   `INTENT_PAYLOAD_TAMPERED` without sending.
5. `pmo_rel_confirmed_never_regresses` — every transition out of
   `confirmed` is refused, including a replayed acknowledgement.
6. `pmo_rel_unknown_is_never_retried` — a sender never moves
   `unknown` to `delivering`; only a read-back fixture moves it.
7. `pmo_rel_per_target_order_holds_under_retry` — with seq 1
   retrying, seq 2 on the same target stays `pending`; a different
   target proceeds.
8. `pmo_rel_duplicate_scope_creates_no_second_intent`.
9. `pmo_rel_busy_store_leaves_no_partial_mutation` — inject
   `SQLITE_BUSY` at each step; then a same-key retry succeeds once.
10. `pmo_rel_attempt_log_matches_status` — after a random schedule of
    transitions, the latest attempt row's `to_status` equals the
    intent's status, and `attempt_no` is dense.
11. `pmo_rel_creation_outbox_unchanged` — the creation pilot's
    existing tests pass unchanged, and no REL-04 row references its
    tables.

## 11. Gates and sequencing

- **Hard blockers named by the issue.** REL-03: its record is merged
  (#15240). Queue Table Improvements: #9241 closed with the effort
  **not located**, and CEN-04 §6 carries that disposition, so REL-04
  reconciles against CEN-04's matrix rather than against a lost
  schema. A proven DB transaction contract: **not met**. §10 item 1
  is that proof, and it lands first, as its own change.
- **Storage admission.** The version column, the operation, intent
  and attempt rows, and `last_operation_id` each need a CEN-04 §5 row
  (fact, owner, home in the broker store, APIs, legacy mapping,
  disable-only rollback) before they are written.
- **Shared files.** The REL-04 row sequences after HYD-05 (on
  `turso_queue.rs`) and DSP-09 (#9449, on `operations.rs`). REL-03's
  ledger delta also lands in `operations.rs` and sequences after
  DSP-04/DSP-08. REL-04's implementation waits for those to land
  rather than editing the same files in parallel.
- **Downstream consumers.** REL-05 consumes §6 for stale relay acks.
  REL-06 consumes §5's `preconditions` and `remote_ref` and owns the
  read-back transitions. REL-08 consumes §7's blocked-head view.
- **Rollback.** Disable the producer: stop creating intents and stop
  the sender. Pending intents, attempt rows and remote effects are
  preserved, nothing is deleted or rewritten, and the old code path
  stays authoritative for the verbs that have not moved. No new
  database and no relay.

## 12. Follow-ups this record names but does not take

- Every multi-write `exec_batch` transaction in `turso_queue/`
  (`admin.rs`, `lifecycle.rs`, `links.rs`, `notes.rs`, `core.rs`)
  inherits §3.2's gap. Moving them to the proven mechanism is a
  separate change once §10 item 1 passes.
- `block_task`'s unguarded `DELETE FROM leases` (§ Verified state)
  can delete another worker's lease when its CAS loses. It is worth
  its own small fix, guarded on the post-block state, independent of
  REL-04.
- `event()` committing after the state transaction is the
  general case of "intent absent when state commits" for the
  timeline. Folding the timeline row into each verb's transaction is
  the same follow-up as the first bullet.

## 13. Requirement trace (design-level)

Full per-occurrence text is in
`doc/2026-11-13T03-20-24Z_design_pm-stack-fleet-orchestration/census/requirements-crosswalk.jsonl`.
This table records where this design speaks to each group of
allocated IDs. It is design coverage, not implementation.

| IDs | Theme | Where addressed |
| --- | --- | --- |
| C-R138, C-R139, C-R140, O-R032, P-R006, P-R021, H-R015 | stable operation ids, durable intent, outbox states, no duplicate effects | §3, §5, §6 |
| H-R016, P-R022, P-R033, P-R038 | crash after mutation, lost acknowledgement, partial outcomes | §6 `unknown`, §9 |
| C-R007, C-R145, P-R009 | direct completion and ordered effects | §6, §7 |
| C-R005, C-R009, C-R107, C-R108, C-R110, C-R133, H-R001, H-R017, H-R024, H-R031, O-R022, O-R023, O-R038, P-R025, P-R027, P-R028, P-R029 | preserved inventory, structured history, inspectable operations | §8, §11 storage admission, REL-08 hand-off |

## 14. Non-goals

- No storage, table, column or migration is created by this record.
- No change to `creation_outbox`, its drainer, its circuit or its
  pilot approval.
- No idempotency or receipt rule (REL-03), no remote precondition or
  read-back rule (REL-06), and no lease or fence rule (REL-01).
- No new database, queue, relay, service or scheduler.
- No change to existing broker verbs. §12 lists the follow-ups.
- No live GitHub effect, deployment or acceptance run.
