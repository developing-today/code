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

Intern the *span text* once (`span_text`, content-addressed by SHA-256
of the exact span bytes), intern each *embedded window* once
(`span_windows`, keyed by SHA-256 of the exact span-plus-neighbours
window bytes), and attach vectors only to `(window_hash, embedder_id)`
(`span_embeddings`). A repeated span reuses its text snapshot; it reuses
its vector only when the full embedded window is byte-identical. Record a
source object's version
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

Ten core tables in the Atlas tier: `span_text`, `span_windows`,
`span_window_parts`, `embedders`, `span_embeddings`, `source_objects`, `source_versions`,
`version_spans`, `span_contexts`, and `span_occurrences`. The per-store
`span_index` and `legacy_chunk_map` mappings are additional. DDL is
SQLite/libSQL dialect, as the
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
  window_hash  TEXT NOT NULL REFERENCES span_windows(window_hash),
  embedder_id  TEXT NOT NULL REFERENCES embedders(embedder_id),
  vec          BLOB NOT NULL,
  created_at   TEXT NOT NULL,
  PRIMARY KEY (window_hash, embedder_id)
);
```

A vector is a function of the **window text that was actually embedded**
and of the embedder configuration, so the key is that pair. The window is
the embedded unit the matcher hands the embedder — the span plus its
recorded neighbours (#6038 §"Reuse-safety rule", landed on `main` as
`pkgs/atlas-search/eval/boundary-drift-6038.md`, PR #7661): two versions
share a vector iff they share the exact window bytes *and* the embedder.

Keying by `text_hash` alone was the earlier draft of this document and is
**withdrawn**. The measurement behind it: holding the target paragraph
byte-identical and changing only a neighbour flips top-10 membership in
10.3–17.8% of query/target pairs, and even a one-sentence boundary shift
flips 5.8–6.9%. Span identity is therefore *not* a sufficient reuse key —
window identity is. Reuse remains byte-identical reuse, never "similar
text" reuse (the ≥0.9 dedup edge is #5733's, applied in the matcher, not
the schema); what changes is that "identical" is judged over the window.

Cost of the stricter key: a span whose neighbours change gets a new
window row and a new vector even though its own bytes are unchanged. That
is the measured price of correctness, and it is bounded — the window is
derived from spans already stored, so a window row costs one hash plus
its vector, never a second copy of the text.

### 4.2a Window interning: `span_windows`

```sql
CREATE TABLE span_windows (
  window_hash   TEXT PRIMARY KEY,   -- sha256 of the exact bytes handed to the embedder
  window_text   TEXT NOT NULL,      -- the embedded bytes, verbatim (archaeological record, as span_text)
  byte_len      INTEGER NOT NULL,
  created_at    TEXT NOT NULL
);

CREATE TABLE span_window_parts (
  window_hash    TEXT NOT NULL REFERENCES span_windows(window_hash),
  text_hash      TEXT NOT NULL REFERENCES span_text(text_hash),  -- the target span of this window
  prev_text_hash TEXT REFERENCES span_text(text_hash),           -- recorded left neighbour, NULL at version start
  next_text_hash TEXT REFERENCES span_text(text_hash),           -- recorded right neighbour, NULL at version end
  PRIMARY KEY (window_hash, text_hash, prev_text_hash, next_text_hash)
);
CREATE INDEX span_window_parts_by_text ON span_window_parts (text_hash);
```

`window_text` is stored for the same reason `span_text.text` is: a hit
must render its evidence even when the neighbours' spans have since been
GC'd or the source version retired. Window-byte identity and
target-span identity are separate keys, deliberately: two different
target spans can produce identical window bytes (in a two-span version
A followed by B, the window of A and the window of B can both be the
bytes A+B), so a single `span_windows` row keyed by `window_hash` is
shared by both targets, and `span_window_parts` records which target
and which neighbour decomposition each use of that window had. The
vector is a function of the window bytes alone, so sharing the
`span_embeddings` row across those targets is correct; what must never
be shared is the claim about which span the window was centred on.
The `prev_/next_text_hash` columns make the re-embed trigger
computable without re-deriving the manifest — a neighbour added,
removed, replaced or boundary-moved produces a different `window_hash`
and therefore a different `span_embeddings` row.

`version_spans` therefore also carries `window_hash` (the window each
reference was embedded under), so a version's manifest is enough to
reproduce its embeddings without recomputing boundaries.

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
  window_hash       TEXT NOT NULL REFERENCES span_windows(window_hash),  -- the window this reference was embedded under (§4.2a)
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
  window_hash TEXT NOT NULL,
  embedder_id TEXT NOT NULL,
  UNIQUE (window_hash, embedder_id)
);
-- vec_snowflake_m_v2_768_f32(span_uid INTEGER PRIMARY KEY, vec BLOB)
-- span_fts(span_uid, text)  -- FTS5, populated from span_text
```

