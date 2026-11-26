# REL-05 — Lease-fenced outbox relay, ordering and supersede policy

Issue: #9515 (slice REL-05 of #9209). Status: design record. It is not
an authorization to implement. It decides three things for REL-04's
intent table:

- how the relay that sends an intent is leased and fenced, so a stale
  relay's acknowledgement can never move a newer state backwards;
- how sends to one target are serialized;
- when an intent may be superseded before it is sent, and what
  happens once a send may already have happened.

## 0. Governing decisions consumed

| Source                         | What this record takes from it                                                                                                                                                    |
| ------------------------------ | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| #9515 slice text               | Owned relay model in `work/operations.rs`; red test `pmo_rel_stale_relay_ack_cannot_regress_operation`; three negatives; acceptance; staged migration; no invented retry cap.     |
| #9209 §3.2, §4, failure matrix | A stale worker may only append its remote result to its own operation as evidence; the relay's lease has its own generation; supersede only before send; no cancellation fiction. |
| REL-04 record                  | Intent fields, the immutable intent block, the status table, CAS on `(intent_id, status, attempts)`, per-target `target_seq`, the attempt log, §9.1 stale-send rules.             |
| REL-02 record §7               | Generations are monotone integers on the wire as decimal strings; the send lease is not the task claim; REL-05 decides what an older-generation ack does.                         |
| REL-03 record                  | `operation_id` identity; receipts change only through REL-04's table; a replay never asks for a send.                                                                             |
| REL-06 record                  | Verdicts, `retry_class`, rate-limit and scope refusals before execution may retry per provider guidance and the caller's deadline; a timeout after send is `unknown`.             |
| AUTH-02 record, #9208 §7       | A revoked mirror permission ends sends with a terminal auth failure and no repeated unauthorized retry.                                                                           |
| `work/contracts.rs` (main)     | `Command.lease_generation: Option<String>`, documented as the relay lease generation, a canonical decimal string.                                                                 |

## 1. Verified state on main `4e82d4fd`

- `work/operations.rs` (461 lines) holds DSP-01's command acceptance
  model only: `Principal`, `Verb`, `ResourceRef`, `Command`,
  `Receipt`, `Effect`, `AcceptanceLedger`. It has no relay, lease,
  generation or intent status.
- No REL-02, REL-03 or REL-04 code is on main. `claim_generation`,
  `relay_generation` and an intent table do not exist.
- The creation outbox (`creation_outbox.rs`,
  `turso_queue/stacks_outbox.rs`) has states `pending`, `leased`,
  `uncertain`, `done`, `blocked`, and lease fields `lease_owner` and
  `lease_until`, with no generation. `verb_outbox_resolve` takes only
  `id` and `outcome`, and every `UPDATE` there is `WHERE id = ?`. A
  drainer whose lease was reclaimed can therefore still resolve the
  row, including over another drainer's `done`. That is the stale-ack
  defect this slice exists to rule out, live today in an older path.
- `reprioritize_sync_outbox` in the broker re-sends from the latest
  task row ("a newer reprioritize supersedes"). Its ack is guarded by
  a re-read, not a CAS, and its failures back off by a fixed 30 s.
- The creation outbox's own constants (`BACKOFF_CAP_SECONDS=3600`,
  `DEFAULT_MAX_ATTEMPTS=8`, `HOURLY_BUDGET=300`) and `backoff.rs`
  (PostgREST retry) belong to those paths and are not a provider
  policy.
- `pmo_rel_` tests exist only in `work/completion.rs` (REL-09) and
  `work/lifecycle.rs` (REL-01). None covers a relay.

## 2. Intent

At most one relay sends to a target at a time, and only the relay
holding the current lease generation can move an intent's status.
Whatever an older relay learns still lands on the record, as evidence
on its own immutable attempt, never as a status change. An intent is
cancelled only while nothing can have been sent; after that, the only
way forward is read-back. Retry timing comes from the provider and the
caller's deadline, not from a number made up here.

## 3. Ownership

- **Owned:** the relay model in
  `pkgs/fleet/crates/fleet-queue/src/work/operations.rs`, as new pure
  types and functions beside the DSP-01 model, with tests in its
  existing `mod tests`.
