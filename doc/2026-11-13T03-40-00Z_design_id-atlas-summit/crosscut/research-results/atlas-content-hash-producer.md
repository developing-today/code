# Atlas `content_hash` producer trace — 9110-R2 research record

Issue: https://github.com/developing-today/code/issues/10147 (slice 9110-R2)
Parent: #9113 · Statement IDs: S0.7, S0.13, S1.118
Design basis: `crosscut/versioned-design.md`, "Remaining evidence gaps
and conservative defaults" (lines 39, 89, 103).
Traced at: `main` 67596fcbac55 (2026-11-16). Method: exhaustive
checked-in-source search (see "Searches performed"); no production
rollout is authorized or implied by this record.

## Verdict (oracle branch taken)

**Producer unavailable.** No checked-in production producer of the
Atlas corpus chunk `content_hash` exists in this repository. The
value is declared in schemas and consumed as a cross-store identity,
but nothing in the repo computes it over chunk content for a
production corpus. Per the design basis (§103):

> Until #9110-R2 locates and pins one and provides a known-answer
> vector, make no cross-system content-hash interoperability claim
> and do not infer an algorithm from schema, consumers, fixture
> strings, or vector joins.

This record takes that branch of the research-matrix oracle: it
records **producer unavailable** and **forbids interop claims** —
including any claim that Atlas chunk `content_hash` equals an ID
blob/content hash for the same bytes — until a producer is pinned
to the contract in "What a future producer must pin" below.

## What `content_hash` names (three different things)

1. **Corpus chunk identity** — `chunks.content_hash` in Atlas corpus
   databases (and `chunks_raw.content_hash` in staging). This is the
   value 9110-R2 asks about. Consumers treat it as THE identity of a
   chunk across id spaces: `crates/atlas-core/src/local.rs` ("(id
   spaces differ between stores; content_hash is the identity)",
   mapping `.vec` side stores to corpus rows) and the mirrored
   Python surface `pkgs/atlas-search/atlas_search.py`.
2. **Classification manifest hash** — `Manifest::compute_hash()` in
   `crates/atlas-core/src/manifest.rs`. This one HAS a checked-in
   producer, and it is pinned exactly: SHA-256 (lowercase hex) over
   the canonical form `v{version}\n` followed by one
   `{name}|{shape}|{semantics}|{label_column}\n` line per dimension
   in declaration order. It is the manifest surface identity — a
   different object from chunk identity, over different bytes. It
   must not be cited as the chunk `content_hash` algorithm; the
   known-answer vectors in `pkgs/atlas-search/test-vectors/
   content-hash-v1.json` record it under its own scope solely to
   keep the two apart.
3. **Trace/query hashes** — `crates/atlas-core/src/trace.rs`
   computes SHA-256 over normalised *query* text, and trace rows
   carry a nullable `embed_content_hash` linkage field. Neither
   produces chunk `content_hash`.

## Consumer inventory (chunk `content_hash`)

- `pkgs/atlas-search/crates/atlas-core/src/local.rs` — reads
  `chunks(id, content_hash)` from corpus and side stores; joins
  vector stores to corpus chunks by hash.
- `pkgs/atlas-search/atlas_search.py` — the same join on the Python
  surface.
- `pkgs/atlas-search/schema/atlas_stage_v1.sql` — staging DDL:
  `chunks_raw.content_hash TEXT NOT NULL` with a hash index. The
  staging layer (`crates/atlas-core/src/stage.rs`) stores the value
  but never computes it: staging writers (unit owners) supply it.
- `pkgs/atlas-search/eval/eval3.py` — corpus fingerprint check:
  SHA-256 over the *stored hash strings* of sampled chunks against
  a frozen expectation (`EXPECTED_CHUNKS`/`EXPECTED_SHA`). A
  consumer-side integrity oracle over an existing corpus — it
  presupposes the hashes; it does not produce them.
- Fixtures (`fixtures/fixture_corpus.sql`,
  `fixtures/dns-search-corpus.sql`, `fixtures/gen_fixtures.py`) and
  crate tests insert literal hash strings (e.g. `'h9002'`);
  `gen_fixtures.py`'s SHA-256 is over fixture *files*, for
  provenance. None of these is a production producer.

## Searches performed (2026-11-16, tree at 67596fcbac55)

- `content_hash` across `pkgs/atlas-search` (Rust, Python, SQL,
  worker TypeScript): consumers and schema only, per above.
- `INSERT INTO chunks` / `CREATE TABLE chunks` across the whole
  repo (all languages): only tests and fixtures write chunk rows.
- Hash computation in Atlas surfaces (`sha256`, `Sha256`,
  `hashlib`) across `pkgs/atlas-search`: manifest `compute_hash`
  (manifest bytes), trace query-text hash, eval fingerprint,
  fixture file hash — none over chunk content for a corpus.
- Native GitHub blockers on #10147 (`dependencies/blocked_by`):
  none, matching the issue's own "Hard dependencies: none".

Production corpus databases are data artifacts provisioned outside
this repository; their builder is not checked in, so the bytes and
profile behind their stored hashes cannot be pinned from source.

## What a future producer must pin (the producer contract)

Before any interoperability claim, a follow-up must record, in
`pkgs/atlas-search/test-vectors/content-hash-v1.json` (status
flipped from `producer_unavailable`):

1. The producer's identity and location (repo path or external
   pipeline reference) and the corpus builds it feeds.
2. The exact input bytes: which chunk fields, in which order and
   encoding, including any normalisation (the trace query hash's
   trim/whitespace-collapse is an example of a normalisation that
   must be stated, not assumed) and any framing/prefix.
3. The algorithm and profile (hash function, output encoding,
   truncation if any) as a named profile, versioned with the
   vector file.
4. A known-answer vector: at least one chunk whose bytes are
   reproduced in the vector file and whose stated hash an
   independent implementation (the validator below, extended)
   recomputes exactly.
5. The relationship, if any, to ID-side hashes (Iroh BLAKE3
   `ContentId`, id-store blob hashes) stated as a separate
   verified claim — never inferred from equal-looking strings.

## Validator

`doc/2026-11-13T03-40-00Z_design_id-atlas-summit/crosscut/validate_atlas_content_hash_research.py`
(stdlib only) re-derives every checkable claim in this record:
the vector file's status and emptiness for the chunk scope, the
manifest known-answer vectors by independent recomputation, and
the presence of this record's required findings (verdict,
forbidden-claims statement, statement IDs, consumer inventory).
Exit 0 iff all checks pass.
