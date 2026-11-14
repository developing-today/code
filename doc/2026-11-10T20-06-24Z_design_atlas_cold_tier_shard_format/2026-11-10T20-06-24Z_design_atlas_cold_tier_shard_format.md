# Atlas — cold-tier shard format and restore path

Design doc — 2026-11-10. Deliverable for #6042 (parent: content
versioning + retention, #6026; program: Atlas ingestion, #6018).
Companions: the content versioning and retention design
(`doc/2026-11-08T10-26-00Z_design_atlas_content_versioning_retention/`),
the PITR-vs-retention decision
(`doc/2026-11-05T15-50-41Z_decision_atlas_pitr_vs_retention/`), and the
storage growth model
(`doc/2026-11-08T11-18-02Z_design_atlas_storage_growth_model/`).
Design only; no implementation is authorized by this note.
Implementation is split into follow-ons in §10.

State verified at `origin/main` `19920eaaa0` (2026-11-10): the #6026
design (§6) fixes the cold-tier contract — rustfs shards, resolvability
preserved, self-describing format — and explicitly leaves the shard
layout, the restore protocol, and the cold thresholds to this doc.
The inputs below are settled and are not re-litigated here: the
version object model of #6026 §2, the retention rule of #6026 §4.1,
and the layer boundary of the #6043 decision record.

---

## 1. Requirement in one paragraph

