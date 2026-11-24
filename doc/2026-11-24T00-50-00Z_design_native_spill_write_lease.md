# Native-spill write lease protocol for hot worlds

- Status: proposed design (no implementation in this record)
- Date: 2026-11-24 (UTC)
- Source issue: [#2819](https://github.com/developing-today/code/issues/2819)
- Parent: [#2059](https://github.com/developing-today/code/issues/2059) investigation,
  [`2026-10-30T19-35-00Z_investigation_hot_world_sharding.md`](2026-10-30T19-35-00Z_investigation_hot_world_sharding/2026-10-30T19-35-00Z_investigation_hot_world_sharding.md) §3 rung 3 and §7
- Program: ID platform expansion, epic [#2056](https://github.com/developing-today/code/issues/2056); consumer [#2046](https://github.com/developing-today/code/issues/2046)
- Tracking: [#13873](https://github.com/developing-today/code/issues/13873), umbrella [#13871](https://github.com/developing-today/code/issues/13871)

## 1. Problem and context

Rung 3 of the hot-world strategy moves a hot world's program turns to a native
`id` node, because one indivisible topic or region stays critical after rungs
0–2 and cannot be partitioned. The investigation names the consequence
precisely: the hosted Durable Object (DO) and a native node must never both
accept writes for one world. That is split-brain, and it corrupts the journal
of record. The investigation says "the lease is the whole design" but does not
design it.

Today nothing in `pkgs/id/src/world/` provides a write fence. Each world's
actor is the single writer by construction, because it is the only process
holding that world's state. Spill breaks that construction: two processes now
hold a copy of the world and can both run commands. Without an explicit,
journaled grant, the safe handoff rests on timing, and timing is not a
correctness argument.

Evidence the design must respect:

- Journal append is the write that matters. Signals in investigation §2
  (`journal_append_latency`, `checkpoint_every: 1000`) show the journal is the
  durable state; the actor is a serialization point over it.
- Signatures are transport-independent (investigation §3 rung 3, #2044 bridge
  model). Native results come back as signed events, so the journal can verify
  who produced an entry, but a valid signature alone does not say the producer
  was the current writer.
- Spill is temporary by design (investigation §2): it starts when the critical
  band is sustained and ends when the warning band clears for about one hour.

## 2. Goals and non-goals

Goals:

1. At most one writer for a world at any journal position. Two epochs can never
   both append accepted entries.
2. The grant, revoke and handoff are recorded as journaled entries, so any
   auditor can answer "who held the write lease at journal position P".
3. A partitioned or crashed ex-holder is fenced out by the journal, not by its
   own honesty or clock.
4. Handoff in both directions keeps the journal contiguous: no lost entries and
   no duplicated entries, and in-flight commands have a defined outcome.
5. Liveness is bounded: a partition makes a world unwritable for at most one
   lease term plus a skew margin, then the DO reclaims.

Non-goals:

- Multi-writer or CRDT merge of one world. Rung 3 keeps one writer.
- Choosing *when* to spill. The triggers and bands are owned by #2275 (the
  detector) and investigation §2.
- Read fan-out, which is rung 0 or rung 4.
- Cross-world coordination or hierarchical objects (rung 4).
- Replacing the journal format. This design adds entry kinds only.

## 3. Alternatives considered

| Option | Shape | Why not chosen |
| --- | --- | --- |
| Do nothing (no spill) | Hot indivisible worlds stay on the DO and are throttled | Leaves the critical case unserved. Kept as the default until slice 7 ships, so do-nothing remains the fallback while spill is off. |
| Clock-only lease | Native node writes while its wall-clock lease is unexpired; DO waits out the term | Correct only if clocks are bounded. Nothing in the journal rejects a late writer, so a native node with a skewed clock can write after the DO has reclaimed. Rejected as the sole mechanism; kept only as a liveness aid. |
| Consensus (Raft or Paxos over DO and node) | Replicated log with leader election | Strongest, but two-node consensus needs a third voter for liveness and adds a new protocol to operate. Disproportionate for one temporary spill per hot world. |
| DO-only arbitration with journaled epoch fence (chosen) | DO grants epochs as journal entries; every append carries its epoch; the journal rejects stale epochs | Uses the one component that already serializes the world. The fence is checked where the data is, so safety does not depend on clocks. |
| Shard the world instead | Split the indivisible world by a finer key | Excluded by the premise of rung 3: the load is indivisible by definition. |

## 4. Chosen approach

### 4.1 Actors and state

- **Journal of record (JoR):** the DO for the world, normally. It is the only
  process that grants, revokes and renews leases.
- **Spill node (N):** a native `id` node running the world's program turns for
  one epoch.
- **Mode, held by the JoR and derived from the journal:**
  `Hosted` or `Spilled { epoch, holder, start_seq }`. The JoR keeps one current
  lease epoch `E` and its holder. At any journal position exactly one of these
  holds: the JoR accepts commands itself (`Hosted`), or the JoR accepts none and
  N owns epoch `E` (`Spilled`).

### 4.2 Journaled entries

Four new entry kinds in the world journal. They use the existing signed-entry
envelope in `pkgs/id/src/world/` and the sequence number the journal already
assigns.

- `LeaseGrant { epoch, holder, start_seq, term_ms, granted_at_do }`
  - Written only by the JoR. `epoch` is strictly greater than every prior
    epoch. `start_seq` is the journal position from which N must replay before
    writing. `granted_at_do` is the DO's clock and is informational for the
    holder.
- `LeaseRenew { epoch, through_seq }`
  - Written by the JoR while N is alive and renewing. It extends the term. A
    missed renewal starts reclamation.
- `LeaseRelease { epoch, last_seq }`
  - Written by N at de-escalation, after its in-flight work is drained.
    `last_seq` is the last entry N wrote under `epoch`. This is N's request;
    the JoR decides whether to accept it.
- `LeaseRevoke { epoch, reason, last_seq }`
  - Written by the JoR when it reclaims: after release, or after term expiry
    plus margin. It closes `epoch`.

Every program-result entry written by N carries `epoch`. Entries the JoR writes
while `Hosted` are stamped with the epoch that most recently closed, so the
epoch sequence is contiguous in the journal.

### 4.3 The fence (append rule)

The journal accepts an entry only if all three hold:

1. If the writer is the JoR, the journal mode is `Hosted`.
2. If the writer is N, the journal mode is `Spilled` with `holder == N` and
   `epoch == entry.epoch`.
3. The entry's sequence number is the next position (no gap, no duplicate).

The check runs inside the JoR's single-threaded actor at the moment the entry
would be appended. Mode and sequence are read and updated in the same step, so
there is no window between check and append. Anything not matching is rejected
with a typed error (`StaleEpoch` or `NotHolder`). Nothing is appended and
nothing is acknowledged to the client.

### 4.4 Grant (Hosted to Spilled)

1. Spill is decided by the detector (#2275), outside this design.
2. The JoR stops admitting new commands into the actor queue. It keeps the queue
   for a short bounded pause; any command still queued afterward gets a
   retryable `Handoff` error. Clients retry with their idempotency key (Q2).
3. The JoR appends `LeaseGrant { epoch: E+1, holder: N, start_seq: head+1 }` and
   sets mode to `Spilled`.
4. N replays the journal from `start_seq`, then serves. If N's mirror lags the
   JoR, N replays up to the grant's journal head before it serves any command,
   and rejects commands until it has caught up.

### 4.5 Renew and expiry

- N renews every `term / 3`. The JoR appends `LeaseRenew` on each renewal it
  receives.
- Default term is 30 s (Q4). Renewal is every 10 s.
- The JoR reclaims only when `now > last_renew + term + skew_margin`, where
  `skew_margin` defaults to 10 s. The margin is measured on the DO's clock, so
  no node clock is trusted for the decision.
- N stops accepting commands when its own monotonic view says its last renewal
  is older than `term`. This is a liveness aid. Correctness does not depend on
  it, because the fence in §4.3 rejects N's writes after reclaim regardless.

### 4.6 Release (Spilled to Hosted)

1. Spill-end is decided by the detector: the warning band has cleared for about
   one hour (investigation §2). This design does not own that trigger.
2. The JoR tells N to drain. N stops admitting commands, finishes work already
   journaled, and rejects the rest with `Handoff`.
3. N appends `LeaseRelease { epoch: E+1, last_seq }`. The JoR accepts it only if
   `last_seq` is the journal head under `E+1`. Otherwise it is rejected and N
   must reconcile.
4. The JoR appends `LeaseRevoke { epoch: E+1, last_seq }` and sets mode to
   `Hosted`. The next JoR entry continues the sequence.

### 4.7 Reclaim without release (N unreachable)

If N is partitioned or crashed and never sends `LeaseRelease`, the JoR waits
until `term + skew_margin` has passed since the last renewal. It then appends
`LeaseRevoke { epoch: E+1, last_seq: head }` and resumes `Hosted`. Any entry N
later tries to append under `E+1` fails rule 2 of §4.3, because mode is no
longer `Spilled` with that holder. N's unjournaled work is lost. The client
never got an ack for it, so retry with the idempotency key is the recovery path.

### 4.8 Why split-brain is impossible (the argument)

Claim: no two entries that could both be accepted can conflict in the journal.

- The journal is written by one actor, the JoR's world actor. Appends are
  serialized in that actor, so the journal has a single total order of accepted
  entries.
- Accepting a JoR entry requires mode `Hosted`. Accepting an N entry requires
  mode `Spilled` with the same holder and epoch. Mode is a field of that single
  journal state and changes only through an appended grant or revoke.
- Therefore at every journal position at most one of these two states holds.
  Both writers are never accepted at the same position.
- N's acceptance depends on the JoR's journal, not on N's clock. The only way
  for N to write is for the JoR to have appended a grant naming N. A partitioned
  N with a wrong clock is still rejected, and it cannot create an accepted entry
  the JoR did not grant.
- Grant and revoke are themselves journal entries, so an auditor replaying the
  journal reads the holder for any position from the entries alone.

Assumptions, stated so they can be checked:

- A1. The JoR's world actor is single-threaded and its journal append is atomic
  (true for the DO model and for the native actor).
- A2. Entries are signed, so a forged `LeaseGrant` is rejected at replay. Only
  the JoR's key can write grants; N's key cannot.
- A3. The JoR does not lose its own journal. If it does, rung 3 is already
  broken by the durability model, not by this protocol.

If A1 fails (a second JoR actor for the same world), the argument fails. That is
the most important invariant to test (slice 2).

### 4.9 Failure walkthroughs

1. **N crashes holding the lease.** N stops renewing. The JoR reclaims after
   `term + skew_margin` (about 40 s by default) with a revoke at its head. N's
   unjournaled work is lost and clients retry. The journal stays contiguous with
   no duplicates.
2. **JoR and N partitioned, both alive.** N stops renewing. Its liveness aid
   stops it accepting commands after `term`. The JoR reclaims after the margin.
   If the partition heals, N's late writes hit the fence and get `StaleEpoch`.
   N must re-fetch the journal and rejoin as a follower before it can be granted
   again.
3. **Lease expires mid-turn.** N finishes the turn it is in, but its append is
   rejected because the mode is no longer `Spilled` for it. The turn's result is
   not journaled and not acked. This matches case 1 for that turn.
4. **Journal mirror lag at handoff.** N has not replayed up to the grant's
   `start_seq`. N refuses commands until it has caught up. The JoR holds the
   journal of record and is never behind, so no entries are lost. If N cannot
   replay within one term, the JoR does not grant to it.

### 4.10 In-flight command treatment at the boundary

- A command is **acknowledged** to the client only after its entries are
  journaled under the current mode. An unacknowledged command has no effect the
  client can rely on.
- At grant: commands queued at the JoR that have not reached the journal get a
  retryable `Handoff`. Commands already journaled before the grant are part of
  the history N replays.
- At release: N finishes journaled work, rejects new work with `Handoff`, and
  acknowledges nothing that is not journaled.
- Clients retry with an idempotency key so a retry after an ambiguous `Handoff`
  does not apply twice. The key is unique per command and journaled with the
  command's effects (Q2).

## 5. Build slices (dependency-ordered)

Each slice is sized for one lane session and has its own acceptance criterion.
Slice 1 is the precondition for all code. Slice 2 is the safety gate.

1. **Entry kinds and envelope.** Add `LeaseGrant`, `LeaseRenew`, `LeaseRelease`,
   `LeaseRevoke` to the journal entry type in `pkgs/id/src/world/`, using the
   existing signed envelope and sequence numbering. *Acceptance:* round-trip
   serialization and replay of an entry sequence with these kinds; signature
   check rejects a grant signed by a non-JoR key.
2. **Fence in the append path.** Implement the three rules of §4.3 in the
   single-writer actor, with typed `StaleEpoch` and `NotHolder` errors. Oracle:
   the mode/epoch state machine of §4.1 (model-based; the spec exists).
   *Acceptance:* model-based test over random interleavings of grant, renew,
   revoke, release and appends from JoR and N, asserting that no accepted
   position has two writers and that every accepted N entry carries the current
   epoch.
3. **Grant and handoff, Hosted to Spilled.** §4.4, including the bounded queue
   pause and the `Handoff` retry. *Acceptance:* integration test where a world
   spills to an in-process N, and the journal replays to the same state on both
   sides.
4. **Renew, expiry and reclaim.** §4.5 and §4.7 with the DO-clock margin.
   *Acceptance:* injected partition tests for failure walkthroughs 1 and 2
   (§4.9): reclaim happens within `term + skew_margin`, and a late write from N
   is rejected with `StaleEpoch`.
5. **Release, Spilled to Hosted.** §4.6 with the `last_seq` check and drain.
   *Acceptance:* round-trip spill and return with no lost or duplicated journal
   entries; the journal before and after release is identical for the shared
   prefix.
6. **Idempotency keys for `Handoff`.** Q2. *Acceptance:* a retried command after
   an ambiguous `Handoff` is applied exactly once in the journal.
7. **Operator controls and metrics.** Expose lease epoch, holder and mode through
   `id world stats` (investigation §2 signals). Spill is enabled only behind an
   explicit operator flag until slices 1 to 6 pass. *Acceptance:* stats show the
   current epoch and holder; spill refuses to start with the flag off.

## 6. Rollout and rollback

- **Rollout:** behind an operator flag, default off (slice 7). Enable first for
  one named test world under observation, then for hot worlds that meet the
  sustained critical band from investigation §2.
- **Rollback during spill:** the JoR issues `LeaseRevoke` with `reason =
  operator`. N drains as in §4.6. If N is unreachable, the reclaim path in §4.7
  applies after the margin.
- **Rollback of code:** the new entry kinds are additive. A world with no grant
  entries replays exactly as before, so disabling the flag does not change the
  journal format.

## 7. Verification plan

- Model-based test of the mode/epoch state machine (slice 2). Oracle: the state
  machine in §4.1 and the rules in §4.3.
- Property tests over journal sequences (slices 1 and 5): contiguity and no
  duplicate sequence numbers. Oracle: the sequence the actor assigns.
- Partition integration tests for §4.9 walkthroughs 1 to 4 (slices 3 and 4).
- Live proof, once slice 7 is done: spill a test world under synthetic load,
  return it, and show the stats epoch increments with no lost acknowledgement.
  Oracle: the client-side ack log matches the replayed journal.

## 8. Open questions

Each question has a default chosen now, so no implementation waits on it. The
decider is named so the default can be overturned.

- **Q1. Who signs grants?** Default: only the JoR's world key signs
  `LeaseGrant`, `LeaseRevoke` and `LeaseRenew`. Decider: the ID platform design
  record, ratified through the #2046 synthesis.
- **Q2. Idempotency key format for `Handoff` retries.** Default: a
  client-generated UUID per command, stored with the command's journaled
  effects, scoped to the world. Decider: the ID command-protocol owner, through
  #2046.
- **Q3. Queue pause length at grant.** Default: at most one lease term (30 s)
  before queued commands get `Handoff`. Decider: the spill owner, after slice 3
  measures it.
- **Q4. Term and skew margin values.** Default: term 30 s, renewal every 10 s,
  skew margin 10 s. Decider: the spill owner, after slice 4 measures real
  partition behaviour. The investigation §2 bands are starting points, not final.
- **Q5. Can the JoR be a native node?** Default: no. The DO remains JoR; a native
  node only ever holds an epoch. Making the native node the JoR is a different
  design and is out of scope. Decider: the platform design record.

## 9. Recorded follow-ups

- Investigation §7 already names the lease protocol as a follow-up. This record
  is that follow-up.
- The buildable slices in §5 should be filed as their own issues, each linked to
  #2819, with slice 1 blocking slices 2 to 7 and slice 2 as the safety gate.
  Filing is done by the issue owner in the same pass as this doc.
- Anything the detector (#2275) needs from this protocol (for example, a windowed
  queue-depth maximum) stays with #2275.
