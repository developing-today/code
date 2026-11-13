# Design: span-table schema for offset embeds (atlas)

- **Issue:** #6035 (parent #6028, delta-only ingestion + offset embeds)
- **Siblings:** #6036 (offset matching — this design settles the question
  its §4 leaves open, §3 below); #6026 (content versioning and
  retention — file/version identity is taken from that design, not
  redefined here); #5733 (Jaccard third signal — dedup coordination,
  §7); #3125 (keep-all rule and occurrence semantics).
- **Status:** design record. No implementation in this document's PR.
- **State verified at:** origin/main ce913073f7 (2026-11-13). The
  publish-shaped store is still chunk-centric:
  `pkgs/atlas-search/fixtures/fixture_corpus.sql` defines
  `chunks (id, message_id, chunk_idx, text, content_hash UNIQUE,
  sensitivity)`, `vec_snowflake_m_v2_768_f32 (chunk_id, vec)`, and a
  `chunks_fts` FTS5 table; atlas-core resolves retrieval through
  chunks. No span table or span schema exists on main.

## 1. Problem

In the offset-embed model (#6028) a file version is an ordered list of
span references `(embed_id, start, end)`: most of a new version is
byte-identical to spans already embedded, and only genuinely new
ranges are embedded and stored. That model needs a schema in which

- a span's stored text *is* the text that was embedded (the snapshot),
  because the same paragraph embedded with different neighbours is a
  different vector — embeddings are not hashes;
- spans are reused **only** when byte-identical;
- context (heading path, section) versions with the *reference*, not
  the span, so a reused span under a new heading carries no stale
  context;
- a version can be reconstructed from its span list exactly once (the
  no-double-up join of #6028), including when the underlying version
  is cold or retired.

## 2. Identity and units (settled)

- **File / version identity** is #6026's: a logical file has a stable
  `file_id`; a version is `(file_id, version_seq)`, immutable once
  written. This design anchors to the version's surrogate key
  `file_version_id`.
- **Offsets are bytes** into the version's UTF-8 content, half-open
  `[start, end)` — the unit the #6036 matcher works in (Gear rolling
  hash over bytes, byte-verified copy tokens). Anchors never drift:
  versions are immutable, so an anchor cannot be invalidated by a
  later edit; a later edit is a *new version* with its own manifest
  (§5, I8).

## 3. The question #6036 left open — settled: split-on-reference

#6036 §4 asks whether a copy token covering part of an old span
resolves to a sub-span row (split-on-reference) or rounds outward to
whole old spans. **This design adopts split-on-reference**, #6036's
recommendation: the restricted range becomes a new span whose
snapshot is the restricted text, re-embedded, with provenance
pointing at the parent span and range. Rounding outward would force
re-embedding a whole 1–2 KB span for every mid-span edit and would
quietly re-introduce chunk-scale waste into a delta-only pipeline.
Whole-span copies — the common case for unedited regions — still
resolve to the existing span rows unchanged.

## 4. Tables

DDL is illustrative SQLite (the publish store's dialect today);
column types follow the existing schema's conventions.

```sql
-- One row per unique embedded text. Content-addressed, immutable.
CREATE TABLE spans (
  span_id       INTEGER PRIMARY KEY,
  content_hash  TEXT NOT NULL UNIQUE,   -- BLAKE3 of text (see I1)
  text          TEXT NOT NULL,          -- the embedded-text snapshot
  byte_len      INTEGER NOT NULL,       -- length of text in bytes
  sensitivity   TEXT,                   -- content-derived, see I7
  origin_kind   TEXT NOT NULL,          -- 'fresh' | 'split' | 'backfill'
  parent_span_id INTEGER REFERENCES spans(span_id),  -- split provenance
  parent_start  INTEGER,                -- range within parent snapshot
  parent_end    INTEGER,
  created_at    TEXT NOT NULL
);

-- One vector per span per embedding model. A model change adds rows;
-- it never mutates or re-keys existing ones.
CREATE TABLE span_vectors (
  span_id   INTEGER NOT NULL REFERENCES spans(span_id),
  model_id  TEXT NOT NULL,              -- e.g. 'snowflake-m-v2-768-f32'
  vec       BLOB NOT NULL,
  PRIMARY KEY (span_id, model_id)
);

-- Full-text index over snapshots, one row per span (replaces
-- chunks_fts, which indexes every chunk occurrence).
CREATE VIRTUAL TABLE spans_fts USING fts5(text);  -- rowid = span_id

-- The manifest: a version as an ordered list of span references.
-- Context lives HERE, per reference (I3).
CREATE TABLE file_version_spans (
  file_version_id   INTEGER NOT NULL,   -- #6026 version surrogate key
  ordinal           INTEGER NOT NULL,   -- 0-based position in the version
  span_id           INTEGER NOT NULL REFERENCES spans(span_id),
  start_byte        INTEGER NOT NULL,   -- into the version, half-open
  end_byte          INTEGER NOT NULL,
  context_heading_path TEXT,            -- e.g. 'Guide > Install > Linux'
  context_section      TEXT,
  PRIMARY KEY (file_version_id, ordinal),
  UNIQUE (file_version_id, start_byte)
);

-- Permanent resolution for ids the old store issued (§6).
CREATE TABLE chunk_span_map (
  chunk_id  INTEGER PRIMARY KEY,        -- chunks.id as issued today
  span_id   INTEGER NOT NULL REFERENCES spans(span_id)
);
```

Notes:

- **No separate occurrences table.** A manifest row *is* an
  occurrence of its span (#3125 semantics): "where does this text
  appear" is `file_version_spans WHERE span_id = ?`, and an embed's
  matched head/tail in #6028's example are manifest rows of the
  earlier version resolving to the same span. Split provenance is the
  one occurrence-like fact that belongs on the span itself
  (`parent_span_id` + range), because it describes how the span's
  text was derived, not where it is used.
- **The span is the unit of embedding, indexing, and dedup.** Vectors
  and FTS rows exist once per unique text, however many versions
  reference it. Retrieval resolves span → manifest rows → versions,
  which is the direction the sensitivity filter and the join need.
- `spans.sensitivity` is classified once per unique text.
  Sensitivity of a *source* lives on the version (#6026); the
  effective sensitivity used by filters is defined in I7.

## 5. Invariants (testable)

- **I1 — Byte-identical reuse only.** A span is reused for a range
  iff the range's bytes equal the span's snapshot bytes.
  `content_hash` (BLAKE3) is the lookup key and audit record; at
  ingest the candidate bytes are compared before a reference is
  written (the #6036 rule: the hash is a filter, never the proof).
- **I2 — Snapshot immutability.** `spans` rows are never updated.
  Changed text is a new span. (Model changes are new `span_vectors`
  rows, not span changes.)
- **I3 — Context versions per reference.** Heading path / section
  exist only on `file_version_spans`. A span reused under a
  "Deprecated:" heading shows the new context; nothing about the
  span row changes.
- **I4 — Exact reconstruction.** For every version: manifest rows
  ordered by `ordinal` are contiguous and non-overlapping,
  `start_byte` of the first row is 0, each row's
  `end_byte - start_byte = spans.byte_len` of its span, and the last
  row's `end_byte` equals the version's byte length. Concatenating
  the referenced snapshots in ordinal order yields the version's
  bytes exactly. This is a validation query, run at ingest and in
  the migration gate (§6).
- **I5 — No double-up.** Search-time paragraph assembly reads
  version ranges through the manifest (I4), never by concatenating
  hit texts; overlapping hits on sibling references resolve to the
  same version range once. I5 is a corollary of I4 plus this rule,
  and is the #6028 join invariant stated against the schema.
- **I6 — Vector validity is snapshot-scoped.** A vector is valid
  only for the exact snapshot text and `model_id` it was computed
  from. A split sub-span never inherits its parent's vector (§3);
  it is embedded as its own span.
- **I7 — Sensitivity never lowers on reuse.** Filters evaluate
  `max(spans.sensitivity, file_version.sensitivity)` per manifest
  row (the publish schema's ordering). A span classified from
  secret-bearing content keeps that classification wherever it is
  reused; reuse into a lower-sensitivity version does not dilute
  it, and a low-sensitivity span referenced by a high-sensitivity
  version is filtered at the version's level. This preserves the
  fixture corpus's stated behaviour (secret-bearing content absent
  at the default ceiling, not down-ranked).
- **I8 — Anchors do not detach; versions retire whole.** Because a
  version is immutable, an anchor `(file_version_id, start, end)`
  is valid for exactly as long as its manifest row exists. When a
  version rolls to cold storage or is retired under #6026's policy,
  its manifest and the referenced snapshots remain resolvable —
  the snapshot *is* the served text, so a query trace (#6027)
  resolves without re-reading the source version. Span rows are
  garbage-collected only when no manifest row and no
  `chunk_span_map` row references them and retention policy permits;
  this design expects that to be rare under keep-all (#3125).

## 6. Migration from the chunk store

Corpus at design time: ~107,639 chunks (2026-11-08 count, #6035).
The chunk store's unit is the *message* (session corpus), the span
store's unit is the *version*; the migration bridges that honestly
instead of pretending chunks were file spans.

- **Phase 0 — schema alongside.** The §4 tables land in the publish
  schema next to `chunks`; nothing reads them yet. New ingests are
  unchanged.
- **Phase 1 — backfill spans from chunks.** For each chunk: the
  span candidate is its `text`. Chunks already dedupe by
  `content_hash UNIQUE`, but that hash's algorithm is the chunker's,
  not this design's: backfill recomputes BLAKE3 over `text`, byte-
  compares on hash collision, and writes one span per unique text
  (`origin_kind='backfill'`) plus its vector row *by copying the
  chunk's existing vector* — the vector was computed from exactly
  this text, so I6 holds without re-embedding. Every chunk gets a
  `chunk_span_map` row.
- **Phase 2 — backfill manifests where provable.** A message is a
  single-version object in this migration: mint `file_id` per
  message, `version_seq = 1`. Its manifest is its chunks in
  `chunk_idx` order, with byte offsets assigned cumulatively — and
  the manifest is written **only if I4 verifies** (concatenated
  chunk texts equal the message text byte-for-byte). Overlapping or
  trimmed chunking fails verification for that message; it is
  counted and reported, not forced. Unverified messages keep
  chunk-path reads until they are re-ingested under the span model.
- **Phase 3 — dual-read.** Retrieval resolves through spans for
  versions with verified manifests, through chunks otherwise;
  atlas-core's filters (including the sensitivity ceiling) must
  produce identical results on both paths — the fixture corpus is
  the parity test, extended with a span-path twin. New ingests
  write the span model only (files natively; messages as
  single-version objects with verified manifests by construction).
- **Phase 4 — cutover.** When the Phase 2 report shows 100% of
  *active* messages verified (or explicitly excepted and
  re-ingested), chunk writes stop and `chunks` /
  `vec_snowflake_m_v2_768_f32` / `chunks_fts` become read-only
  legacy. They are dropped only in a later major schema change,
  with `chunk_span_map` archived first.
- **Chunk IDs already referenced by traces and feedback** resolve
  permanently through `chunk_span_map`: chunk → span → every
  manifest row (its occurrences). A chunk id is never reissued
  with a different meaning; a trace recorded against chunk *N*
  still names the same text after cutover, and the span path can
  additionally name every version that text occurs in — strictly
  more resolution than the chunk store offers today.
- **Rollback.** Until Phase 4, the chunk store is untouched and
  authoritative for unverified content; abandoning the migration
  is dropping the §4 tables. After Phase 4, rollback is a
  re-export from spans (I4 makes versions fully reconstructible).

## 7. Coordination with #5733 (Jaccard) and the #6028 thresholds

- **Shingle sets live on the span.** #5733 gives each chunk a
  stored token-shingle set for rescore and ingest dedup. In the
  span model the shingle set is, like the vector, a property of
  the snapshot text: computed once per span (an implementation
  follow-on adds its storage — a `span_shingles` table keyed by
  `(span_id, shingle_hash)` or an equivalent blob; this design
  fixes only that it is span-scoped, not reference-scoped).
- **The ≥0.9 dedup edge does not authorize reuse.** #6028 asks for
  the maximum overlap before an embed "is just a duplicate",
  interacting with #5733's ≥0.9 Jaccard edge. Settled here: the
  0.9 edge is a *retrieval and reporting* classification — a new
  range whose shingle Jaccard against an existing span is ≥0.9
  is recorded as a near-duplicate of that span (for rescore dedup
  and occurrence reporting) and is still stored and embedded as
  its own span if its bytes differ. Reuse remains exactly I1
  (Jaccard 1.0 on shingles is not byte identity; byte identity is
  proven by comparison, not estimated by shingles). Anything
  weaker would let a one-word edit silently serve the pre-edit
  vector — the failure the snapshot rule exists to prevent.
- **Minimum anchor run** is the matcher's parameter (#6036:
  `MIN_MATCH` 128 bytes, boundary rule at any length); the schema
  imposes no minimum span size — a boundary-rule span may be a
  few bytes, and its manifest row costs the same as any other.

## 8. Worked example (Drew's 17–241 case, #6028)

Old version `V1` of a file, 1,000 bytes, has spans `S1 [0, 400)`,
`S2 [400, 1000)` in its manifest. An edit produces `V2`: bytes
0–16 match `S1`, bytes 17–241 are new, bytes 242–999 match `S1`'s
remainder and `S2` (offsets shifted by the edit's net length).

1. The matcher (#6036) emits: copy `[0, 17)` from `S1`, literal
   `[17, 242)`, copy of `S1 [17+Δ, 400)` and all of `S2` at their
   new offsets.
2. Resolution (§3): the two `S1` copy ranges are *parts* of `S1`,
   so each becomes a split span — `S3` = `S1[0, 17)` and `S4` =
   `S1`'s matched remainder — each re-embedded with
   `parent_span_id = S1`. The literal becomes fresh span `S5`.
   `S2` is copied whole: referenced unchanged.
3. `V2`'s manifest: `(S3, 0, 17)`, `(S5, 17, 242)`,
   `(S4, 242, 242 + len(S4))`, `(S2, …, 1000 + Δ)` — contiguous,
   covering `[0, |V2|)` (I4). Only `S3`, `S4`, `S5` were embedded;
   `S2`'s vector and snapshot were reused untouched, and `V1`'s
   manifest is unchanged, so traces against `V1` still resolve.

## 9. Follow-on implementation split (proposed)

1. Schema + backfill tooling (Phases 0–2) with the I4 verification
   report over the live corpus.
2. Ingest writes spans: offset matcher integration writing
   manifests for files; single-version manifests for messages.
3. Dual-read retrieval + fixture parity (Phase 3), then cutover
   (Phase 4) as its own decision with the Phase 2 report attached.
4. `span_shingles` storage for #5733's rescore, on this schema.

Nothing in this design authorizes implementation; each follow-on
is its own packet under #6028.