Search joins `version_spans` → `span_index` → vector/FTS **within one
store**; results carry `(locator, version_key, start_off, end_off,
text_hash, window_hash)` and are de-duplicated by `text_hash` at the
cross-store merge, mirroring today's content-hash merge in `local.rs`
(the merge dedups *text* for rendering; the vector hit was scored under
its own window, which is what the ranking means).

## 5. Identity rules

1. **Content-addressed bytes are global identities.** `text_hash`,
   `window_hash`, and `content_hash` cross stores; integer ids do not.
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

**Phase A — dual write.** Ingest writes `span_text`/`span_windows`/
`span_embeddings`/`source_versions`/`version_spans` and today's `chunks` rows, deriving
spans from the chunker's own intervals so both views agree by
construction. Backfill the existing corpus:

- For every message with chunks: `source_objects` row
  (`object_kind='message'`), one `source_versions` row from
  `messages.text` (`version_key` = sha256 of the text).
- Locate each chunk's text in the message text by exact byte search.
  - unique match → candidate `start_off/end_off` recorded (see the
    coverage gate below before `synthetic` is decided);
  - multiple matches → first match recorded, plus a
    `span_occurrences` row per match (the occurrence table is where
    ambiguity belongs);
  - no exact match (normalized whitespace, re-chunked text) →
    `synthetic=1` version whose text is the concatenation of the
    message's chunk texts, offsets into *that*, and a
    `legacy_chunk_map` row with `anchor_quality='synthetic'`.
- Coverage gate, applied per message before any version is marked
  `synthetic=0`: the union of the matched chunk intervals must cover
  `[0, byte_len)` with no gap, and the I3 reconstruction hash must
  equal the version `content_hash`. Unique matches alone do not prove
  coverage — legacy chunkers omit separators and trailing text (the
  `mA2` chunks in `pkgs/atlas-search/fixtures/fixture_corpus.sql` both
  match uniquely while their union omits `; ` and ` follow`). A version
  that fails the gate is marked `synthetic=1`, its uncovered ranges are
  filled with spans interned from `messages.text` itself, and it is
  counted for reconciliation; the shadow-read parity plan in Phase B
  therefore treats synthetic rows as the only expected parity risk
  because every non-synthetic row has passed this gate.
