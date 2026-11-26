# REL-02 — Monotone fence generation and conditional local mutations

Issue: [#9487](https://github.com/developing-today/code/issues/9487)
(slice `REL-02` of [#9209](https://github.com/developing-today/code/issues/9209),
program [#9196](https://github.com/developing-today/code/issues/9196)).
Status: **design proposal — not an authorization to implement.**

This record elaborates the REL-02 row of the merged design package
(`doc/2026-11-13T03-20-24Z_design_pm-stack-fleet-orchestration/`,
PR #9225). The row says:

- Goal: "stale claimant cannot mutate local lifecycle after takeover".
- First oracle: `pmo_rel_stale_generation_cannot_write_after_reclaim`,
  which races a generation N mutation with an N+1 takeover.
- Acceptance: "strict generation increment, conditional versioned
  writes, DB clock authority".
- Negatives: "clock skew, renew after takeover, stale
  release/park/checkpoint, DB busy".

This record _decides_:

- what the fence generation is;
- exactly when it changes;
- how every holder mutation is conditioned on it;
- whose clock decides lease time;
- how legacy claims and rollback behave.

It authorizes no code, schema, migration, deployment or live
acceptance (§10).

Governing decisions consumed, not re-opened:

- **REL-01 record** (#9512,
  `doc/2026-11-25T01-23-08Z_design_rel01_claim_lease_compat_9512/note.md`):
  - the claim doors (pick, by id, batch, transfer and take-over,
    sink claim) and the current versus #9178 transition table;
  - "a `claimed` row whose lease lapsed is taken by no door until the
    sweep returns it".
- **#9178 / FDR-0001…0004**: take-over without the holder's consent
  is a target door (FDR-0003). That makes a stale holder a normal
  case, not a corner case.
- **Component #9209 §4**: operation identity carries a lease
  generation. `contracts.rs` `Command` already has an optional
  `lease_generation` (canonical decimal string).
- **REL-04 record** (#9514, merged in #15266):
  - multi-statement `BEGIN … COMMIT` pipelines on the broker store
    are not proven atomic;
  - follow-on writes are guarded on the row's version and the
    operation id.

  REL-02 only puts the fence into the **first** statement of a
  mutation. It does not re-decide multi-statement atomicity.

- **CEN-04** (#9434, record merged): §3.4 owns lease and claim fields,
  §5 admits no persisted field without a row, and §7 makes rollback
  disable-only.

## Verified state at `origin/main` `db23862b`

| Surface                                                                                                                  | What it does today                                                                                                                                                                                                                        | Gap for REL-02                                                                                                                                                                                                                                                         |
| ------------------------------------------------------------------------------------------------------------------------ | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Holder identity                                                                                                          | Holder writes check `status = 'claimed' AND worker_id = ?`, mostly through a separate `claimed_by` read (`turso_queue/lifecycle.rs:1651`). The claim code notes that "every lane shares worker_id repo-worker-code" (`lifecycle.rs:459`). | The worker id cannot tell two holders apart. A stale session and its replacement often carry the same id.                                                                                                                                                              |
| `heartbeat` (`turso_queue/heartbeat.sql`)                                                                                | One conditional `UPDATE … WHERE worker_id = ? AND claimed_at IS ? AND lane_id IS ? AND attempts = ?`.                                                                                                                                     | The strongest fence today, but by proxy. `claimed_at` is a one-second ISO string from the broker's clock. `attempts` does not increase on a claim from `parked`/`handed_off` (`claim.rs:1251`) or on a transfer (`lifecycle.rs:317`). Two holders can share the tuple. |
| `release` (`lifecycle.rs:2209`), `park` (`:2053`, `:2073`), checkpoint progress (`:1854`), `complete` (`admin.rs:~1604`) | Each reads the row with `claimed_by`, then writes `… WHERE id = ?` with no ownership condition.                                                                                                                                           | Check-then-act. A take-over or sweep between the read and the write is overwritten: a stale holder can release, park, checkpoint or complete a task someone else now holds. These are the row's "stale release/park/checkpoint" negatives, present today.              |
| `block_task` (`admin.rs:92`), transfer (`lifecycle.rs:317`)                                                              | Conditional on `status = 'claimed' AND worker_id = ?`.                                                                                                                                                                                    | Conditional, but on the shared worker id.                                                                                                                                                                                                                              |
| Lease time                                                                                                               | The broker computes `now_unix()` from its own system clock (`turso_queue/core.rs:68`) and passes it as a parameter. `expire_leases` (`core.rs:2582`) and the heartbeat's `julianday(?1)` compare against that parameter.                  | Two broker processes, or a broker and a repair tool, with different clocks disagree on whether a lease is live.                                                                                                                                                        |
| Generation field                                                                                                         | `Command.lease_generation: Option<String>` (`work/contracts.rs:648`) and `OwnershipExecutionProjection.lease_generation: Option<u64>` (`work/lifecycle.rs:255`) exist in the models.                                                      | No column stores it, no claim returns it, and no write checks it.                                                                                                                                                                                                      |

## 1. Intent

Exactly one holder at a time can change a claimed task, and "holder"
means one specific grant of the claim, not a worker name. Once the
claim moves on, whether by take-over, transfer, sweep, release or
anything else, every request made under the old grant fails with a
typed refusal and changes nothing. That holds however late the
request arrives, however wrong the sender's clock is, and whether or
not it shares a worker id with the new holder.

Safety comes from the generation. Liveness, meaning when a lapsed
claim may be taken over, comes from the lease clock. A wrong clock
can delay a take-over or bring one forward. It can never let two
holders write.

## 2. Ownership

| Concern                                                                                                                 | Owner                                              |
| ----------------------------------------------------------------------------------------------------------------------- | -------------------------------------------------- |
| The fence generation, its increments, the fenced write rule, the clock rule, legacy and rollback behavior (this record) | **REL-02**                                         |
| Which doors exist and who may use them                                                                                  | REL-01 + #9178 implementation slices               |
| Multi-statement atomicity, row version, follow-on guards                                                                | REL-04 (#9514)                                     |
| Carrying the generation on operations and relay acks                                                                    | REL-03 receipt (provenance), REL-05 (#9515) relay  |
| Remote preconditions                                                                                                    | REL-06 (#9516)                                     |
| The column and its migration                                                                                            | the queue broker as store owner, through CEN-04 §5 |
| CLI flags and env that carry the generation to a worker                                                                 | CLI-05 / CLI-13                                    |

## 3. The fence generation

### 3.1 Definition

`claim_generation` is a per-task integer, stored on the `tasks` row
and returned to callers as a canonical decimal string. `"0"` means
"never claimed under fencing" (§8).

It increases by **exactly one** every time the **holder** of the
claim changes, and at no other time:

| Event                                                                                       | Generation                |
| ------------------------------------------------------------------------------------------- | ------------------------- |
| Any door moves the row into `claimed` (pick, by id, batch, sink claim, override claim)      | +1, in the same statement |
| Transfer with the holder's consent                                                          | +1, in the same statement |
| Take-over without consent (FDR-0003)                                                        | +1, in the same statement |
| Heartbeat, checkpoint, progress update by the holder                                        | unchanged                 |
| Exit from `claimed` (release, park, block, complete, hand-off, sweep expiry, admin requeue) | unchanged                 |
| Any write to a row that is not `claimed`                                                    | unchanged                 |

Exits do not need an increment. Every holder write is conditional on
`status = 'claimed'` as well as the generation (§4), so after an exit
every old request fails on status. The next claim then increments, so
a request from before the exit also fails on generation.

### 3.2 Monotone

- The generation never decreases and is never reset: not on requeue,
  release, repair, quarantine, a backup load or a migration.
- A backup load restores the stored value, never a lower one.
- An administrative reset is a new claim (+1) or an exit (unchanged),
  never an assignment.
- The increment is always `claim_generation = claim_generation + 1`
  inside the conditional statement that changes the holder, with
  `RETURNING claim_generation`. It is never computed by the client
  and written back.

### 3.3 Carried by the holder

- Every claim response (single, batch, by id, transfer, take-over)
  returns `claim_generation` with the claim.
- The worker carries it on every later request for that task, as
  `lease_generation` in the shared `Command` (`contracts.rs:648`).
  The CLI carries it in the claim envelope it already passes along
  (CLI-05).
- The worker id, lane id and session id stay on the row and in
  refusals as diagnostics. They are not the fence.

## 4. Fenced writes

### 4.1 The rule

Every mutation a holder makes is authorized by the **first write
statement** of that mutation, and that statement's `WHERE` clause
includes:

```sql
WHERE id = ? AND status = 'claimed' AND claim_generation = ?
```

with the generation the request carries. The affected-row count of
that statement is the authorization decision:

- **1**: the request was made by the current holder, and the
  mutation proceeds.
- **0**: the request is fenced, and nothing else in the mutation may
  take effect. With REL-04's mechanism, the follow-on writes are
  guarded on the post-state and become no-ops.

A preceding read, such as `claimed_by`, may stay to build a precise
error message. It never authorizes. This replaces every check-then-act
write in the verified-state table:

| Verb                       | First statement gains                                                                                        |
| -------------------------- | ------------------------------------------------------------------------------------------------------------ |
| heartbeat                  | `claim_generation = ?` replaces the `claimed_at`/`lane_id`/`attempts` proxy (which may stay as a diagnostic) |
| checkpoint (task progress) | the full fence; today `WHERE id = ?`                                                                         |
| park, hand-off             | the full fence; today `WHERE id = ?`                                                                         |
| release                    | the full fence; today `WHERE id = ?`                                                                         |
| complete                   | the full fence; today `WHERE id = ?`                                                                         |
| block                      | `claim_generation = ?` alongside the current worker check                                                    |
| transfer (by the holder)   | the full fence, then +1 in the same statement                                                                |

Take-over is not a holder write. It is conditional on the row's
current generation as the take-over reads it, so two concurrent
take-overs cannot both win.

### 4.2 Refusals

A fenced write returns a typed refusal, never a string to parse:

- `FENCED { task_id, presented_generation, current_generation,
current_status }` when the generation or status no longer matches.
- `FENCE_REQUIRED` when a request without a generation reaches a
  task claimed under fencing (§8).

Both are added to the stable error vocabulary next to REL-03's
`IDEMPOTENCY_KEY_REUSED`, in the same uppercase spelling. Each one
writes a `claim_refused` timeline row with `reason: "fenced"`, as
`claimed_by` already does for a foreign lane. The worker treats
`FENCED` as final for that task: it stops acting on it and drops
its local claim.

### 4.3 What the fence does not cover

- **Remote effects already sent.** A request that left the worker
  before the take-over can still land remotely. That is #9209 §3.2's
  case, handled by REL-04 §9.1 and REL-06, not by a local fence.
- **Writes by non-holders.** The sweep, admin requeue and quarantine
  are conditional on their own predicates (lease lapsed, status). They
  do not carry a holder generation and do not increment it, except
  where they hand the claim to someone (§3.1).

## 5. The clock

### 5.1 The DB clock decides lease time

Every lease time decision is made in SQL against the store's own
clock, never against a time the broker passes in:

- whether a lease is live;
- whether it has lapsed for the sweep;
- the new `lease_until` on claim and heartbeat;
- the not-before of a park.

The store's clock is SQLite's `'now'`, evaluated server-side on the
hosted store and in-process on a local replica. Lease times stay in
the existing ISO-8601 UTC text format and are compared with
`julianday()` on both sides, as `heartbeat.sql` already does. The
broker's own clock may only be used for display and for timeline
`created_at` values, which carry no authority.

### 5.2 Skew affects liveness only

Because the fence (§4) carries safety:

- **Holder's clock ahead or behind.** Irrelevant. The holder never
  decides lease time.
- **Broker clock skewed.** Irrelevant after §5.1, because it decides
  nothing.
- **Store clock jumps.** A forward jump can expire leases early, so a
  take-over may happen sooner than intended and the old holder is
  fenced. A backward jump can keep a lease live longer. Neither lets
  two holders write.
- **Heartbeat after the lease lapsed, before the sweep.** Allowed, and
  the lease is renewed. REL-01's fact that no door takes an unswept
  lapsed claim means the holder is still the only holder, and the
  fence proves it. Once the sweep or a take-over has run, the same
  heartbeat is fenced.

## 6. Busy store and lost responses

- **DB busy or store 5xx on a fenced write.** Retryable with the same
  generation. The write is conditional, so a retry either applies once
  or is fenced. It never applies twice under a different holder.
- **Lost response.** The worker re-reads the row:
  - if the generation and status show the mutation took effect
    (for example `queued` after its own release, with its generation
    and no newer one), it treats the mutation as done;
  - if the generation moved, it is fenced;
  - otherwise it retries.

  Heartbeat and checkpoint are safe to retry blindly. Release, park
  and complete reach a non-`claimed` status, so a duplicate is fenced
  on status, not applied twice.

- **Busy during a claim.** The claim's own conditional `UPDATE`
  either increments once or does nothing. A retried claim is a new
  claim attempt through its door.

## 7. Interaction with the other slices

- **REL-04.** The fence is part of the state CAS of REL-04 §3.1 step 1
  (the "lease generation where REL-01 applies"). The operation and
  intent rows record the generation that accepted them. A sender that
  delivers an intent re-checks its own send lease (REL-04 §9.1),
  which is a different lease from the task claim.
- **REL-03.** The receipt's provenance records the generation the
  command was accepted under. A replay never re-checks the fence,
  because a replay writes nothing.
- **REL-05.** A relay acknowledgement carries the generation of the
  operation it acknowledges. REL-05 decides whether an ack from an
  older generation is dropped. REL-02 only guarantees the number is
  monotone and comparable.
- **#9178 take-over.** FDR-0003's recorded "prior worker and
  hand-back" gains the prior generation, so the timeline shows which
  grant was revoked.

## 8. Legacy claims, rollout and rollback

- **Column.** `claim_generation INTEGER NOT NULL DEFAULT 0`, added
  through a CEN-04 §5 row on §3.4 (owner: the queue broker). It is
  additive: old code ignores it.
- **Enable point.** Fencing is switched on per queue. At the switch,
  the store records `fence_enabled_at` (DB clock) for that queue in
  the broker's existing config surface, which is also a CEN-04 row.
- **Legacy claims.** A claim whose `claimed_at` is before
  `fence_enabled_at` was granted without a generation. Its holder's
  requests carry none, and they are accepted under today's checks
  until that claim exits. A request without a generation for any
  claim granted after the switch is `FENCE_REQUIRED`. A legacy claim
  that is transferred or taken over becomes fenced, at generation 1 or
  more.
- **Canary.** One queue first. The §9 oracles pass against a store
  copy before the switch. The timeline's `claim_refused`/`fenced`
  count is watched for false refusals.
- **Rollback.** Disable fencing for the queue. The old checks resume,
  and the column and its values stay. Old code does not increment
  the generation, so a holder change during the rollback window would
  leave two holders on the same number. **Re-enabling** therefore
  runs one statement that sets `claim_generation =
claim_generation + 1` for every row of the queue, still monotone,
  and moves `fence_enabled_at` to the re-enable time. Every token from
  before the rollback is dead, and claims made during the window are
  legacy claims under the rule above. Nothing is deleted.

## 9. Acceptance oracle

Tests a separately scoped implementation must add. They are listed by
exact name before they count, per the row's done command. The
store-backed ones run on the rusqlite fake that `turso_queue`'s tests
already use (`lifecycle.rs` `fake_backend`), plus a hosted-store run
for §5.1.

1. `pmo_rel_stale_generation_cannot_write_after_reclaim` — the row's
   first oracle. For each holder verb (heartbeat, checkpoint, park,
   hand-off, release, block, complete, transfer), a holder at N reads,
   a take-over to N+1 commits, then the N write runs. It is `FENCED`,
   and the row is byte-identical to after the take-over.
2. `pmo_rel_generation_increments_once_per_holder_change` — every door
   in REL-01 §3 plus transfer and take-over gives exactly +1. Holder
   writes and exits give 0.
3. `pmo_rel_generation_never_decreases` — requeue, sweep, repair,
   quarantine, backup load and re-enable never lower it.
4. `pmo_rel_renew_after_takeover_is_fenced`.
5. `pmo_rel_renew_after_lapse_before_sweep_is_allowed` — and the same
   heartbeat after the sweep is fenced.
6. `pmo_rel_shared_worker_id_holders_are_fenced_apart` — two sessions
   with the same worker and lane ids, consecutive grants; the older
   one is fenced.
7. `pmo_rel_lease_decisions_ignore_broker_clock` — with the broker's
   clock skewed ±1 h, the sweep, liveness and new `lease_until` are
   unchanged.
8. `pmo_rel_store_clock_skew_never_admits_two_writers` — the store
   clock jumps forward and back across a take-over; at most one
   generation's writes apply.
9. `pmo_rel_busy_fenced_write_retries_once` — busy injected on each
   holder verb; the same-generation retry applies once, or is fenced
   if a take-over intervened.
10. `pmo_rel_legacy_claim_accepted_only_before_enable` and
    `pmo_rel_reenable_bumps_every_generation`.

## 10. Gates and sequencing

- **Hard blockers named by the row.**
  - REL-01: record merged.
  - #9178: its door implementations land the take-over this fence
    protects. The fence can land first, because it only adds
    conditions.
  - Queue Table Improvements: #9241 closed with QTI not located, and
    CEN-04 §6 carries that. The column goes through CEN-04 §5 instead
    of a QTI crosswalk.
- **Shared files.** The row sequences after PRI-03 on `turso_queue.rs`
  and after CLI-05 on `work/lifecycle.rs`. The verb changes in §4.1
  live in `turso_queue/lifecycle.rs` and `turso_queue/admin.rs`, so
  they also queue behind REL-04's follow-up on the same files (REL-04
  §12).
- **Independent fix.** Making today's check-then-act writes
  conditional on `status = 'claimed' AND worker_id = ?` is a smaller,
  valid step that closes the sweep race without the column. It does
  not close the shared-worker-id case. It can land first as a
  separate change, and REL-02 then swaps the worker condition for the
  generation.
- **Downstream.** REL-04's state CAS, REL-05's relay, the #9178
  take-over and the CLI-05 receipts carry the generation.

## 11. Requirement trace (design-level)

Full per-occurrence text is in
`doc/2026-11-13T03-20-24Z_design_pm-stack-fleet-orchestration/census/requirements-crosswalk.jsonl`.
This is design coverage, not implementation.

| IDs                                                                                                                            | Theme (from the crosswalk's source quotes)                                                                    | Where addressed              |
| ------------------------------------------------------------------------------------------------------------------------------ | ------------------------------------------------------------------------------------------------------------- | ---------------------------- |
| C-R134, C-R135, H-R014, O-R027, O-R035                                                                                         | atomic claim issuing a fencing counter; a stale agent cannot overwrite a newer claim; two claimers, one lease | §3, §4                       |
| C-R144, P-R023                                                                                                                 | completion validates the lease before it acts                                                                 | §4.1 (`complete` row)        |
| C-R151, C-R153, O-R052, P-R007                                                                                                 | park, resume and near-timeout checkpoint across interruptions                                                 | §4.1, §6                     |
| H-R006, P-R001, P-R011, P-R020                                                                                                 | claiming among lanes and work operations                                                                      | §3.3 (shared worker ids), §4 |
| C-R005, C-R009, C-R107, C-R108, C-R110, C-R145, H-R016, H-R017, H-R024, H-R031, O-R023, O-R032, P-R006, P-R009, P-R025, P-R033 | structured refusals, preserved inventory, crash and recovery                                                  | §4.2, §6, §8                 |

## 12. Non-goals

- No new claim door, flag or verb (REL-01, #9178).
- No multi-statement transaction mechanism (REL-04).
- No remote fencing of requests already sent (REL-04 §9.1, REL-06).
- No change to lease durations, the sweep cadence or the attempt cap.
- No new database, queue or service. No live queue change.