- **Integration, later and separately approved:** wiring the model
  into the broker's drain for REL-04's intents. The slice text allows
  the current broker drain "only after integration approval".
- **Read-only:** REL-04's intent and attempt shapes, REL-02's
  generation rules, REL-06's verdicts, `contracts.rs`.
- **Persisted fields:** the send-lease record in D1 is a logical
  shape. Its table, columns and migration go through REL-07 and the
  #9198 store owner via CEN-04 §5, as REL-04 §2 assigns.
- **Added nowhere:** no new queue, service, scheduler, GitHub client
  or retry constant.

## 4. Decisions

### D1. One send lease per target, with its own generation

```rust
struct SendLease {
    target_ref: String,       // REL-04's target_ref
    holder: String,           // relay instance id
    relay_generation: String, // canonical decimal, monotone per target
    lease_until: String,      // DB clock, REL-02 §5
}
```

- One lease exists per `target_ref`. Taking it is a CAS that either
  creates it at generation `1` or, when the current lease has lapsed
  by the DB clock, replaces the holder and increments
  `relay_generation` by exactly 1. Renewing keeps the generation.
- The generation never decreases and never resets, as REL-02 requires
  of every generation. It is unrelated to the task's
  `claim_generation`; a sender needs no task claim, and a task claim
  grants no send.
- `Command.lease_generation` in `contracts.rs` carries this number
  when a command acts as a relay. It means nothing else.
- Holding the lease is what REL-04's `pending → delivering` condition
  "holds the target's send lease" refers to.

### D2. Every attempt is stamped, and only the current stamp moves status

- Moving an intent to `delivering` opens REL-04's attempt row and
  writes the lease's `relay_generation` into it. The attempt row's
  identity block (`intent_id`, `attempt_no`, `relay_generation`,
  `started_at`) is immutable once written.
- An acknowledgement is the relay's report of one attempt: the HTTP
  outcome, any remote ref, any provider retry hint. Applying it is
  `apply_ack(intent, lease, attempt, ack) -> AckDecision`, a pure
  function with three answers:

| Answer              | When                                                                                                                 | Effect                                                                                                                               |
| ------------------- | -------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------ |
| `Apply(transition)` | The intent is `delivering`, `attempts` matches, and the attempt's `relay_generation` equals the lease's current one. | One REL-04 transition, by its CAS on `(intent_id, status, attempts)`, plus the attempt row closed.                                   |
| `RecordLateEffect`  | The attempt's generation is older than the lease's, or the intent has already left `delivering`.                     | The ack is written to that attempt as `late_effect` evidence (`remote_status`, `remote_ref`, observed time). Status does not change. |
| `Reject(code)`      | The ack names an attempt that does not exist or does not belong to the intent.                                       | Nothing is written. Code `relay_ack_unknown_attempt`.                                                                                |

- So a generation-N ack that arrives after generation N+1 took the
  lease can never regress or advance the intent. This is the first
  red oracle.
- A `late_effect` is never thrown away. If it reports a success, the
  current holder treats it as a read-back hint and still runs REL-06
  read-back before any transition; the hint alone confirms nothing.
- `confirmed`, `conflict`, `terminal_failure` and `superseded` are
  terminal. Any ack after them is `RecordLateEffect`.
- A lapsed lease with no new holder yet keeps its generation. If the
  same holder's ack then arrives, it still matches and may apply.
  REL-04's "next sender moves `delivering` to `unknown`" happens when
  a new holder takes the lease, by that new generation.

### D3. One sender per target, oldest first

- The send lease (D1) serializes sends to one target: one holder, one
  `delivering` intent at a time.
- Within the target, REL-04 §7 decides which intent is next: the
  lowest `target_seq` whose predecessors are all `confirmed`,
  `conflict`, `terminal_failure` or, with this record,
  `superseded`.
- A newer intent never overtakes an older unresolved one on the same
  target, including an older one in `unknown`.
