# Atlas — span-table schema: spans as `(file_version_id, start, end)` with embedded-text snapshots

Design doc — 2026-11-08. Deliverable for #6035 (follow-up 1, Cluster 1),
child of the delta-only-ingestion design parent #6028. Companion
children: #6036 (offset matcher), #6037 (boundary policy v2), #6038
(boundary drift), #6039 (overlap budget), #6040 (CDC refuted).
Sibling mechanism already settled on #6028: the rsync/restic span model
— a file version is an ordered list of span references `(embed_id,
start, end)`; spans are reused only when the embedded text is
byte-identical, and search-time reconstruction unions overlapping spans
so merged lines do not double up.

**Design only; no implementation is authorized by this note.** The
go/no-go is §10.

State verified at `origin/main` `b6bd188624` (2026-11-08), the same main
the queue task 12884 revalidated against:

- `pkgs/atlas-search/crates/atlas-core/src/filters.rs` queries the chunk
  store directly: `turso_vector_scan_stmt` joins
  `vec_snowflake_m_v2_768_f32 v JOIN chunks c ON c.id = v.chunk_id`
  (`filters.rs:304-312`), the FTS branch selects `c.id AS cid FROM chunks
  c JOIN chunks_fts f ON f.rowid = c.id` (`filters.rs:286-289`), and
  `details_sql` reads `c.id, c.text, c.chunk_idx, c.message_id` joined to
  `messages`/`sessions`/`session_labels` (`filters.rs:425-442`).
- `pkgs/atlas-search/crates/atlas-core/src/local.rs:1-10` records the
  identity constraint that shapes this design: vector side stores are
  joined **by content hash, never by `chunk_id`, "the id spaces differ
  between stores"** — chunk ids are per-database integers, not a global
  identity.
- The publish shape is `sessions -> messages -> chunks`, with
  `chunks(id INTEGER PRIMARY KEY, message_id, chunk_idx, text NOT NULL,
  content_hash TEXT UNIQUE NOT NULL, sensitivity)`,
  `vec_snowflake_m_v2_768_f32(chunk_id INTEGER PRIMARY KEY, vec BLOB)`,
  and an FTS5 table rowided by `chunks.id`
  (`pkgs/atlas-search/fixtures/fixture_corpus.sql:24-46`).
- The ingestion brief's numbers (#3125 body, verified 2026-10-31):
  canonical 18,325 sessions / 122,124 messages / **107,639 chunks**;
  published Turso atlas 100,698 chunks; 6,941 local-only chunks.
- No span concept exists on main: `git grep -i 'span-table\|span table'`
  over `doc/` and `pkgs/atlas*` returns nothing; the staging schema
  (`pkgs/atlas-search/schema/atlas_stage_v1.sql`) has `chunks_raw`, not
  spans.

---

## 1. Problem

The offset-embed model needs three things the chunk store cannot express:

1. **A reused embed is not a chunk row.** Today every stored unit
   corresponds to exactly one `chunks.id` per database. A span reused
   under a new file version must not gain a new identity, a new vector,
   or (worse) a new copy of the same bytes; and an id that external
   traces and feedback already reference must keep resolving.
