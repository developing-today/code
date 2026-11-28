# F12.5 `kv` provider — design for the first slice of #1062

Issue: developing-today/code#1062 (F12.5 first new providers). The
2026-11-21 decision on that issue fixes the order — `kv` first, then
`blob`; `http.fetch` and `notify` after — and the exit shape: each
provider has bound, refusal, and replay tests. This document covers
only the `kv` slice.

## Shape

Three request capabilities, following the `schedule.at` /
`schedule.cancel` naming and execution pattern:

- `kv.get` — args `{key}`; result `{"key", "value"}` where `value` is
  the stored JSON value, or `null` when the key is absent.
- `kv.set` — args `{key, value}`; result `{"key", "value_bytes"}`.
  Overwriting an existing key is a replacement: it is validated
  against the store as it will be after the write, so a replacement
  never consumes a second slot or double-counts bytes.
- `kv.delete` — args `{key}`; result `{"key", "deleted"}`, `false`
  when nothing was stored under the key (mirrors `schedule.cancel`).

No new Wasm import: requests travel in the existing `wants` document,
results in the existing result events. Each use passes the standard
grant/ceiling/scope decision in the actor's `execute`, which writes
the audit entry per use, exactly as every other provider.

## Bounds (constants in `id-caps` `world_caps`)

- Key: 1 to `MAX_KV_KEY_BYTES` (256) bytes, printable ASCII — the
  `validate_schedule_id` rule shape, via `validate_kv_key`.
- Value: at most `MAX_KV_VALUE_BYTES` (8 KiB) serialized — the
  `MAX_RECORD_VALUE_BYTES` precedent for records.
- At most `MAX_KV_ENTRIES` (1024) entries per world.
- At most `MAX_KV_TOTAL_BYTES` (64 KiB) per world, charged as key
  bytes plus serialized value bytes per entry. The total is sized
  against the checkpoint format, not against appetite: a checkpoint
  declares the store in its scalar remainder, which is bounded by
  `MAX_CHECKPOINT_PART_BYTES` (192 KiB), so the cap keeps a full
  store's declaration — at most twice the charge once JSON escaping
  is counted, plus per-entry framing — inside that bound alongside
  the other scalar fields. A world can therefore always compact its
  journal, whatever it stored.

Bounds are enforced twice, like schedules: at request time by
`WorldCore::check_kv_set` (the program sees `bad_args` with a reason)
and again at journal replay, where an out-of-bounds entry refuses the
restore instead of silently widening the store. Journal appends are
also charged against the per-world total memory budget (#557) by the
existing persist path.

## Durability

Store state lives in `WorldCore` (`kv: BTreeMap<String, Value>`) and
is journaled: `JournalEntry::KvSet` before a set's result is
delivered, `JournalEntry::KvDeleted` before a delete's result. New
entry kinds require journal version 13 (`JOURNAL_VERSION_KV`); older
binaries refuse such a journal at the header check rather than
restoring without the store. Checkpoint compaction mirrors the
schedule treatment: the writer emits one `KvSet` per live entry
before the checkpoint, the checkpoint carries the declared entries
(`kv` field, absent when empty), and replay refuses a checkpoint
whose declared entries disagree with the `KvSet` entries before it.

## Registry

`kv.get` / `kv.set` / `kv.delete` register in the F12.4 provider
registry as version-1 host requests, `RequiredScope::Join`,
`DefaultCeiling::Included` — inward-facing like `records.read` and
`schedule.*`. (The off-by-default requirement in the #1062 decision
applies to `http.fetch` and `notify`, the outward-facing pair.)
The `value` argument is declared with the schema's `Object` kind:
values are arbitrary JSON, and the executor bounds the serialized
size; the schema kind names the structured-payload slot.

## Tests (the #1062 exit shape, for this provider)

- Bound: `kv_bounds_refuse_each_violation_with_a_reason` (core) and
  `validate_kv_key` cases (caps).
- Refusal: ungranted and outside-ceiling `kv.set` actor tests —
  refused, counted, and nothing stored or journaled.
- Replay: set/delete journal entries restore the store;
  checkpoint + compaction round-trips it; a checkpoint that
  disagrees with its `KvSet` entries is refused; a journal past the
  entry/total bounds is refused at restore.
