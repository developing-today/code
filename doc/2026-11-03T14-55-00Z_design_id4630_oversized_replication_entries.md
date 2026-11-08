# Design: replicating a journal whose `Checkpoint` entry exceeds the payload bound

- Date: 2026-11-03
- Status: **design** — no implementation is included in this change.
- Issue: [#4630](https://github.com/developing-today/code/issues/4630), "id(world-replication): a journal whose Checkpoint line exceeds 256 KiB can never be replicated — split_entry_payload refuses the single entry and send_snapshot dies before the first frame".
- Parent design: [#806](https://github.com/developing-today/code/issues/806) / `doc/2026-11-01_design_id806.md` (F12.13 journal replication), whose §4 contract this record extends without changing the wire.
- Admission contract: [#4592](https://github.com/developing-today/code/issues/4592) and its phase 3 [#4593](https://github.com/developing-today/code/issues/4593) — _a verified cursor covers only a restorable chain_.
- Evidence baseline: `origin/main` at `e7510b90a`, worktree `~/git/fleet/6557-4630-oversized-entry-replication`, 2026-11-03.

## Decision to make

`pkgs/id/src/world/replication.rs` fixes the v1 replication payload at
`MAX_REPLICATION_PAYLOAD_BYTES = 256 * 1024` and splits multi-entry payloads only at
entry (newline) boundaries; a single entry over that cap returns
`ReplicationError::PayloadTooLarge`, documented as an entry that "can never be
replicated in v1". A compacted `JournalEntry::Checkpoint` is one such entry, and
`send_snapshot` (`pkgs/id/src/world_replication_net.rs:206-243`) calls
`split_entry_payload` before the first frame, so the whole feed dies.

The task 6557 packet-defect block was right: the v1 envelope has no continuation
mechanism, so _how_ to make oversized checkpoints replicable is a contract choice.
This note evaluates the three candidates named in #4630's acceptance, against the
v1 envelope/cursor/hash-chain contract, `ReplicationTracker::accept`, and #4592's
receiver-admission contract, and picks one.

**Recommendation:** bound the checkpoint **at write time** (mechanism c) by emitting
it as a sequence of bounded entries, with a part size _below the existing 256 KiB
payload bound_. Every entry the writer produces then fits one v1 replication frame,
no wire-version bump is needed, no receiver upgrade is needed, and the v1 invariant
("a payload is a sequence of complete journal entries; the cursor is entries
absorbed") stays exactly true. Fragmentation (a) is rejected as a permanent
continuation protocol for a single writer-controlled entry kind; a larger in-frame
allowance (b) is deferred as optional transport headroom, because on its own it only
moves the cliff and cannot bound a checkpoint.

## Current state (evidence)

- **Payload bound and split rule:** `pkgs/id/src/world/replication.rs:74-88`
  (`MAX_REPLICATION_PAYLOAD_BYTES`, `MAX_REPLICATION_FRAME_BYTES`),
  `:167-197` (`split_entry_payload`: splits at newlines, errors
  `PayloadTooLarge` when one entry exceeds the cap).
- **Cursor/chain rule:** `chain_payload` (`replication.rs:355-380`) refuses a
  payload over the cap or not ending at an entry boundary, and decodes every line as
  a `JournalEntry` before advancing the cursor; the module doc (`:44-56`) states a
  payload carries exact journal-entry bytes, never a projection.
- **Receiver:** `ReplicationTracker::accept` (`replication.rs:705-760`) re-checks
  `payload.len() > MAX_REPLICATION_PAYLOAD_BYTES` at `:735`, so a receiver on any
  existing binary refuses a larger frame regardless of sender intent.
- **Sender:** `send_snapshot` (`world_replication_net.rs:206`), `split_entry_payload`
  at `:215`, error propagated before any frame is written; the laptop probe on
  2026-11-02 (300,000-byte checkpoint line → `PayloadTooLarge`; 262,144 passes,
  262,145 fails) matches the constants exactly.
- **The offending entry:** `WorldCore::checkpoint_entries`
  (`pkgs/id/src/world/core.rs:1063-1200`) builds one `Checkpoint` holding
  `snapshot: String` (the program's own serialization), `events: Vec<WorldEvent>`
  (`retained_events`, default 1024), `admin_history` (up to
  `min(retained_events, MAX_ADMIN_HISTORY_EVENTS=4096)`), `open_presence`
  (up to `MAX_CHECKPOINT_OPEN_PRESENCE=4096`, `world/mod.rs:92-100`), plus scalars,
  `transfers`, `schedules`, `caps_ceiling`.
- **Nothing bounds the checkpoint:** `WorldLimits::snapshot_bytes` (default
  512 KiB, `world/mod.rs:119,141`) bounds only the client-facing snapshot's event
  selection (`core.rs:2063`); the program snapshot string handed to
  `checkpoint_entries` at `pkgs/id/src/world/actor.rs:275` has **no** `WorldLimits`
  bound. With `retained_events=1024` and `input_bytes=16 KiB` (hex-encoded entity
  input events double that), a checkpoint can exceed 1 MiB as well as 256 KiB, so
  mechanism (b) alone cannot make the guarantee total.
- **Every other entry kind is bounded:** chat ≤ `chat_bytes` (2 KiB), input ≤
  `input_bytes` (16 KiB, ~32 KiB encoded), participant/ceiling/schedule entries are
  small; `Checkpoint` is the only entry kind that can be arbitrarily large. This is
  what makes a checkpoint-only fix sufficient.
- **Journal version contract:** `JOURNAL_VERSION`=1, current
  `MAX_JOURNAL_VERSION`=5 (`world/mod.rs:984-1019`); new entry kinds require a
  header-version bump _before_ the entry is written, so an older binary refuses at
  the header check (`:1021-1060`), never mid-read.

## Options

Scoring: (1) does it make the guarantee total, (2) what does it cost the v1
contract and #4592's admission argument, (3) compatibility, (4) code surface.

### (a) Sub-entry fragmentation / continuation frames

Add envelope fields for `fragment_index` / `fragment_count` (or
`entry_seq` + `byte_offset` + `entry_len`), let a payload be a byte range rather
than whole entries, advance the cursor only on the final fragment, and have the
receiver buffer a partial entry before admitting it.

- **Totality:** yes, up to available memory — any entry fits eventually.
- **Contract cost (high):** the v1 cursor is "entries absorbed"; fragments introduce
  a state where the sender has committed bytes to the chain that the receiver may not
  yet admit. #4592's rule ("a verified cursor covers only a restorable chain") survives
  only if the tracker additionally guarantees no partial entry is ever treated as
  verified — reachable, but it makes `accept` a six-state machine (mid-entry,
  digest-partial, etc.) and adds a new receiver-side buffering surface that the
  admission audit must then disposition. Every accepted v1 guarantee has to be
  re-proved under a second payload grammar.
- **Compatibility (high):** `accept` refuses any `version != REPLICATION_ENVELOPE_VERSION`
  (`replication.rs:709`), so this is a **v1→v2 wire bump**: sender and receiver must
  agree, old replicas cannot read the new feed, and the module's domain/magic tags
  must bump with it. That is a Drew-owned protocol decision, not a worker one.
- **Code surface (high):** envelope, `split_entry_payload`, `chain_payload`,
  `payload_digest`, `validate_payload_journal_header` (its "first entry must be a
  `Created` header" check needs a reassembled-entry notion), `ReplicationTracker`,
  `world_store` receiver, and all wire tests.
- **Verdict: reject** as the primary fix. It buys generality for exactly one
  writer-controlled entry kind, at a wire bump and a permanent receiver-buffering
  surface. Revisit only if entries can ever become unbounded in ways the writer does
  not control (e.g. a future entry kind carrying opaque third-party bytes).

### (b) Larger single-entry allowance within the 1 MiB transport frame

Raise `MAX_REPLICATION_PAYLOAD_BYTES` to `MAX_REPLICATION_FRAME_BYTES` minus a
measured envelope overhead (magic + postcard envelope + 64-byte signature), with a
test asserting a maximal payload still fits one transport frame.

- **Totality: no.** The residual is real and large: the program snapshot string has
  no bound at all, and retained/admin events alone can reach ~32 MiB worst case
  (1024 × ~32 KiB). 1 MiB moves the cliff; it does not remove it, and #4630 asks for
  a guarantee with "no silent remainder".
- **Contract cost (low):** the written grammar is unchanged (still complete-entry
  payloads, same cursor).
- **Compatibility (medium, and subtle):** no envelope-version bump is _format_-legal,
  but the constant is a receiver-side limit too (`accept`, `:735`): a sender that
  uses the extra headroom sends frames every existing v1 receiver rejects as
  `PayloadTooLarge`. There is no capability negotiation in v1, so raising the
  constant silently splits the fleet. Raising it is therefore effectively a
  compatibility decision, and if adopted at all it should be after the fix makes the
  raise unnecessary.
- **Verdict: defer as optional headroom**, not as the fix. If chunk count or
  per-frame overhead ever measures badly, raise it in a separate change with an
  explicit compatibility note (and treat the receiver limit as a negotiated
  capability, not a silent constant).

### (c) Bounded checkpoint at write time

Make compaction emit the checkpoint as a **sequence of bounded entries**: the
scalars stay on `Checkpoint`; the variable-size payloads move to preceding part
entries, mirroring the existing pattern where `CapsCeiling`, `ScheduleSet`,
`Isolation`, `ExecutionProfile` and transfer state are emitted _before_ the
`Checkpoint` that summarizes them (`core.rs:1063-1200`):

- `CheckpointSnapshotPart { sequence, part_index, part_count, bytes }` — the program
  snapshot string, cut at UTF-8 boundaries every `MAX_CHECKPOINT_PART_BYTES`.
- `CheckpointEventsPart { sequence, part_index, part_count, events }` — the retained
  events, split by accumulated encoded size.
- `CheckpointAdminPart { sequence, part_index, part_count, events }` — the admin
  history, split the same way.
- `Checkpoint` keeps scalars plus `open_presence` (bounded at 4096 IDs ≈ 86 KiB
  encoded — see the per-entry bound below) and the flags that already have their own
  pre-entries.
- Restore assembles the parts before a `Checkpoint` for the same `sequence`, refuses
  a missing/duplicate/out-of-order part, and applies exactly the state it applies
  today. Compaction emits parts and never a single oversized checkpoint.

- **Totality: yes, for every journal this writer produces** — because the writer
  chooses the split. The one residual case is a journal _already on disk_ whose
  `Checkpoint` was written before this change (see "Residual" below).
- **Contract cost (low, and only at the journal layer):** the replication wire,
  cursor, hash chain, `accept`, and #4592's admission argument are untouched: payloads
  remain whole entries, the cursor remains entries-absorbed, and every entry is
  restorable by construction.
- **Compatibility:** new entry kinds → bump `JOURNAL_VERSION` to 6 per the existing
  contract (`world/mod.rs:980-1019`), so older binaries refuse the journal at the
  header check, as they already do for versions 2-5. Replication is unaffected: a
  v6 journal replicates over the same v1 envelope, and `header_version` in the
  envelope already carries 1..=`MAX_JOURNAL_VERSION`.
- **Code surface (medium):** a new entry kind (serde + version mapping), compaction
  emission, restore assembly, and tests. No network or tracker changes.
- **Verdict: choose this.**

## Chosen mechanism: bounded checkpoint (c), staged

**Stage 1 — land (c) with `MAX_CHECKPOINT_PART_BYTES = 192 * 1024`.**

192 KiB is deliberately _below_ the existing 256 KiB `MAX_REPLICATION_PAYLOAD_BYTES`,
leaving room for JSON framing of the part's scalar fields, so:

- no change to `MAX_REPLICATION_PAYLOAD_BYTES`, no envelope-version bump, no
  receiver upgrade, no capability negotiation;
- the v1 replication path needs **no code change at all** for the fix — it starts
  working because the writer stopped producing oversized entries;
- the on-disk journal also stops containing multi-megabyte JSONL lines, which is a
  second, independent robustness win for `world_store`.

**Stage 2 — residual handling for pre-existing journals (own issue).** A journal
compacted by an older binary can still hold one oversized `Checkpoint`, and
re-compaction is not automatic at open (`checkpoint_due` requires
`since_checkpoint >= checkpoint_every`, which resets on restore). Stage 2 detects an
oversized entry when the feed is built and either re-compacts immediately (when a
program snapshot exists, so compaction is legal) or returns an actionable
`PayloadTooLarge` diagnostic naming the entry and the bound instead of failing
opaquely in `send_snapshot`. This closes the only remaining unreplicable case
without touching the wire.

**Stage 3 — not proposed, recorded for completeness.** (b) may be revisited as pure
transport headroom once Stage 1 ships; it is not needed for the guarantee, and it is
a compatibility decision because existing receivers enforce the old constant.

**Why not (a):** it changes the wire contract to solve a problem the writer can
prevent for free. The v1 invariant that makes #4592's admission argument short — a
frame is a complete prefix of journal entries and the cursor is exactly the entries
absorbed — would become conditional on per-fragment receiver state. That complexity
belongs in the protocol only if entry size becomes genuinely outside the writer's
control.

## Resulting per-entry guarantee (for the replication module docs)

> Every journal entry the writer emits is at most `MAX_CHECKPOINT_PART_BYTES`
> (192 KiB) encoded, which is below `MAX_REPLICATION_PAYLOAD_BYTES` (256 KiB).
> Therefore every entry fits one replication payload frame, `split_entry_payload`
> never returns `PayloadTooLarge` for a journal this writer produced, and the
> snapshot/tail feed is fully replicable under the v1 envelope with no continuation
> mechanism.

The only entries not covered by that sentence are:

1. a `Checkpoint` written by a binary older than this change, on a journal that has
   not been re-compacted since (Stage 2 gives it an actionable path); and
2. a journal written by a _different_ writer implementation that violates the bound —
   out of scope, and the receiver still refuses it exactly as today.

No other entry kind can violate the bound: chat ≤ 2 KiB, input ≤ 16 KiB (≤ ~32 KiB
hex-encoded), participant/ceiling/schedule/isolation/profile entries are small, and
`open_presence` is capped at 4096 IDs (≈ 86 KiB encoded, under the 192 KiB part
bound). The implementation must assert these bounds in a test rather than leave them
as prose.

## Questions for Drew

- **Wire-version bump:** not needed by the chosen mechanism — explicitly recorded so
  no later change slips one in. If Stage 3 (raising the payload allowance) is ever
  approved, it changes the receiver-side constant and therefore the compatibility
  envelope; that is Drew's call, and it should be a negotiated capability rather than
  a silent raise.

## Implementation follow-ups

Filed from this note (linked on #4630; #4630 stays open until the implementation
merges):

1. **[#4966](https://github.com/developing-today/code/issues/4966) — primary — `id(world): bound every journal entry at write time by splitting the checkpoint`**
   — new part entry kinds, compaction emission, restore assembly, `JOURNAL_VERSION`
   6, part bound 192 KiB, plus the #4630 regression test (an oversized-Checkpoint
   journal image — program snapshot > 256 KiB — round-trips
   `split_entry_payload` → `send_snapshot` frames → `ReplicationTracker::accept` and
   lands on a matching cursor) and the existing suites
   (`cargo test -p id --lib world::replication`, the `world_store` replication feed
   tests, the journal-version/restore tests).
2. **[#4967](https://github.com/developing-today/code/issues/4967) — residual — replication diagnostics and re-compaction for legacy oversized
   checkpoints** — detect at feed build, re-compact when legal, otherwise return an
   actionable error naming the entry and the bound.

## Appendix: why the bound is stated as encoded size

The replication contract hashes and frames the **encoded** JSONL bytes
(`journal_entry_line`), and `split_entry_payload` measures those bytes. The part
sizes above are therefore encoded-size bounds, not field-size bounds; the
implementation should enforce the budget during emission by measuring
`serde_json::to_vec` of the assembled part, not by counting fields.