2. **Position is first-class.** `chunk_idx` is an ordinal within one
   message, not a range. The model's unit is `(file_version_id, start,
   end)` — an interval into a specific version of a source object — and
   the join is interval union, not adjacent-row concatenation.
3. **Context is a property of the reference, not the span.** The same
   bytes reused under a new heading must not inherit the old heading
   path. Today context lives on `session_labels` (per session) and on
   the chunk's denormalized `sensitivity`, so there is no place to hang
   a per-reference heading.

## 2. Proposal in one paragraph

Intern the *embedded text* once (`span_text`, content-addressed by
SHA-256 of the exact bytes), attach vectors only to
`(text_hash, embedder_id)` (`span_embeddings`) — so a reused span costs
one manifest row and zero vectors — and record a source object's version
as an ordered manifest of span references (`source_versions` +
`version_spans`) whose overlap, head/tail match provenance and
**per-reference** context (`span_contexts`) are explicit columns. Keep
every legacy `chunks` row reachable through a per-store
`legacy_chunk_map`, and migrate by dual-write → dual-read → cutover in
three phases, never by mutating an applied schema. Identity is
content-addressed; integer ids stay local to a store.

## 3. Where "file version" comes from today

The model generalizes past files: a **source object** is whatever the
ingest path versions — a chat message, a repository file, a database
row. The existing corpus maps cleanly:

| model | today | note |
|---|---|---|
| `source_objects.object_kind='message'` | `messages.id` | locator `session:<session_id>/message:<message_id>` |
| version of that object | the message row as ingested | `version_key` = hash of `messages.text` |
| spans of that version | `chunks` rows of the message, `chunk_idx` ordering | backfilled intervals, §6 |
| vector | `vec_snowflake_m_v2_768_f32.chunk_id` | re-keyed to `span_embeddings` |
| context | `sessions`+`session_labels` denormalized at query time | becomes a `span_contexts` row |

Git files add `source_kind='file'`, locator `repo@commit:path`,
`version_key` = blob sha. That is the shape #6018's repo ingestion needs;
nothing in the tables below is repository-specific.

## 4. Schema

Six tables in the Atlas core tier. DDL is SQLite/libSQL dialect, as the
publish files use today. Applied once per version
(`_schema_version`), never edited in place — the same rule
`atlas_stage_v1.sql` states.

### 4.1 Interning: `span_text`

```sql
CREATE TABLE span_text (
  text_hash    TEXT PRIMARY KEY,        -- sha256 of the exact UTF-8 bytes embedded
  text         TEXT NOT NULL,           -- the embedded-text snapshot, verbatim
  byte_len     INTEGER NOT NULL,
  token_count  INTEGER,                 -- nullable; embedder accounting only
  created_at   TEXT NOT NULL
);
```

The row's `text` is *what was embedded*, not a pointer into a current
file: it is the archaeological record that lets a hit render even when
the source version is cold or retired (#6028 join requirement). One row
per distinct embedded string; the `content_hash UNIQUE` constraint that
today inflates `chunks` into one row per occurrence disappears.

### 4.2 Vectors: `embedders`, `span_embeddings`

```sql
CREATE TABLE embedders (
  embedder_id  TEXT PRIMARY KEY,   -- 'snowflake-m-v2-768-f32'
  model        TEXT NOT NULL,
  dim          INTEGER NOT NULL,
  template     TEXT NOT NULL,      -- query/doc prefix, pooling, normalization
  params_hash  TEXT NOT NULL,      -- hash of every setting that can move a vector
  created_at   TEXT NOT NULL
);

CREATE TABLE span_embeddings (
  text_hash    TEXT NOT NULL REFERENCES span_text(text_hash),
  embedder_id  TEXT NOT NULL REFERENCES embedders(embedder_id),
  vec          BLOB NOT NULL,
  created_at   TEXT NOT NULL,
  PRIMARY KEY (text_hash, embedder_id)
);
```

A vector is a function of the text **and** the embedder configuration,
so the key is the pair. Reuse is therefore sound by construction: two
versions share a vector iff they share both. The boundary-drift question
(#6038: same paragraph, different neighbours) is *out of scope here* and
does not weaken the invariant — reuse is byte-identical-text reuse, never
"similar text" reuse (the ≥0.9 dedup edge is #5733's, applied in the
matcher, not the schema).

### 4.3 Object graph: `source_objects`, `source_versions`

```sql
CREATE TABLE source_objects (
  object_id      INTEGER PRIMARY KEY,
  object_kind    TEXT NOT NULL,    -- 'message' | 'file' | 'row'
  locator        TEXT NOT NULL,    -- 'repo@commit:path', 'session:<id>/message:<id>', 'db:pk'
  first_seen_at  TEXT NOT NULL,
  UNIQUE (object_kind, locator)
);

