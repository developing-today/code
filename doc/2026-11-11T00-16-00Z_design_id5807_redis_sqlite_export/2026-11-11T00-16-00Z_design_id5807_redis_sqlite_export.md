# Design: Redis→SQLite export + query-Redis-get-SQLite (from #5672)

- Date: 2026-11-11
- Status: **decision record** — design only; no implementation is included in
  this change.
- Issue: [#5807](https://github.com/developing-today/code/issues/5807),
  "id: design: Redis→SQLite export + query-Redis-get-SQLite (from #5672)".
- Parent design: [#5672](https://github.com/developing-today/code/issues/5672),
  "id: design: selective sharing + foreign data-store adapters",
  decomposition item 9 (its §B4 and §B5). Programme epic:
  [#2056](https://github.com/developing-today/code/issues/2056).
- Sibling designs this record is written _against_:
  [#5799](https://github.com/developing-today/code/issues/5799) (selective-sharing
  grants model, closed), [#5801](https://github.com/developing-today/code/issues/5801)
  (encrypted shares & key distribution, closed),
  [#5804](https://github.com/developing-today/code/issues/5804) (IndexedDB
  adapter, closed), [#5806](https://github.com/developing-today/code/issues/5806)
  (Turso/libsql HTTP adapter, closed),
  [#5800](https://github.com/developing-today/code/issues/5800) (partial
  replication, open), [#5802](https://github.com/developing-today/code/issues/5802)
  (public shares, open), [#5803](https://github.com/developing-today/code/issues/5803)
  (foreign data-store adapter contract, open),
  [#5805](https://github.com/developing-today/code/issues/5805)
  (Postgres-over-HTTP adapter, open).
- Evidence baseline: `origin/main` at `effdff265b`, 2026-11-11. Verified at
  that baseline: no decision record for #5807 exists in `doc/`, and the
  general foreign-adapter contract ticket #5803 is still open — §1.2 states
  what this record assumes from the parent body itself; if #5803's record
  lands differently, #5803 governs the contract and this record's adapter
  surface is re-read against it (its type mapping and lossy corners stand —
  they are properties of Redis and SQLite, not of the contract).

## 1. What is already fixed (not re-decided here)

### 1.1 From the parent #5672

- §B4 fixes the export's existence and shape: a one-shot **and repeatable**
  export that snapshots a Redis instance (keyspace scan; per-type mapping
  strings/hashes/lists/sets/zsets → tables/rows) into a SQLite database file
  the user can keep, query, and import into ID. Type-mapping rules and lossy
  corners (expiry/TTL, ordering, huge keys) are documented **in the export
  report**.
- §B5 fixes the query path's shape: an ad-hoc form of §B4 — run a
  query/selection against a live Redis instance and receive the result _as_
  a SQLite database (file or ID-hosted), shaped by the query. **Read-only
  against Redis by definition; no mutation path smuggled in.**
- Part B's preamble fixes the adapter-contract questions every adapter must
  answer: how the foreign store is named/addressed in ID, what schema/type
  mapping is promised, live vs. materialized, read-only vs. writable,
  credential handling (**never in ID data itself; referenced, not stored**),
  and error/latency behaviour when the foreign store is slow or gone.
- The v1 non-goals bind here: **no bidirectional sync** with foreign stores
  (adapters are read/materialize-first), and REDB remains ID's native store —
  nothing in this record makes SQLite a live ID store; the SQLite file is an
  export artifact and an import source, never the system of record.
- The parent's open questions name two this record must answer for its
  scope: how non-UTF-8/binary values are represented in SQLite output, and
  how key expiry is represented. §3 and §4.1 answer both.
- The parent's grants model (#5799), existence-privacy rule, and revocation
  boundary are **not** re-decided. They govern sharing of ID data; an export
  is an operator holding Redis credentials making a local copy, which the
  parent already frames as outside revocation's reach (revocation cannot
  un-copy bytes). Nothing here grants anyone anything.

### 1.2 Assumed contract surface (pending #5803)

Until #5803 lands, this record fixes only what §B4/§B5 need: the source is
addressed in ID as a **foreign-store reference** (a name resolving to a
connection description held outside ID data); credentials are referenced,
never stored, and never written into the SQLite output (§5.3); the adapter
is **materialized, read-only** (§2). Any richer contract vocabulary #5803
adopts renames this surface; it does not change the decisions below.

## 2. The chosen shape: one core, two operations

Both operations are the same pipeline — *select → scan → map → write SQLite

- report* — differing only in how the selection is made:

1. **Export** (`export`): the selection defaults to a whole logical database
   (one Redis DB index), optionally narrowed by a key pattern and/or type
   filter. Output: a SQLite file the user keeps.
2. **Query-get-SQLite** (`query`): the selection is an explicit, recorded
   selection spec (§5.1) against a live instance. Output: a SQLite database,
   as a file or ID-hosted, in **exactly the export schema** (§3) restricted
   to the selected keys.

Decisions:

- **Online scan, not RDB parsing.** The exporter talks to the live instance
  with `SCAN` (cursor iteration, bounded `COUNT`) and per-key reads. It
  never uses `KEYS`. RDB-file parsing is rejected as the primary path
  (§6): it needs filesystem access to the Redis host and cannot express a
  selection. It remains a legitimate future bulk path, not v1.
- **Honest snapshot semantics.** `SCAN` gives no point-in-time snapshot.
  The export is a _scan-window view_: it records start/end timestamps, the
  source identity (Redis `run_id`, host fingerprint, DB index), and
  `DBSIZE` at start and end in the report. A key mutated during the window
  may appear in its before or after state; a key visited twice by rehashing
  is deduplicated by key, last read wins. The report says this plainly
  rather than claiming snapshot isolation the source does not provide.
- **Read-only by construction.** The adapter issues only the enumerated
  read commands: `SCAN`, `TYPE`, `PTTL`, `GET`, `HGETALL`, `LRANGE`,
  `SMEMBERS`, `ZRANGE … WITHSCORES` (and `DBSIZE`/`INFO` for the report).
  No `EVAL`, no writes, no `CONFIG`, no `DUMP`-based write-back. Connecting
  with a read-only user / replica where available is deployment guidance;
  the command allowlist is the design-level guarantee, auditable in one
  place in the implementation.

## 3. Output schema and type mapping

One schema version, shared verbatim by export and query results
(`export_meta.schema_version`). All payloads are byte strings in Redis;
SQLite is typed. The mapping rule that resolves every case below:

> **Bytes are preserved exactly; text is a view, never a reinterpretation.**
> Every value (and every key) is stored once as `BLOB`, plus a `*_text`
> column that is non-NULL **iff** the bytes are valid UTF-8, in which case
> it holds exactly those bytes as text. The exporter never infers a numeric
> or boolean type from a string's contents.

### 3.1 Tables

- `export_meta(key TEXT PRIMARY KEY, value TEXT)` — exporter version,
  schema version, operation (`export`/`query`), the selection spec verbatim
  (for `query`), source identity (run_id, host fingerprint, DB index — no
  credentials, §5.3), scan start/end timestamps, `DBSIZE` at start/end.
- `keys(key_bytes BLOB PRIMARY KEY, key_text TEXT, type TEXT NOT NULL,
ttl_ms INTEGER, expires_at_ms INTEGER)` — one row per **exported** key.
  `type` is the Redis `TYPE` string (`string`, `hash`, `list`, `set`,
  `zset`). TTL columns are both NULL for persistent keys (§4.1).
- `string_values(key_bytes BLOB PRIMARY KEY REFERENCES keys, value BLOB
NOT NULL, value_text TEXT)`.
- `hash_entries(key_bytes BLOB REFERENCES keys, field BLOB NOT NULL,
field_text TEXT, value BLOB NOT NULL, value_text TEXT,
PRIMARY KEY (key_bytes, field))`.
- `list_items(key_bytes BLOB REFERENCES keys, idx INTEGER NOT NULL,
value BLOB NOT NULL, value_text TEXT, PRIMARY KEY (key_bytes, idx))` —
  `idx` is the Redis list index, so order is data, not scan order.
- `set_members(key_bytes BLOB REFERENCES keys, member BLOB NOT NULL,
member_text TEXT, PRIMARY KEY (key_bytes, member))`.
- `zset_members(key_bytes BLOB REFERENCES keys, member BLOB NOT NULL,
member_text TEXT, score REAL NOT NULL, score_text TEXT NOT NULL,
PRIMARY KEY (key_bytes, member))` — `score_text` preserves the exact
  score string Redis returned; `score` is its REAL projection (§4.4).
- `export_skips(key_bytes BLOB, key_text TEXT, reason TEXT NOT NULL,
detail TEXT)` — every selected key that was **not** exported, with a
  reason from the closed vocabulary in §4. Nothing selected is ever
  silently absent: it is in `keys` or in `export_skips`.

### 3.2 Type-by-type notes

- **Strings** are opaque bytes (they may hold integers, serialised blobs,
  JSON, or anything else) → `string_values`, uninterpreted.
- **Hashes** map field-by-field; field names get the same bytes+text
  treatment as values.
- **Lists** map by index; duplicate values are distinct rows (index is the
  key), as they are in Redis.
- **Sets** map member-per-row; duplicate members cannot exist in Redis and
  the primary key enforces the same here.
- **Sorted sets** map member-per-row with score; ordering is recovered by
  `ORDER BY score, member`, never by row/insertion order.
- **Streams and module types** (including RedisJSON) are **not mapped in
  v1**. A stream's entries would map naturally, but its consumer-group
  state has no honest SQLite projection, and a partial projection would
  look complete. Selected stream/module keys land in `export_skips` with
  reason `unmapped-type` and their `TYPE` string in `detail`. A follow-on
  design may add streams; silently dropping them may not happen (§4.6).
- Redis's internal encodings (listpack, intset, …) are **not** part of the
  mapping: the contract is at the logical type level, so an encoding change
  inside Redis never changes the output.

## 4. Lossy corners — stated, not smoothed over

### 4.1 Expiry / TTL

SQLite does not expire anything. Expiry is exported as **data**:
`keys.ttl_ms` (the `PTTL` observed when the key was read) and
`keys.expires_at_ms` (scan-time wall clock + TTL). Two consequences are
recorded, not hidden: (a) the values are approximate — TTL kept ticking
during the scan, so `expires_at_ms` is the exporter's best observation,
not a guarantee; (b) importing the file into ID imports the expiry as
metadata for the importer to honour or ignore — the export itself never
extends, shortens, or enforces a key's life. A key that expires between
`SCAN` and its read is skipped with reason `expired-during-scan`.

### 4.2 Ordering

Lists keep true order (index). Sorted sets keep score order via the score
columns. **Sets have no order in Redis**; their rows' physical order in
SQLite is scan order and means nothing — consumers must not read meaning
into it. This is stated in the export report template so the warning
travels with the artifact.

### 4.3 Binary and non-UTF-8 data

Answered per §3's rule: bytes exact in `BLOB`; `*_text` NULL when the
bytes are not valid UTF-8. This applies to keys as well as values, fields,
and members (`key_bytes` is the join key everywhere for exactly this
reason). Nothing is escaped, base64'd, or lossily transcoded anywhere in
the output.

### 4.4 Numbers

Redis strings are not numbers, and the exporter must not pretend
otherwise: a string `"007"` or `"9007199254740993"` exported as a SQLite
`INTEGER`/`REAL` would lose exactly the information that makes it that
string. Strings stay bytes/text (§3.2). The one genuinely numeric Redis
value, a **zset score**, is an IEEE-754 double in Redis: `score REAL`
holds it, and `score_text` preserves Redis's exact rendering, because
REAL→text round-trips are not guaranteed byte-identical. `+inf`/`-inf`
scores are stored in `score_text` and as SQLite `9e999`/`-9e999`
infinities in `score`, with the corner named here rather than discovered
later.

### 4.5 Huge keys and values

A configurable per-value size cap (implementation-set default, recorded in
`export_meta`) bounds any single value/field/member. An over-cap payload is
**never truncated**: the key is skipped with reason `oversize`, `detail`
carrying the true length, and an explicit operator flag is the only way to
include it. Silent truncation is the one failure this design forbids
outright — a partial value that looks whole is worse than an absent one.

### 4.6 Everything skipped is named

The skip-reason vocabulary is closed: `expired-during-scan`,
`unmapped-type`, `oversize`, `read-error` (with the error in `detail`).
The export report (§5.2) reconciles counts: selected = exported + skipped,
by type and by reason. An export whose counts do not reconcile is a failed
export, not a partial success.

### 4.7 Empty aggregates

Redis cannot hold an empty hash/list/set/zset (the key ceases to exist),
so the output never needs an "empty collection" row: an aggregate key is
either present with ≥1 element row or absent (and, if selected, in
`export_skips`). No special case exists, and none may be invented by an
implementation "for symmetry".

## 5. The query path, the report, and credentials

### 5.1 Selection spec (the "query")

The v1 query is a **selection**, not a command language and not SQL over
Redis: `{ db, pattern (Redis glob), types[], keys[] (explicit list),
include_ttl (default true) }`. It is recorded verbatim in `export_meta`,
so any result database is self-describing — a reader can always tell
exactly what question produced it. Projection below the key level (hash
fields, score ranges) is deliberately **not** in v1: it can be done in SQL
against the materialised result, which is the point of returning SQLite.
Refused shapes are refused loudly (invalid spec → error, no partial
database), never silently narrowed.

### 5.2 Export report

Per parent §B4, the report is part of the artifact: the `export_meta`
rows, per-type exported counts, per-reason skip counts, scan window,
`DBSIZE` delta, and the §4.2 ordering warning. The same content may be
mirrored to a sidecar file for humans, but the database is the canonical
report — it cannot be separated from its data.

### 5.3 Credentials and addressing

The source is addressed by a foreign-store **reference** resolved outside
ID data (per the parent contract). The connection's secret is never
written to the SQLite file, the report, ID data, or logs: `export_meta`
records a non-secret host fingerprint and the Redis `run_id` only. An
export file is therefore safe to keep and hand on as data — a property
implementations must test, not assume.

### 5.4 Failure behaviour

If the source goes away mid-scan, the export **fails**: the partial SQLite
file is marked failed in `export_meta` (`status = 'failed'` with the
error) or removed — an implementation picks one, states it in its release
notes, and never leaves a partial file indistinguishable from a complete
one. Slow sources are bounded by the scan `COUNT` and a configurable
overall deadline, likewise recorded.

## 6. Alternatives rejected

- **DUMP/RESTORE payloads as BLOBs.** Byte-exact and easy, but opaque:
  the result is an archive, not a queryable database — defeats both §B4
  ("query") and §B5 ("joins against ID data").
- **One wide `key/value` table with JSON payloads for aggregates.** Loses
  list order as data, zset score semantics, and per-element SQL
  addressability; every consumer re-implements the type mapping in JSON
  gymnastics.
- **Live pass-through (translate SQL to Redis commands at query time).**
  Contradicts the parent's materialize-first non-goal, hides unbounded
  latency/semantics mismatches behind a familiar interface, and is a
  different (larger) design. Not this record.
- **Type inference for strings** (numeric-looking → numeric columns).
  Lossy by construction (§4.4); rejected even as an opt-in, because an
  opt-in lossy mode in the exporter makes every artifact suspect.
- **RDB parsing as the primary path.** Valid for offline bulk copies;
  rejected as primary (host filesystem access, no selection, snapshot
  semantics tied to RDB policy). May return as a follow-on bulk path
  feeding the same §3 schema.

## 7. Invariants implementation must hold

1. Read-only against Redis, by the §2 command allowlist — no mutation path
   exists, including error paths and retries.
2. Bytes are preserved exactly; text columns are views (non-NULL iff valid
   UTF-8); no transcoding, escaping, or type inference anywhere.
3. Selected = exported + skipped, reconciled per type and reason; a
   non-reconciling export is failed, and a partial artifact is never
   indistinguishable from a complete one.
4. No silent truncation, no silent drops: oversize and unmapped types are
   named in `export_skips` with reasons from the closed vocabulary.
5. Expiry is exported as observed metadata, never enforced, extended, or
   presented as exact.
6. Credentials appear in no artifact: not in the SQLite file, the report,
   ID data, or logs.
7. Export and query results share one versioned schema; a query result is
   self-describing (selection spec in `export_meta`).
8. The mapping is at the logical type level; Redis-internal encoding
   changes do not change output.

## 8. Not decided here

- Implementation timing and crate placement (the issue itself notes
  implementation may be months out; this record only removes the design
  unknowns).
- The general foreign-adapter contract (#5803, open) — §1.2's assumptions
  are re-read against it when it lands.
- Stream and module-type mapping (follow-on design, if wanted; §3.2).
- How ID _imports_ an export file beyond "importable as a snapshot" —
  import semantics belong to the ID data-layer design (#2048 lineage), not
  to this export record.
- Write-back in any form (parent non-goal; would need its own design).

<!-- agent-note:v1 agent=repo-worker-code lane=code-k state=done task=14794 -->
