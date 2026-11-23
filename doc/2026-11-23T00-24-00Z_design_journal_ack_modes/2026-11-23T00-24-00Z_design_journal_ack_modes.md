# Selectable journal acknowledgement modes for world input

Issue: [#557](https://github.com/developing-today/code/issues/557) (the
remaining p99 latency criterion). Decision of record: Drewry, 2026-11-22
([comment 7087900634](https://github.com/developing-today/code/issues/557#issuecomment-7087900634)) —
async/batched acknowledgement is approved as **selectable modes**, not a
default flip; the durability contract stays the default; the tradeoffs
are documented where the modes are defined; the fsync/syncfs
group-commit tradeoffs are discussed in the same pass.

## Intent

`input_ack` waits for the journal sync in both the single and the batch
path: the actor persists an input through `WorldJournal::append` /
`append_batch` and acknowledges it only when the append returns, and
`FileJournal`'s append returns only after `write_all` + `sync_data`.
That is the documented `WorldJournal` contract — acknowledged history
must never get ahead of durable history. The post-Fix-1 laptop profile
shows what the contract costs: `input_ack_p50` 18.892 ms against a
journal append mean of 25.784 ms under load — the ack essentially *is*
the sync wait.

Two earlier attempts to cut that cost without touching the contract
were measured and closed:

- **Group commit via `syncfs`** (Fix 2, draft PR #12496): one `syncfs`
  costs ~2.5× one `fdatasync` on the fleet-runner filesystem, ~1.26× on
  sandbox btrfs, and 1.07× median / 1.44× p99 on the reference laptop's
  ext4. A shared sync only pays when it is cheaper than the sync it
  replaces; it is not, anywhere measured. Closed.
- **Group commit via `fdatasync` at the writer**: at the F12.12
  operating point, same-journal append queueing is zero — a journal's
  only producer is its owning actor, blocked until the in-flight append
  is durable, so a drain-and-sync writer forms batches of exactly one.
  There is nothing to coalesce. Closed on measurement.
- **Durability classes** (split inputs into must-be-durable and
  ephemeral): audited and stopped — the journal layer has no ephemeral
  class in the ack path; every journaled entry kind is replay
  load-bearing by design, and the frames the shell contract classifies
  ephemeral are never journaled at all. Inventing a class would weaken
  designed contracts. Not pursued.

What remains is the lever this document specifies: let a world *choose*
to relax the ack, per configuration, with the relaxation explicit,
bounded, and visible — instead of pretending the sync can be shared or
the inputs reclassified.

## The three durability levers, side by side

1. **The sync primitive** (per append): `fdatasync` (`File::sync_data`)
   on the journal file. Unchanged by this design. `syncfs` was priced
   against it on three filesystems and lost on all three (above); the
   question reopens only if `syncfs` gets cheaper than `fdatasync`.
2. **Sync sharing** (group commit): one sync covering many appends.
   Sound only where appends queue; per-journal concurrency is 1 by
   construction (single owning actor, rendezvous append), and the
   actor-level drain into `append_batch` already lands a batch with a
   single sync when a batch exists. Measured headroom: zero. Not
   implemented; `append_batch`'s one-sync-per-batch stands.
3. **Ack timing** (this design): *when* the input is acknowledged
   relative to the sync that covers it. This is the only lever that
   reduces syncs per input without sharing or reclassifying, and it is
   a durability-semantics choice — which is why it is a mode the world
   opts into, never a silent default change.

## The modes

The mode is a property of the journal, selected per world at open time
(`WorldLimits::journal_ack_mode`, or `FileJournal::open_with_ack_mode`
directly). It is runtime configuration, not journaled state: the entry
bytes, the journal format, and the recovery scan are identical in every
mode, so reopening a journal under a different mode re-reads exactly
the entries that survived — a mode can change what a *future* ack
promises, never what existing bytes mean. The mode is observable:
`WorldJournal::ack_mode()` (and `FileJournal::ack_mode()`) reports it,
and `FileJournal::unsynced_appends()` reports how many acknowledged
appends are not yet covered by a completed sync.

### `sync` — the default, unchanged

Every append (and every batch, as a whole) is durable before its
acknowledgement returns: one `write_all` + one `sync_data` per append,
one sync per `append_batch`. Acknowledged ⇒ durable, unconditionally.
This is the pre-existing contract, byte-for-byte: the default mode
writes exactly the bytes the old code wrote and syncs at exactly the
points the old code synced.

- **Guarantee:** an acknowledged input survives any crash.
- **Cost:** one sync latency per input on the ack path (the measured
  p99 problem).
- **Choose it** unless a world has a measured reason not to. It is the
  only mode that keeps the original contract.

### `batched` — bounded relaxation

Appends write immediately but sync in batches: the journal issues one
`sync_data` per `JOURNAL_ACK_BATCH_SIZE` (64) appends, counted across
`append` and `append_batch` entries. An append acknowledged before its
batch's sync is written but not yet durable; once the batch sync
completes, every append it covers is durable. A sync point (an explicit
`WorldJournal::sync()`, a checkpoint/compaction, a transaction) flushes
any partial batch.

- **Guarantee:** acknowledged ⇒ durable, *except* a tail of at most
  `JOURNAL_ACK_BATCH_SIZE - 1` acknowledged appends (plus the one in
  flight) whose batch has not synced yet. Recovery after a crash sees
  a prefix of the acknowledged history: entries are lost only from the
  unsynced tail, never from the middle, and never reordered.
- **Cost of the relaxation:** an OS crash or power loss can lose up to
  one batch of acknowledged inputs. A *process* crash loses nothing —
  written bytes survive in the page cache; only an OS-level failure
  drops unsynced data.
- **Choose it** for high-rate worlds whose inputs are individually
  cheap to lose in a bounded tail (presence-like streams, rapid game
  inputs) and whose bottleneck is sync latency: it pays one sync per
  64 inputs instead of one per input. At one sequential producer the
  win is sync *count*, not sync sharing.

### `async` — unbounded relaxation

Appends write and acknowledge without syncing at all. Durability
arrives only at sync points: an explicit `WorldJournal::sync()`, a
checkpoint/compaction, or a transaction (the rewrite paths always sync
in full, in every mode).

- **Guarantee:** acknowledged ⇒ written to the OS, in order. Nothing
  more. Recovery after an OS crash sees a prefix of the acknowledged
  history, of unbounded length (in practice bounded by how often the
  world checkpoints: `checkpoint_every` commits trigger a compaction,
  which syncs).
- **Cost of the relaxation:** an OS crash or power loss can lose every
  input since the last sync point.
- **Choose it** only for worlds whose journal is a cache of state that
  can be rebuilt or re-derived — throwaway rooms, dev worlds — never
  for history anyone will audit.

### What no mode changes

- **Transactions** (`append_transaction`) and **compaction** always
  build, sync, and rename a complete journal, in every mode: atomicity
  is not an ack-timing question.
- **The journal format and recovery.** Same JSONL entries, same
  torn-tail rule (an unterminated final line was never acknowledged
  and is truncated), same corruption refusal. A journal written under
  one mode replays identically under any other.
- **Ordering.** Appends land in call order in every mode; a sync
  always covers a prefix.
- **Replication honesty.** The replication feed reads written journal
  bytes, so in `batched`/`async` mode a replica can receive an entry
  before it is durable on the source — and, after a source OS crash,
  can hold an entry the source's recovered journal no longer has.
  Worlds that replicate for durability (rather than for liveness)
  should stay in `sync` mode.

## Measurement status

This change makes no new performance claim. The deciding measurement
for the #557 criterion remains the unloaded laptop re-run of the
F11.8 many-worlds harness, still outstanding; the expectation that
relaxed modes cut the ack's sync wait is an inference from the loaded
profile's components (`input_ack_p50` 18.892 ms vs journal append mean
25.784 ms), labelled as such, and no loaded number is substituted for
the baseline.

## References

- Issue [#557](https://github.com/developing-today/code/issues/557) —
  profiler record, Fix 1 (PR #12499), Win A (PR #12552), the deciding
  loaded laptop run (PR #12503), the lane A/A2/A3/A4 evidence comments,
  and Drewry's decision (comment 7087900634).
- `pkgs/id/crates/id-worlds/src/world/journal.rs` — `JournalAckMode`
  and the `WorldJournal` contract the modes qualify.
- `pkgs/id/crates/id-worlds/src/world_store.rs` — `FileJournal`, the
  mode's implementation and `JOURNAL_ACK_BATCH_SIZE`.
- `pkgs/id/benches/BASELINES.md` — the recorded measurements.