CREATE TABLE source_versions (
  version_id    INTEGER PRIMARY KEY,
  object_id     INTEGER NOT NULL REFERENCES source_objects(object_id),
  version_key   TEXT NOT NULL,     -- blob sha / git commit+path / strong hash of the row
  content_hash  TEXT NOT NULL,     -- sha256 of the full version bytes
  byte_len      INTEGER NOT NULL,
  observed_at   TEXT NOT NULL,
  sensitivity   TEXT NOT NULL,     -- classification floor for the version
  synthetic     INTEGER NOT NULL DEFAULT 0,  -- 1: text reconstructed from legacy chunks (§6)
  UNIQUE (object_id, version_key)
);
```

`version_id` is a local integer (cheap joins, FTS rowids); the
cross-store identity is `(object_kind, locator, version_key,
content_hash)`, which is what a shard or export carries. This keeps
`local.rs`'s rule intact: no query crosses stores by chunk/version id.

### 4.4 The manifest: `version_spans`

```sql
CREATE TABLE version_spans (
  version_id        INTEGER NOT NULL REFERENCES source_versions(version_id),
  ordinal           INTEGER NOT NULL,   -- ordering for reconstruction, not identity
  text_hash         TEXT NOT NULL REFERENCES span_text(text_hash),
  start_off         INTEGER NOT NULL,   -- UTF-8 byte offset, inclusive
  end_off           INTEGER NOT NULL,   -- exclusive
  head_match_hash   TEXT,               -- prior span whose tail this span's head matched (#6036)
  tail_match_hash   TEXT,
  context_hash      TEXT NOT NULL REFERENCES span_contexts(context_hash),
  PRIMARY KEY (version_id, ordinal)
);
CREATE INDEX version_spans_by_start ON version_spans (version_id, start_off);
CREATE INDEX version_spans_by_text  ON version_spans (text_hash);
```

- **Offsets are UTF-8 bytes**, half-open. Bytes survive every transport
  (SQLite `text`, HTTP, FTS); character offsets are derived at render
  time. This is the interface #6036's matcher emits.
- `head_match_hash` / `tail_match_hash` are the *provenance of the
  acceptance decision*, not the overlap itself: the matcher records which
  prior embed's head/tail run justified accepting this span at this
  offset. #6036 owns the minimum-run and maximum-overlap thresholds;
  the schema only stores their outcome.
- An overlap between two spans of one version is legal and expected
  (Drew's 17–241 case). It is recorded as two rows whose intervals
  intersect; nothing is deduplicated at write time.
- `ordinal` is a stable write order for deterministic diffs. It is not
  identity: re-ingesting the same version must produce the same manifest
  (sort by `start_off, end_off, text_hash`), which is the property test
  in §7.

### 4.5 Context per reference: `span_contexts`

```sql
CREATE TABLE span_contexts (
  context_hash   TEXT PRIMARY KEY,   -- sha256 over the normalized JSON below
  heading_path   TEXT,               -- markdown heading stack / code symbol path
  section        TEXT,
  kind           TEXT,
  topics         TEXT,               -- JSON list text, same shape as session_labels
  projects       TEXT,
  sensitivity    TEXT NOT NULL,
  scheme         TEXT,
  confidence     TEXT
);
```

The row is content-addressed and referenced from the **manifest row**,
so a span reused under a new `"Deprecated:"` heading gets a new
`context_hash` on the new version's row while the old version's row is
untouched. That is the "context versions with the reference" rule made
mechanical. Sensitivity here is the classification assigned *for this
reference*; filters continue to apply before rank
(`filters.rs:364-392`), and #5410's rule that sensitivity semantics are
never data-defined stands.

### 4.6 Provenance: `span_occurrences`

```sql
CREATE TABLE span_occurrences (
  text_hash    TEXT NOT NULL REFERENCES span_text(text_hash),
  object_id    INTEGER NOT NULL REFERENCES source_objects(object_id),
  version_id   INTEGER REFERENCES source_versions(version_id),
  start_off    INTEGER,   -- NULL for a synthetic/unanchored legacy occurrence
  end_off      INTEGER,
  locator      TEXT NOT NULL,   -- CAS path, repo@commit:path, range in an archive
  observed_at  TEXT NOT NULL,
  PRIMARY KEY (text_hash, locator, start_off, end_off)
);
```

This is #3125's `chunk_occurrences` requirement in span terms: every
place a span's bytes were found is recorded rather than silently deduped
— including the occurrences the current dedup drops (6,344 canonical
sessions own zero chunks for exactly this reason).

### 4.7 Per-store index mapping

Each store keeps its own integer handle for the vector table and FTS
rowid, because libSQL vector and FTS5 keys are integers:

```sql
CREATE TABLE span_index (
  span_uid   INTEGER PRIMARY KEY,        -- store-local
  text_hash  TEXT NOT NULL,
  embedder_id TEXT NOT NULL,
  UNIQUE (text_hash, embedder_id)
);
-- vec_snowflake_m_v2_768_f32(span_uid INTEGER PRIMARY KEY, vec BLOB)
-- span_fts(span_uid, text)  -- FTS5, populated from span_text
```

Search joins `version_spans` → `span_index` → vector/FTS **within one
store**; results carry `(locator, version_key, start_off, end_off,
text_hash)` and are de-duplicated by `text_hash` at the cross-store
merge, mirroring today's content-hash merge in `local.rs`.

## 5. Identity rules

1. **Content-addressed text is the only global identity.** `text_hash`
   and `content_hash` are the values that cross stores.
2. **Integer ids never leave their store.** `version_id`, `span_uid`,
   and legacy `chunk_id` are local.
3. **No id reuse, ever.** A retired version keeps its `version_id`; a
   purged span keeps its `span_text` row until every reference is gone
   (refcounted by `version_spans` + `span_occurrences`).
4. **Tenant boundary:** `span_text` for `secret-bearing` / `local-only`
   text is never interned into a published store. The ingest path
   filters before interning, exactly as the publish path filters chunks
   today (6,941 local-only chunks stay local).

## 6. Migration from the chunk store

Three phases; the chunk tables are never dropped in this design.

**Phase A — dual write.** Ingest writes `span_text`/`span_embeddings`/
`source_versions`/`version_spans` and today's `chunks` rows, deriving
spans from the chunker's own intervals so both views agree by
construction. Backfill the existing corpus:

- For every message with chunks: `source_objects` row
  (`object_kind='message'`), one `source_versions` row from
  `messages.text` (`version_key` = sha256 of the text).
- Locate each chunk's text in the message text by exact byte search.
  - unique match → `start_off/end_off` recorded, `synthetic=0`;
  - multiple matches → first match recorded, plus a
    `span_occurrences` row per match (the occurrence table is where
    ambiguity belongs);
  - no exact match (normalized whitespace, re-chunked text) →
    `synthetic=1` version whose text is the concatenation of the
    message's chunk texts, offsets into *that*, and a
    `legacy_chunk_map` row with `anchor_quality='synthetic'`.
- Every legacy chunk gets `legacy_chunk_map(store, chunk_id,
  text_hash, version_id, start_off, end_off)`.

**Phase B — dual read.** Search runs the span path and the chunk path
side by side in shadow; a golden-set parity report (the #6027 set) gates
cutover. Expected parity risks are exactly the synthetic rows: they are
excluded from join reconstruction until reconciled, and counted.

**Phase C — cutover.** Readers move to spans; `chunks`/`chunks_fts`/
vector tables stay read-only for a retention window. External
references (traces, feedback, saved searches) resolve through
`legacy_chunk_map`, which is append-only, so old ids keep meaning what
they meant. `chunk_idx` survives only as a display hint.

Scale check: 107,639 canonical chunks backfill in one pass on a laptop
(~1 GB publish); the operation is a single SQLite transaction per
message, idempotent by `(object_id, version_key)`.

## 7. Invariants (each is a property test, with its oracle)

| # | Invariant | Oracle |
|---|---|---|
| I1 | `span_embeddings` has at most one row per `(text_hash, embedder_id)`, and `span_text.text` hashes to `text_hash` | recompute hash; `UNIQUE` PK |
| I2 | Every non-synthetic version's span intervals cover `[0, byte_len)` with no gap; overlaps are allowed only where `head_match_hash`/`tail_match_hash` is set | interval arithmetic vs `source_versions.byte_len` |
| I3 | Reconstruction (`sort by start_off`, union overlaps by byte-equality trimming) reproduces the version bytes exactly once — the non-doubling guarantee | sha256 of reconstruction == `source_versions.content_hash` |
| I4 | Re-ingesting an unchanged version emits an identical manifest (same ordinals, offsets, hashes) and creates **zero** new `span_text`/`span_embeddings` rows | manifest diff; row counts before/after |
| I5 | Editing one region adds spans only for that region plus acceptance metadata; untouched spans keep their rows and vectors across versions | manifest diff between version N and N+1 |
| I6 | Reusing a span under a different heading changes only the new version's `context_hash`; the prior version's row is unchanged | row snapshot compare |
| I7 | Every legacy `chunk_id` resolves through `legacy_chunk_map` to a `text_hash` whose text equals the old chunk text | join + string compare |
| I8 | No `secret-bearing`/`local-only` `text_hash` appears in a published store | label join at export time |

I3 is the one that catches the merge bug Drew described: three
overlapping spans covering one paragraph must reconstruct it once, not
three times. I4/I5 are the delta-only guarantee stated at the storage
layer.

## 8. Worked example — the 17–241 case end to end

Version 1 (`object_kind='file'`, locator `repo@abc:doc/notes.md`), text
`T1` of 300 bytes, three spans:

| ordinal | text_hash | start | end | context |
|---|---|---|---|---|
| 0 | `h_A` | 0 | 80 | `ctx_1` (heading `# Backup`) |
| 1 | `h_B` | 80 | 200 | `ctx_1` |
| 2 | `h_C` | 200 | 300 | `ctx_1` |