- A relay never holds a database write transaction across the HTTP
  call (#9209 §3). It commits `delivering`, sends, then applies the
  ack in a new transaction under D2.

### D4. Supersede before send, reconcile after

REL-04's table gains one terminal state, `superseded`, which this
record adds and REL-04's owner must accept:

| From                | To           | Condition                                                                                        |
| ------------------- | ------------ | ------------------------------------------------------------------------------------------------ |
| `pending`           | `superseded` | CAS on `(intent_id, status, attempts)`; actor, reason and `superseded_by` operation id recorded. |
| `retryable_failure` | `superseded` | Same, and only when REL-06 classed the last failure as before any remote acceptance.             |

- **Never after a possible send.** `delivering` and `unknown` cannot
  become `superseded`. Their only way out is REL-06 read-back. This is
  #9209's "no cancellation fiction".
- **Who may supersede.** Only an explicit operation by an authorized
  principal (AUTH-01's capability), or the producer of a newer intent
  in the case below.
- **Automatic supersede is narrow.** A newer intent may supersede an
  older one automatically only when both are set-to-value kinds on the
  same field of the same target, so the newer one fully decides the
  final remote value. Additive kinds (comments, appended text) are
  never superseded automatically, because each is its own fact.
- **Desired state reverses after a send.** If the older intent is
  already `delivering` or `unknown`, the newer intent waits behind it
  (D3). The older one is reconciled by read-back first. Only then does
  the newer one send, and it sends against what read-back found. A
  correction is a new intent, never an edit of the old one.
- **Revocation stops future sends only.** A revoked permission (for
  example AUTH-02's mirror consent) moves `pending` and
  `retryable_failure` intents of that kind to `terminal_failure` with
  `AUTH_DENIED`, and leaves a `delivering` or `unknown` one to
  read-back.

### D5. Retry timing comes from the provider and the deadline

- A failure REL-06 classes as before execution (`retry_class`
  rate-limit or transient) moves the intent to `retryable_failure`.
  `next_attempt_at` is the provider's own instruction when it gives
  one (GitHub `Retry-After`, or `x-ratelimit-reset` when remaining
  is `0`). Otherwise it is the provider's published guidance for that
  response. It is never clamped to a number chosen here.
- The caller's deadline bounds it. If `next_attempt_at` falls after
  the operation's deadline, the intent goes to `terminal_failure` with
  `DEADLINE_EXCEEDED` instead of waiting.
- No retry count, ceiling or interval constant is introduced. The
  creation outbox's caps stay where they are and are not reused here.
- Every retryable result returns the exact `next_attempt_at` and the
  current lease holder, so an operator can see who sends next and
  when.
- **Verify before any resend.** A `retryable_failure` resends only
  when REL-06 classed it as before execution. Anything else goes
  through `unknown` and read-back first (REL-04 §6, §9.1), so no
  intent is sent twice on a guess.

### D6. Lease expiry during the HTTP call

1. The holder re-checks its lease just before the request (REL-04
   §9.1). If it has lapsed, the holder does not send, and the intent
   stays `delivering` for the next holder to move to `unknown`.
2. If the lease lapses while the request is in flight and a new holder
   takes it, the new holder moves the intent to `unknown` under the
   new generation and starts read-back.
3. When the old request returns, its ack is `RecordLateEffect` (D2).
   Read-back, not the old ack, decides the outcome. If the effect is
   additive and read-back cannot see it yet, REL-04 §9.1's
   no-retry-on-absence rule holds.

### D7. Error codes

| Code                        | Meaning                                                                          |
| --------------------------- | -------------------------------------------------------------------------------- |
| `relay_lease_held`          | Another holder has a live send lease on the target; try after `lease_until`.     |
| `relay_generation_stale`    | A relay presented a generation older than the lease's; its ack is evidence only. |
| `relay_ack_unknown_attempt` | The ack names no attempt of this intent.                                         |
| `supersede_after_send`      | A supersede was asked of a `delivering` or `unknown` intent.                     |
| `DEADLINE_EXCEEDED`         | The provider's next allowed attempt is after the operation's deadline.           |

`AUTH_DENIED`, `REMOTE_CONFLICT` and `REMOTE_UNAVAILABLE` stay REL-06's
codes and are used as REL-06 defines them.

### D8. The creation outbox defect is named, not fixed here

`verb_outbox_resolve` accepting a resolve from a drainer that no
longer holds the row (§1) is the same class of bug as D2 forbids.
REL-04 keeps the creation outbox unchanged
(`pmo_rel_creation_outbox_unchanged`), so this record does not edit
it. The bounded follow-on is:

- add `lease_owner = ?` and `state = 'leased'` to every resolve
  `UPDATE`;
- have the client send its owner with `/outbox-resolve`;
- record a refused resolve as an `outbox_resolve_stale` event.

It needs its own issue and approval because it changes a live path.

## 5. Negative cases

| Negative (slice text)                    | Result                                                                                              | Test                                                  |
| ---------------------------------------- | --------------------------------------------------------------------------------------------------- | ----------------------------------------------------- |
| Stale relay ack after a newer claim      | Generation-N ack after N+1 took the lease is `RecordLateEffect`; status unchanged.                  | `pmo_rel_stale_relay_ack_cannot_regress_operation`    |
| Old op sent, then desired state reverses | The newer intent waits; the older is read back first; no supersede after send.                      | `pmo_rel_reversal_waits_for_older_read_back`          |
| Rate limit or transient error            | `retryable_failure` with the provider's own next time; past the deadline it is `DEADLINE_EXCEEDED`. | `pmo_rel_retry_time_comes_from_provider_and_deadline` |
| Lease expires during HTTP                | New holder moves to `unknown` under its generation; the old ack is late evidence.                   | `pmo_rel_lease_expiry_mid_send_goes_unknown`          |
| Supersede after a possible send          | `supersede_after_send`.                                                                             | `pmo_rel_supersede_only_before_send`                  |
| Additive effect auto-superseded          | Refused; only set-to-value pairs on one field supersede automatically.                              | `pmo_rel_additive_intent_never_auto_superseded`       |

## 6. Oracles and tests

Unit tests in `work/operations.rs`'s `mod tests`, against a pure
in-memory model of the lease, the intent row and the attempt log:

| Test                                                     | Asserts                                                                                                                                                                                  |
| -------------------------------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `pmo_rel_stale_relay_ack_cannot_regress_operation` (red) | Interleave: N takes lease, sends; N lapses; N+1 takes lease and moves the intent to `unknown`; N's success ack arrives. The intent stays `unknown`; N's attempt holds the `late_effect`. |
| `pmo_rel_late_effect_attaches_to_immutable_attempt`      | The late ack lands on attempt N's row and nowhere else; the identity block is unchanged.                                                                                                 |
| `pmo_rel_one_sender_per_target`                          | A second holder cannot take a live lease; generations only increase.                                                                                                                     |
| `pmo_rel_order_holds_with_supersede`                     | A `superseded` predecessor releases the next `target_seq`; an `unknown` one does not.                                                                                                    |
| `pmo_rel_supersede_only_before_send`                     | `pending` and pre-execution `retryable_failure` supersede; `delivering` and `unknown` refuse.                                                                                            |
| `pmo_rel_additive_intent_never_auto_superseded`          | Only set-to-value pairs on one field supersede automatically.                                                                                                                            |
| `pmo_rel_reversal_waits_for_older_read_back`             | A reversing intent sends only after the older one leaves `unknown` through read-back.                                                                                                    |
| `pmo_rel_retry_time_comes_from_provider_and_deadline`    | `Retry-After` and reset headers set `next_attempt_at` unclamped; a time past the deadline gives `DEADLINE_EXCEEDED`.                                                                     |
| `pmo_rel_lease_expiry_mid_send_goes_unknown`             | The D6 sequence, step by step.                                                                                                                                                           |
| `pmo_rel_retry_reads_back_before_resend`                 | Only a before-execution failure resends directly; every other failure passes through `unknown`.                                                                                          |
| `pmo_rel_revocation_stops_future_sends_only`             | Revocation ends `pending` and `retryable_failure`; `delivering` and `unknown` wait for read-back.                                                                                        |
| `pmo_rel_legacy_rows_start_at_generation_zero`           | A row with no stamped generation is read as generation `0` and `unknown`, never as current.                                                                                              |

Done command, as the slice states, run remotely through a
`fleet-rust-tests.yml` dispatch on the branch:

```text
cargo test --manifest-path pkgs/fleet/Cargo.toml -p fleet-queue --lib \
  --features client,queue-broker -- --list
cargo test --manifest-path pkgs/fleet/Cargo.toml -p fleet-queue --lib \
  --features client,queue-broker pmo_rel_stale_relay_ack_cannot_regress_operation
```

The first must show the test by its exact name; a filter that matches
nothing is not a pass. Then the whole `pmo_rel_` set runs the same
way, with REL-01's and REL-09's existing `pmo_rel_` tests still green.

## 7. Compatibility, migration and reversal

- **Staged.** Read first: the drain learns to read a stamped
  `relay_generation` and treats a missing one as `0`, which can never
  equal a live lease, so every legacy in-flight row goes to `unknown`
  and read-back rather than being trusted. Write second: new attempts
  are stamped.
- **Rollback** disables the new relay and keeps every intent, attempt,
  lease and `late_effect` row. Pending intents wait for an explicit
  drain. No row is deleted or rewritten.
- The creation outbox and `reprioritize_sync_outbox` are untouched
  (D8).

## 8. Requirement trace

| Requirement                                                                            | How this record serves it                                                                                                                     |
| -------------------------------------------------------------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------- |
| C-R133, C-R138, C-R139, C-R140, O-R032, P-R021, P-R053, P-R068                         | REL-04's states and stable ids, with a fenced relay so retries and acks cannot regress them (D1, D2), plus the added `superseded` state (D4). |
| H-R015, P-R006, P-R042, P-R057                                                         | Lease expiry is recovered by a new generation; stale relay writes become evidence only (D1, D2, D6).                                          |
| P-R023, P-R043, P-R058                                                                 | GitHub unavailable or a lost ack keeps the intent and its attempt record; the result is pending or `unknown`, never success (D2, D5).         |
| P-R044, P-R059                                                                         | No resend before read-back unless the failure was before execution (D5).                                                                      |
| P-R041, P-R056                                                                         | One sender per target by lease CAS; the loser gets `relay_lease_held` (D1, D3, D7).                                                           |
| C-R007, C-R145, P-R022, P-R033, P-R038, P-R045, P-R060                                 | Completion's outbox steps send in order and are never reported done on a stale ack; a partial result stays visible (D2, D3).                  |
| P-R046, P-R061                                                                         | Set-to-value supersede only before send; after it, read-back decides against the remote's current value (D4).                                 |
| P-R047, P-R062                                                                         | A reversed desired state waits for the older intent's read-back before sending (D4).                                                          |
| P-R048, P-R063                                                                         | Not served here; cycle refusal is DEP-03's. Listed so the trace is complete.                                                                  |
| C-R005, C-R009, C-R107, C-R110, O-R022, O-R023, O-R038, P-R009, P-R025, P-R028, P-R029 | One relay behind the shared seam, built on existing outbox and lease concepts, with no parallel dispatch path (§3, D3).                       |
| H-R001, H-R016, H-R017, H-R024, H-R031, P-R027                                         | Stale-lease, crash, rate-limit and reversal cases carried into named oracles (§5, §6).                                                        |

## 9. Gates

1. This record merged.
2. REL-04's intent table and transition CAS in code, with REL-04's
   owner accepting the `superseded` state (D4).
3. REL-06's verdicts and `retry_class` in code, for D2's read-back
   and D5's classification.
4. AUTH-02's revocation signal, for D4's last rule.
5. The send-lease storage through REL-07 and the #9198 store owner via
   CEN-04 §5.
6. The pure model and its tests can land after REL-04's types exist;
   the broker drain integration is its own approval. Live sends to
   GitHub stay behind their own acceptance.

## 10. Non-goals

- REL-04's intent table, transitions and attempt log, beyond adding
  `superseded`.
- REL-06's preconditions and read-back.
- Fixing the creation outbox (D8 names it).
- A retry budget, poll interval or ceiling.
- Operator tooling for stuck intents (REL-08).
