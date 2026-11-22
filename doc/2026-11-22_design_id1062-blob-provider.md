# F12.5 `blob` provider — design for the second slice of #1062

Issue: developing-today/code#1062 (F12.5 first new providers). The
2026-11-21 decision on that issue fixes the order — `kv` first
(landed, PR #12489), then `blob`; `http.fetch` and `notify` after —
and the exit shape: each provider has bound, refusal, and replay
tests. This document covers only the `blob` slice, and follows the
`kv` design (`doc/2026-11-22_design_id1062-kv-provider.md`) wherever
the two providers share a question.

## Shape

Three request capabilities, following the `kv` / `schedule.at`
naming and execution pattern, over one bounded, durable, per-world
set of content-addressed blobs:

- `blob.put` — args `{"data"}` where `data` is the content as a
  base64 string; result `{"hash", "size"}`. The hash is the
  content's iroh-blobs address (`iroh_blobs::Hash::new(bytes)`,
  spelled as its 64-character lowercase hex), so a blob a world
  holds names the same content in the node's iroh-blobs store —
  `world_records` addresses stored content with `Hash::new` the
  same way, and the blob gate's reference index can name a world's
  blob by that hash. Putting content the world already holds is a
  no-op: the address is the same, so it consumes no second slot or
  byte charge and journals nothing.
- `blob.get` — args `{"hash"}`; result `{"hash", "data"}` where
  `data` is the base64 content, or `null` when the world holds no
  blob under that hash (mirrors `kv.get`).
- `blob.delete` — args `{"hash"}`; result `{"hash", "deleted"}`,
  `false` when nothing was stored under the hash (mirrors
  `kv.delete`).

No new Wasm import: requests travel in the existing `wants`
document, results in the existing result events. Each use passes
the standard grant/ceiling/scope decision in the actor's `execute`,
which writes the audit entry per use, exactly as every other
provider.

## Bounds (constants in `id-caps` `world_caps`)

- Content: at most `MAX_BLOB_BYTES` (4 KiB) per blob. The bound is
  set by the channel, not by appetite: content travels base64
  inside request args (serialized args are capped by
  `MAX_ARGS_BYTES`, 8 KiB) and inside result events (capped by
  `MAX_EVENT_BYTES`, 64 KiB). Base64 of 4 KiB is
  `MAX_BLOB_ENCODED_BYTES` (5,464) characters, so a maximal `put`'s
  args are about 5.5 KiB — inside the args cap with the envelope —
  and a maximal `get` result is about 5.6 KiB, far inside the event
  cap. 4 KiB is the largest power-of-two content with that property
  on the put side: 6 KiB of content encodes to 8,192 characters,
  the entire args budget before any framing. Larger blobs need a
  chunked or streamed transfer the `wants` channel does not have;
  that is follow-up work, not this slice.
- At most `MAX_BLOBS` (256) blobs per world. The checkpoint
  declares the blob index (below), about 85 bytes of JSON per
  entry, so a full index declares in about 22 KiB — inside the
  checkpoint scalar bound (`MAX_CHECKPOINT_PART_BYTES`, 192 KiB)
  alongside a full `kv` declaration and the pending schedules.
- At most `MAX_BLOB_TOTAL_BYTES` (256 KiB) of content per world,
  charged as content bytes per blob. Journal appends are also
  charged against the per-world total memory budget (#557) by the
  existing persist path; the total cap keeps a world's journaled
  content bounded independently of the count and size caps, whose
  product would allow 1 MiB.
- Blob ids: exactly 64 lowercase hex characters
  (`validate_blob_hash`) — the iroh-blobs `Hash` spelling. No other
  string reaches the journal as a blob id.

Bounds are enforced twice, like `kv`: at request time by
`WorldCore::check_blob_put` (the program sees `bad_args` with a
reason) and again at journal replay, where an out-of-bounds entry
refuses the restore instead of silently widening the store.

## Durability

Blob state lives in `WorldCore` (`blobs: BTreeMap<String, Vec<u8>>`,
hash hex to content) and is journaled: `JournalEntry::BlobStored`
before a put's result is delivered, `JournalEntry::BlobRemoved`
before a delete's result (and only when the blob was present). New
entry kinds require journal version 14 (`JOURNAL_VERSION_BLOB`);
older binaries refuse such a journal at the header check rather
than restoring without the blobs.

Content rides the journal rather than being placed in the node's
iroh-blobs store by the executor, for the same reason `kv` values
do: world state must replay from the journal alone. Replicas
restore from a verified journal without a store connection, and
`WorldCore::restore` is a pure function of the entries; the actor's
execute path is synchronous and holds no store handle. The
*address* is the iroh-blobs hash, so nothing about the naming
changes if the bytes later move behind the store: the journal
would then carry the same put/remove facts and the same bounds,
and this slice's replay checks would verify presence instead of
content. That move is the follow-up if blobs outgrow the channel
bounds above.

Replay re-checks every bound and, unlike `kv`, the content address
itself: a `BlobStored` whose decoded content does not hash to its
stated hash refuses the restore. Content addressing makes the
journal self-verifying — a corrupted or tampered entry is a
refusal, not a silently different blob. A `BlobRemoved` of a blob
that is not stored also refuses the restore (the `kv` delete
treatment).

Checkpoint compaction differs from `kv` in one deliberate way. The
writer emits one `BlobStored` per live blob before the checkpoint,
as for `kv`, but the checkpoint declares only the index — a `blobs`
field of `{hash, size}` per live blob, absent when the set is
empty — not the content: a full set's content declaration would
exceed the checkpoint scalar bound on its own. Replay refuses a
checkpoint whose declared index disagrees with the `BlobStored`
entries before it (hash set and sizes must match exactly), so the
declaration still pins what compaction must preserve; the entries
carry the bytes, and each is hash-verified as it replays.

## Registry

`blob.put` / `blob.get` / `blob.delete` register in the F12.4
provider registry as version-1 host requests, `RequiredScope::Join`,
`DefaultCeiling::Included` — inward-facing like `kv.*` and
`records.read`. (The off-by-default requirement in the #1062
decision applies to `http.fetch` and `notify`, the outward-facing
pair.) The `data` and `hash` arguments are declared with the
schema's `String` kind: base64 content and a hex address are both
strings on the wire, and the executor bounds the decoded size.

## Tests (the #1062 exit shape, for this provider)

- Bound: `blob_bounds_refuse_each_violation_with_a_reason` (core)
  and `blob_hashes_are_64_lowercase_hex` (caps).
- Refusal: ungranted and outside-ceiling `blob.put` actor tests —
  refused, counted, and nothing stored or journaled.
- Replay: put/delete journal entries restore the set; a journal
  entry whose content does not hash to its stated hash is refused;
  checkpoint + compaction round-trips the set; a checkpoint that
  disagrees with its `BlobStored` entries is refused; a journal
  past the count/total bounds is refused at restore; actor round
  trip including the duplicate-put no-op.