When a version rolls out of the hot tier, it must land in cold storage
as a **self-describing shard**: a single rustfs object that a future
reader — years later, with no access to the hot database that wrote
it — can parse, verify, and re-import into a hot shard with
provenance intact. The shard carries its own manifest (what versions
it holds, where every span and vector lives, what retention state
they were in), its payload in an open, documented byte layout with no
libSQL-only assumptions, and the checksums that make a
hot → cold → hot round trip provably byte-identical at the span
level. Cold shards are storage, not a search tier: they are not
searched by default, and they are explicitly addressable when a
query pins or reaches a version they hold (#6026 §5.3, §6).

## 2. Container layout

A cold shard is **one immutable object** in rustfs (S3-compatible
object storage; the 2026-11-04 storage ruling). Immutability is a
format rule, not a storage setting: a shard is written once, never
modified in place, and replaced wholesale when §8 compaction produces
a successor. All fixed-width integers are little-endian.

| region         | contents                                                                                                      |
| -------------- | ------------------------------------------------------------------------------------------------------------- |
| header         | magic `ATLASCLD` (8 bytes), `format_version` u16 (= 1), header length u32                                     |
| manifest       | one UTF-8 JSON document (§3), length from the header                                                          |
| span section   | concatenated span (chunk) bytes, one entry per span-table row                                                 |
| vector section | concatenated embedding vectors, one entry per vector-table row                                                |
| footer         | section offsets/lengths, SHA-256 of each section, SHA-256 of the manifest, trailer magic `ATLASEND` (8 bytes) |

Decisions, with reasons:

- **Manifest as JSON text, not a binary schema.** The manifest is
  small next to the payload and is the part a future reader must be
  able to inspect with generic tools when every Atlas binary is
  gone. JSON costs nothing at this size and is the self-describing
  property the issue demands. The payload layout below is binary
  because spans and vectors are bulk data; it is fully specified by
  the manifest's tables, so no reader needs a code-generated schema
  for it either.
- **Checksums are SHA-256** (footer, per section and for the
  manifest) because SHA-256 tooling is universal and independent of
  Atlas's own content-hash choice: version content hashes are
  carried verbatim from hot metadata (§3), never recomputed into a
  different algorithm at the tier boundary.
- **Span section may be zstd-compressed as a whole** (flagged per
  shard by the manifest's `span_codec`: `none` or `zstd`). Span
  offsets in the span table always name positions in the
  _decompressed_ section, so the table is codec-independent.
- **Vectors are stored uncompressed** as little-endian IEEE 754
  f32, `dimensions` consecutive values per vector, in vector-table
  order: directly memory-mappable on restore, no decode step, and
  bit-identical by construction for the §7 round trip.
- **The footer is the trust anchor.** A reader validates the footer
  checksums before trusting any offset in it, and validates the
  manifest checksum before trusting the manifest. A shard that
  fails any checksum is corrupt, never partially readable.

## 3. Manifest schema

One JSON object. Field names are stable within `format_version` 1;
a reader must ignore unknown fields (additive evolution), and a
writer must not rename or repurpose a field — a breaking change is
`format_version` 2.

| field               | content                                                                                                                                  |
| ------------------- | ---------------------------------------------------------------------------------------------------------------------------------------- |
| `format_version`    | 1                                                                                                                                        |
| `shard_id`          | UUID minted at pack time; the shard's name in the cold catalog (§5)                                                                      |
| `corpus` / `tier`   | corpus name; tier is the literal `cold`                                                                                                  |
| `created_at`        | RFC 3339 UTC pack time                                                                                                                   |
| `created_by`        | writer identity: Atlas build/version + export run id                                                                                     |
| `source_marks`      | one entry per contributing source: source id, source-native coordinates (repo + commit range, store + object range, log + message range) |
| `versions`          | one entry per packed version (§3.1)                                                                                                      |
| `span_table`        | one entry per distinct chunk (§3.2)                                                                                                      |
| `vector_model`      | embedding model id + revision that produced the vectors                                                                                  |
| `vector_dimensions` | dimensions per vector (768 for the current corpus model)                                                                                 |
| `vector_dtype`      | `f32le`                                                                                                                                  |
| `vector_table`      | one entry per vector (§3.2)                                                                                                              |
| `retention`         | retention state at pack time (§3.3)                                                                                                      |
| `provenance`        | hot shard id the versions were exported from + export run id                                                                             |

### 3.1 Version entries

Per #6026 §2, a version is `(file_id, version_seq)` and immutable.
Each entry carries: `file_id`, `version_seq`, `path_at_version`,
`content_hash` (verbatim from hot metadata), `ingested_at` (RFC
3339), source coordinates (source id + source-native id/commit),
and a **lineage summary**: the version's predecessor
`(file_id, version_seq)` and the edge kind that produced it, from
the #6026 §2.2 vocabulary (`edit`, `rename`, `split`, `merge`,
`tombstone`). The summary is deliberately a summary — the full
lineage graph stays in the hot metadata — but it is enough for a
restored version to re-enter the graph at the right place and for a
reader with only the shard to state what the version succeeded.

Each version entry also carries its **occurrence list** (#6026
§2.3): `(span_ref, span_start, span_len)` triples binding span-table
entries to byte ranges of this version, in version order. This is
what makes byte-identical reconstruction (§7) a check over the
occurrence list rather than over file bytes stored redundantly per
version.

### 3.2 Span and vector tables

- `span_table` entry: `span_ref` (the chunk's content hash, verbatim
  from hot metadata), `offset` and `length` in the decompressed
  span section. A chunk shared by twelve packed versions appears
  once (§8's packing rules keep that the common case).
- `vector_table` entry: `span_ref` (the chunk the vector embeds),
  `index` into the vector section (vectors are fixed-size, so the
  index is the offset: `index × vector_dimensions × 4` bytes).
  A chunk with no embedding (embedding failed or not applicable)
  has a span entry and no vector entry; absence is data, not an
  error, and the manifest states it by omission exactly as the hot
  tier does.

### 3.3 Retention state

`retention` records, for the shard as packed: the corpus class
(#6026 §4.2 vocabulary), `state: "cold"`, `cold_since` (RFC 3339),
`retire_not_before` (the earliest instant any version in the shard
may be retired — §8), and the trigger snapshot that made the
versions cold-eligible (§9), so a later reader can see _why_ the
shard exists without the rule engine's database. Retention state
is a statement about pack time; it is never updated in place
(§2 immutability). A version's current state lives in the hot
catalog, not in the shard.

## 4. Packing rules

The mover (#6026 §6) packs versions into shards under these rules:

1. One corpus and one class per shard; never mix classes (their
   retention clocks differ, §8) and never mix corpora.
2. Target object size ≈ 512 MiB, hard ceiling 2 GiB (rustfs/S3
   single-object ergonomics: resumable transfer, whole-object
   checksum verification on restore).
3. Group by lineage where the eligible set allows: versions of one
   file pack together, so a file's history restores from one object
   and shared chunks dedupe within the shard (§3.2).
4. A version packs whole or not at all; a shard never contains a
   partial version.
5. Packing is read-only against hot storage and idempotent per
   export run: re-running an interrupted pack produces a shard with
   the same content and a new `shard_id`, and the catalog (§5)
   decides which one is canonical.

## 5. Fan-out contract

Cold shards sit **outside the default fan-out set**. Default and
`version = latest` queries never open a cold shard. The hot tier
keeps a **cold catalog** — one row per canonical shard:
`shard_id`, object key, manifest SHA-256, corpus, class, and the
`(file_id, version_seq)` ranges it holds — which is the only index
the query path consults about cold content.

A cold version is reached in exactly two ways:

- **Explicit addressing** (#6026 §5.3): a pinned version
  (`version = <version_seq>`) or an `as_of` resolution that lands
  on a cold version names it deliberately; the catalog resolves it
  to its shard and the restore path (§6) serves it.
- **Shard-addressed search**: a query that explicitly names
  `tier: cold` (optionally `shard: <shard_id>`) fans out to cold
  shards as read-only search targets, using the manifest's span and
  vector tables in place — no restore required for search, restore
  required only to make a version hot again.

There is no third way: nothing promotes cold content into a default
result set silently, and a cold version's absence from default
results is a stated property of the response path (the catalog
knows the version is cold), never a silent miss.

## 6. Restore / re-import procedure

Restore makes one version — or every version in a shard — hot
again, with provenance intact:

1. Resolve `(file_id, version_seq)` → `shard_id` via the cold
   catalog; fetch the object from rustfs.
2. Verify the footer: trailer magic, then SHA-256 of the manifest
   and of each section to be read. Any mismatch aborts the restore;
   a corrupt shard is reported, never partially imported.
3. Parse the manifest. For each version being restored, reconstruct
   its bytes from the occurrence list (§3.1): spans in order, byte
   ranges as listed.
4. Verify reconstruction against the version's `content_hash`
   before writing anything hot. Mismatch aborts that version.
5. Re-import into the target hot shard under the version's original
   identity — same `(file_id, version_seq)`, same content hash,
   same lineage summary re-attached as its lineage edge — plus a
   provenance record naming the restore: source `shard_id`, export
   run id, restore run id, restore time. Provenance is additive;
   the original ingestion provenance is never overwritten.
6. Vectors re-import from the vector table as-is (bit-identical
   f32); no re-embedding. If `vector_model` in the manifest differs
   from the hot tier's current model, the version restores with its
   original vectors and is flagged for the hot tier's normal
   re-embedding policy — restore never silently mixes models in a
   search set.
7. Mark the version hot in the catalog. The cold shard is **not**
   deleted by a restore: restore is a copy, and the version's cold
   copy remains until the mover or §8 retirement acts on it.

Restore is idempotent: restoring a version that is already hot with
the same content hash is a no-op success; restoring one whose hot
copy differs is a conflict and is reported, never overwritten.

## 7. Round-trip integrity — the acceptance shape

The acceptance check for every implementation follow-on (§10) is
the named check **`cold-roundtrip-verify`**:

> Export a set of versions hot → cold (pack per §4), restore them
> cold → hot into a scratch hot shard (§6), and compare: for every
> version, the reconstructed span byte sequence must be
> **byte-identical** to the hot original — compared per span by
> content hash and in aggregate by the version's `content_hash`.
> Vectors must be bit-identical f32 sequences. Manifest fields
> (identity, lineage summary, provenance, retention state) must
> survive the round trip with the §6 provenance additions as the
> only differences.

No signed or served object is discarded anywhere in the round
trip: export copies, restore copies, and the check compares copies
against copies. A failure names the version and the first divergent
span; it never degrades to a warning.

## 8. Retention-state and cold-before-delete boundary (#6041)

States of a version: **hot → cold → retired**. This doc owns the
hot → cold transition's format (§§2–4) and the cold state's
representation (§3.3). The retirement decision and its execution
belong to the retention rule engine, **#6041** — this doc does not
duplicate that policy engine, restate its triggers as deletion
rules, or authorize any deletion.

The boundary, stated as format-level invariants the engine and the
mover both rely on:

- **Cold before delete.** A version is retired only from the cold
  tier, never directly from hot. Deletion is the only hard end of
  a version's life (#6026 §6), and the path to it passes through a
  cold shard, so every retired version was once restorable from a
  self-describing object.
- **One full cycle in cold.** `retire_not_before` (§3.3) is set at
  pack time to at least one full retention cycle after
  `cold_since` (per the growth model's recalibration rule: cold
  moves precede deletions by at least one full retention cycle).
  The engine may retire later than that instant, never earlier.
- **Retirement is whole-shard or a rewrite, never an edit.** A
  shard is deleted only when every version it holds is retired.
  Retiring some of a shard's versions produces a successor shard
  (re-pack of the survivors under §4, new `shard_id`, catalog
  updated) — shards are never edited in place (§2).
- **Tombstones stay hot.** The retirement tombstone record (#6026
  §4.4: file id, version seq, content hash, trigger, rule, run id)
  lives in hot metadata, so the corpus can always answer "what was
  here and why is it gone" after the shard itself is deleted.

Layer boundary, for the avoidance of doubt: Turso point-in-time
recovery is whole-database disaster recovery and is **not** a
corpus-retention mechanism; per the #6043 decision record it plays
no part in this format, the restore path, or any version's
lifecycle.

## 9. Cold thresholds (starting defaults)

#6026 §6 leaves the cold thresholds to this doc; §4.2's retirement
defaults there are the engine's (#6041) to parameterize, and these
cold defaults are likewise starting values for the engine to tune
from dry-run reports — not constants in code. A non-head version
becomes cold-eligible when **either** column fires:

| class             | by age since superseded | by increments beyond it |
| ----------------- | ----------------------- | ----------------------- |
| code              | 90 d                    | 10 newer versions       |
| docs              | 180 d                   | 10 newer versions       |
| session logs      | 45 d                    | 5 newer versions        |
| chat logs         | 90 d                    | 5 newer versions        |
| generated reports | 30 d                    | 3 newer versions        |

Reading the table: every threshold sits well inside the matching
§4.2 retirement threshold of the #6026 design (a version rolls cold
long before any retirement trigger could fire on it, per #6026
§6), and the head version of a live file is never cold-eligible.
Access history (§6026 §4.3) may _defer_ cold-eligibility — a
version surfaced within its class's access window stays hot — but
never accelerate it: cold movement is a latency decision, and a
version that keeps being served has earned its latency class.

## 10. Implementation split (follow-ons)

Implementation is split into follow-on issues, filed under #6026;
this task lands the design only.

1. **Shard writer (exporter)** — the §2 container, §3 manifest,
   and §4 packing rules as the hot → cold mover's write side,
   with the §7 check wired into its own test suite.
2. **Restore importer** — §6 end to end: catalog resolution,
   checksum verification, reconstruction, provenance-preserving
   re-import, model-mismatch flagging.
3. **Cold catalog + fan-out addressing** — the §5 catalog and the
   two addressing paths (explicit version reach, shard-addressed
   search), coordinated with #5755 (fan-out) and #5774 (parameter
   surface).
4. **Round-trip verifier** — `cold-roundtrip-verify` (§7) as a
   standalone tool the writer and importer both gate on.
5. **Shard compaction** — the §8 successor-shard rewrite for
   partial retirement, driven by #6041's retirement output.

## 11. What this design does not decide

- The rule engine's parameters and dry-run operation (#6041);
  §9's numbers are starting defaults only.
- Whether shard-addressed search (§5) ships in the first cold
  release or follows explicit-addressing restore; the contract
  above supports either order.
- Compression beyond the span section's optional zstd, and any
  future `format_version` 2 concerns (columnar vector storage,
  per-span compression) — recorded as open questions for the
  writer follow-on, not decided here.