Version 2 after an edit: bytes 0–16 match `h_A`'s head; bytes 17–241 are
new; bytes 242–300 match `h_C`'s tail. Matcher (#6036) emits:

| ordinal | text_hash | start | end | head_match_hash | tail_match_hash | context |
|---|---|---|---|---|---|---|
| 0 | `h_A` | 0 | 17 | — | — | `ctx_2` |
| 1 | `h_N` (new embed of bytes 17–241) | 17 | 241 | `h_A` | `h_C` | `ctx_2` |
| 2 | `h_C` | 242 | 300 | `h_A`\* | — | `ctx_2` |

\* the row's `head_match_hash` names the prior embed whose tail run the
span's head matched; the precise run lengths are #6036's acceptance
data, stored in the occurrence/provenance rows, not in the interval.

Result: one new `span_text` row (`h_N`), one new `span_embeddings` row,
three new manifest rows, zero re-embeds of the 0–16 and 242–300 regions,
and a `span_occurrences` row recording that `h_A` and `h_C` were found
again. Reconstruction of version 2 unions `[0,17)`, `[17,241)`,
`[242,300)` → 300 bytes, exact. If the edit had left a 20-byte overlap
between `h_A`'s tail and `h_N`'s head, both rows would still exist and
the union would trim the duplicate — the non-doubling join.