- Messages whose text was deduplicated into another message's chunks
  own no chunks, and the loop above never reaches them — yet §4.6
  counts 6,344 such zero-chunk sessions whose occurrences this design
  promises to recover. Enumerate retained messages with no chunks
  explicitly: create their `source_objects`/`source_versions` rows from
  `messages.text`, derive their manifest by interning the message text
  (the `span_text` rows already exist, created by the owning message),
  and record a `span_occurrences` row per derived span. Without this
  step their provenance is absent after cutover.
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
| I1 | `span_embeddings` has at most one row per `(window_hash, embedder_id)`; `span_text.text` hashes to `text_hash`; `span_windows.window_text` hashes to `window_hash` | recompute hashes; `UNIQUE` PK |
| I1a | A vector is never reused across a window change: two references share a `span_embeddings` row only if their `window_hash` is equal, and any neighbour add/remove/replace or boundary move yields a different `window_hash` | construct the perturbed window (§#6038 cases) and assert the hash differs and a new row is required |
| I1b | `version_spans` `(window_hash, text_hash)` resolves to a `span_window_parts` row with that pair, whose `window_hash` row's `window_text` hashes to `window_hash` | join + equality check |
| I2 | Every non-synthetic version's span intervals cover `[0, byte_len)` with no gap; overlaps are allowed only where `head_match_hash`/`tail_match_hash` is set | interval arithmetic vs `source_versions.byte_len` |
| I3 | Reconstruction (`sort by start_off`, union overlaps by byte-equality trimming) reproduces the version bytes exactly once — the non-doubling guarantee | sha256 of reconstruction == `source_versions.content_hash` |
| I4 | Re-ingesting an unchanged version emits an identical manifest (same ordinals, offsets, hashes, window hashes) and creates **zero** new `span_text`/`span_windows`/`span_embeddings` rows | manifest diff; row counts before/after |
| I5 | Editing one region adds spans only for that region plus acceptance metadata; untouched spans keep their text rows, and keep their vectors **only where their window is unchanged** (a neighbour edit re-embeds them, per #6038) | manifest diff between version N and N+1, split by window-hash equality |
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

Version 2 after an edit (half-open intervals): bytes `[0,17)` match
`h_A`'s head; bytes `[17,242)` are new; bytes `[242,300)` match `h_C`'s
tail. The matched fragments are **not** references to `h_A`/`h_C`: a
reference must cover exactly the bytes its `text_hash` identifies (§4.1,
I2), and `h_A` identifies 80 bytes, not 17. The matcher therefore
interns the fragments as new `span_text` rows — `h_A17` for the 17-byte
head fragment, `h_C58` for the 58-byte tail fragment — and records the
match against `h_A`/`h_C` as provenance (`head_match_hash`/
`tail_match_hash` on the new span's row, plus `span_occurrences` rows).
Matcher (#6036) emits:

| ordinal | text_hash | start | end | head_match_hash | tail_match_hash | context |
|---|---|---|---|---|---|---|
| 0 | `h_A17` (new intern of bytes [0,17)) | 0 | 17 | — | `h_A` | `ctx_2` |
| 1 | `h_N` (new span, bytes [17,242)) | 17 | 242 | `h_A` | `h_C` | `ctx_2` |
| 2 | `h_C58` (new intern of bytes [242,300)) | 242 | 300 | `h_C` | — | `ctx_2` |

Result: three new `span_text` rows (`h_A17`, `h_N`, `h_C58`) and
**three** new `span_embeddings` rows. Every window changed: the head
fragment's right neighbour is now `h_N` instead of `h_B`; `h_N` is new;
and the tail fragment's *left* neighbour changed from `h_B` to `h_N`
(`h_C` was last in version 1 and the fragment is last in version 2 —
its right side is end-of-version in both). Under the #6038 contract
(I1a) a neighbour change on either side forces a re-embed, so this
edit reuses text interning savings only where a span's *whole window*
survives byte-identical — none does here, and the example does not
pretend otherwise; I5's delta guarantee is stated per window, not per
span. Three new manifest rows, and `span_occurrences` rows recording
that fragments of `h_A` and `h_C` were found again. Reconstruction of
version 2 unions `[0,17)`, `[17,242)`, `[242,300)` → 300 bytes, exact.
If the edit had left a 20-byte overlap between the head fragment's tail
and `h_N`'s head, both rows would still exist and the union would trim
the duplicate — the non-doubling join.

## 9. Boundaries with the sibling designs

- **#6036 (matcher):** owns run thresholds and how `head_match_hash`/
  `tail_match_hash` are chosen. This schema fixes the offset unit (UTF-8
  bytes, half-open) and stores the outcome.
- **#6037 (boundary policy v2):** owns where new spans *start*; the
  schema accepts any interval.
- **#6038 (drift):** **landed** on `main` — `pkgs/atlas-search/eval/boundary-drift-6038.md`
  (PR #7661, merge `ec6938ae4c8e`), measured on 50 real-corpus targets:
  a one-sentence boundary shift flips top-10 membership 5.8–6.9% of
  query/target pairs, one replaced neighbour 10.3–17.8%. Its
  reuse-safety rule — *reuse only byte-identical windows; re-embed on
  any context change; key stored vectors by the hash of the exact window
  text* — is **adopted here as the schema contract**, not deferred to
  policy: §4.2/§4.2a key `span_embeddings` by `window_hash`, and I1a
  makes "a neighbour change forces a re-embed" a schema-level property
  test. This supersedes this document's first draft, which keyed vectors
  by `text_hash` and called drift out of scope. The measurement is
  scoped to its pool (50 targets, 20 queries) — it is the basis for a
  conservative key, and a wider measurement would only strengthen it.
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

**GO** on the ten core tables and the
migration phases as written. The parts that are *not* decided here and
must land before implementation starts: #6036's thresholds (they do not
change the tables), #6039's overlap numbers (they change defaults, not
keys), and a resolved answer on synthetic-span reconciliation for the
107,639-chunk backfill (the only migration step that can silently lose
text). #6038 is settled and folded in (§9); #5733's ≥0.9 dedup edge
stays in the matcher and cannot loosen the `window_hash` key.

**No-go** alternative considered and rejected: extending `chunks` with
`version_id/start/end` columns. It keeps one row per occurrence, so a
reused span still duplicates its bytes and vector, `content_hash UNIQUE`
still cannot express "same text, two versions", and context still has
nowhere to live but the chunk. The normalized tables above are the
minimum that expresses the model.