## 9. Boundaries with the sibling designs

- **#6036 (matcher):** owns run thresholds and how `head_match_hash`/
  `tail_match_hash` are chosen. This schema fixes the offset unit (UTF-8
  bytes, half-open) and stores the outcome.
- **#6037 (boundary policy v2):** owns where new spans *start*; the
  schema accepts any interval.
- **#6038 (drift):** measures whether a reused vector's rank moves with
  neighbours. If drift turns out unacceptable, the remedy is a *policy*
  change (re-embed within a window), not a schema change: the
  `span_embeddings` key already permits several embedder/template
  variants for the same text.
- **#6039 (overlap budget):** supplies the numbers for the size model;
  overlap is stored, so any budget can be evaluated against the same
  data.
- **#6040 (CDC):** refuted as the engine; nothing here depends on it.

Deferred with defaults: (a) fidelity of synthetic legacy spans — default
is excluded from joins and reported; (b) vector-table naming per
embedder — default `vec_<embedder_id>_<dim>_<dtype>`; (c) GC policy for
unreferenced `span_text` — default a 30-day grace then refcount check;
(d) whether `span_contexts` should carry a `heading_path` array type or
JSON text — default JSON text, matching `session_labels.topics`.

## 10. Go/no-go

**GO** on the six-table schema and the migration phases as written. The
parts that are *not* decided here and must land before implementation
starts: #6036's thresholds (they do not change the tables), #6039's
overlap numbers (they change defaults, not keys), and a resolved answer
on synthetic-span reconciliation for the 107,639-chunk backfill (the
only migration step that can silently lose text).

**No-go** alternative considered and rejected: extending `chunks` with
`version_id/start/end` columns. It keeps one row per occurrence, so a
reused span still duplicates its bytes and vector, `content_hash UNIQUE`
still cannot express "same text, two versions", and context still has
nowhere to live but the chunk. The normalized tables above are the
minimum that expresses the model.
