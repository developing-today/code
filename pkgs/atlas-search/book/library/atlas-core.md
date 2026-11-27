# atlas-core: the engine as a library

Source: `crates/atlas-core/src/lib.rs` and the module files under
`crates/atlas-core/src/` (including `src/shards/`), with call sites in
`crates/atlas-search/src/main.rs`, in
https://github.com/developing-today/code/blob/main/pkgs/atlas-search/ —
provenance only; this chapter is complete without following it.

`atlas-core` is Atlas with the process removed. Everything the binaries do
that is not argument parsing, printing or HTTP lives here: the search
pipeline, both data backends, the shard fan-out, staging and promotion,
classification, evaluation, traces and feedback. Its crate documentation
states the shape plainly: a `SearchBackend` — the Turso HTTPS pipeline API,
or a local SQLite publish file — gathers a full-text pool and a vector pool
with the caller's sensitivity ceiling and filters applied _inside the branch
queries, before any ranking_, and `run_search` fuses the pools with
reciprocal rank fusion under the versioned query-parameter contract.

The library is synchronous. Backends are `Send + Sync` behind interior
locks. The query embedder is the one exception: ONNX sessions take a mutable
reference, so serve mode holds one embedder, or a small pool, behind a
mutex — a loaded session is about 1.6 GB of resident memory, and it is
shared state, not per-request state.

The crate is a semantics-exact port of the Python reference — see
[atlas_search.py: the Python reference implementation](../reference/python-implementation.md).
JSON shapes, field order, note placement and fatal messages mirror the
Python, and the parity tests diff rendered envelopes and SQL strings against
fixtures recorded from the production Python module.

## The 50 public modules, grouped by job

`lib.rs` declares 50 public modules. Grouped by the job they do:

### Retrieval

| Module    | Job                                                                                                                                                                                 |
| --------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `search`  | Search orchestration and envelope rendering: `run_search`, `run_get_chunk`, the search options and modes                                                                            |
| `fusion`  | The FTS match expression and reciprocal rank fusion over the two branch pools                                                                                                       |
| `filters` | The filter set, the sensitivity ceiling type, and the branch and detail SQL builders. The SQL strings are the contract with the database; parity fixtures record them byte for byte |
| `join`    | The span join and reconstruction assembler: assembling a paragraph of a version from its span occurrences, byte for byte, with cold and retired (tombstone) rendering               |

### Embedding

| Module        | Job                                                                                                                                                                                            |
| ------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `embed`       | Query and document embedding at the library boundary: the `query: ` prompt, truncation and renormalisation on the query side; prompt-free embedding on the document side                       |
| `embed_cache` | The query-embedding cache: an in-memory tier plus an optional shared Turso tier, keyed by model identity and normalised query text. See [Data stores and schemas](../reference/data-stores.md) |

### Corpus

| Module            | Job                                                                                                                                                                                                                                                                         |
| ----------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `catalog`         | The hot-side cold catalog: maps explicitly addressed file versions to cold shards and builds search scopes for queries that name cold tiers or shards; default fan-out does not open cold shards                                                                            |
| `shards`          | The core tier as a set of scan shards: discovery, naming, tiers, manifests, fan-out scan, content stores and the local-file transport (submodules `shards::scan`, `shards::content`, `shards::local`)                                                                       |
| `turso`           | The Turso backend: the libsql HTTPS pipeline against the hosted DB with a read-only token. Every statement it builds is a `SELECT`                                                                                                                                          |
| `local`           | The local backend: a SQLite publish file, schema-normalising temporary views, and vector side stores joined by content hash with an in-process cosine scan                                                                                                                  |
| `backend`         | The `SearchBackend` seam both backends implement: branch pools, chunk details, neighbours and message reads, with the ceiling enforced inside branch construction                                                                                                           |
| `manifest`        | The classification manifest: schema, validation and loader for the table that declares the filterable dimensions, with the built-in manifest as fallback and validation reference                                                                                           |
| `occurrences`     | Resolves a canonical content hash back to its chunk occurrences across the shards visible at a caller's sensitivity ceiling                                                                                                                                                 |
| `cold`            | Verifies a fetched cold-tier shard and reconstructs version bytes from its manifest and occurrence lists                                                                                                                                                                    |
| `cold_shard`      | The cold-tier shard container format: one immutable object (header, JSON manifest, span and vector sections, checksummed footer) with the writer and reader that pack and unpack it — the write side of the hot → cold mover (#6042)                                                                                                                                                                                   |
| `memory`          | Registers memory corpora as staging units and parses their read-only exports for Atlas ingestion                                                                                                                                                                            |
| `notes`           | The zettelkasten connector (#8626): scans a notes vault's Markdown files in sorted tree order and stages them into the `zettelkasten` unit through the registry path, replay-safe — re-scanning an unchanged vault stages zero, an edited note re-stages exactly its chunks |
| `reports`         | The research-reports connector (#8626): walks a tree of prior fleet research outputs (goal files and briefs) and stages them into the `research-reports` unit with the design's full provenance set, replay-safe — re-staging an unchanged document writes nothing          |
| `repo_docs`       | The repo-docs connector: stages the four fleet repositories' Markdown docs from explicit local checkouts into the `repo-docs` unit — offline and replay-safe, with no remote discovery or fetching                                                                          |
| `source_registry` | The provenance source registry (#3125): stable source-root and source-locator registration. The read side is `occurrences`, which resolves chunk occurrences by canonical hash back to every place the content was found                                                    |
| `boundary`        | Source-boundary-safe embed splitting: the splitter and the classifier share source offsets, every declared boundary splits, and uncertain text becomes one secret-bearing chunk                                                                                             |
| `disclosure`      | Disclosure contexts: the destination-side half of the classification model. The caller ceiling and the destination context's ceiling intersect; neither widens the other, and enforcement happens at the final output boundary in `search`                                  |

### Ingest

| Module               | Job                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                      |
| -------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `stage`              | Staging databases and the staging-unit registry: provisioning, staged artifacts, classified staging, and the promotion transition                                                                                                                                                                                                                                                                                                                                                                                                                                                                        |
| `gate`               | The staging-shard ingest gate's durable record and state machine: five ordered stages (write, classify, dedup, provenance check, publish), enumerated reason codes, and a strike limit of 3 per reason before a batch needs review                                                                                                                                                                                                                                                                                                                                                                       |
| `attestation`        | One in-toto-shaped attestation statement per ingest batch (#6045): the statement bytes are embedded verbatim as the claim body, with the statement hash as the claim's subject — no transcoding                                                                                                                                                                                                                                                                                                                                                                                                          |
| `id_projection`      | The Atlas-to-ID attestation projector (#11183): one `attestation.claim` per batch digest, projecting ingest attestations and batch runs to ID                                                                                                                                                                                                                                                                                                                                                                                                                                                            |
| `version_projection` | Deterministically projects immutable Atlas version rows into ID document and version/batch claim descriptors; publication remains a later slice                                                                                                                                                                                                                                                                                                                                                                                                                                                          |
| `consumer_preset`    | The consumer view presets (#13766, #15227): three consumer-side selections over one pinned publication — `attested_only`, exactly the versions whose batch has a published `attestation.claim`, joined on `batch_id`; `heads_only`, each file's current head, the tip of its note chain, leaving out superseded heads and non-head backfills (a file with only backfills has no head); and `full_history`, every published version, heads, superseded heads and backfills alike. The three nest inside `full_history`. Any inconsistency refuses the whole selection rather than returning a partial one |
| `sweep`              | The source sweep: candidate filtering, guardrails, headroom, state, locking and run reporting for periodic ingest scheduling                                                                                                                                                                                                                                                                                                                                                                                                                                                                             |
| `github_comments`    | GitHub comments as a corpus source: comment documents, provenance marks and watermarks                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                   |
| `session_stage`      | Shared staging core for conversation-export adapters: retains user-facing messages, chunks text at paragraph boundaries, and stages each message with stable IDs and source metadata                                                                                                                                                                                                                                                                                                                                                                                                                     |
| `muse`               | Muse JSONL conversation-export adapter: reads sorted session exports, accepts flat and text-part messages, and stages them through the shared conversation core                                                                                                                                                                                                                                                                                                                                                                                                                                          |
| `trace_ingest`       | Query traces as a searchable corpus source, plus trace retention and compaction                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                          |

### Classification

| Module           | Job                                                                                                                                                                                                                                                                                                                                                                                                             |
| ---------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `classification` | The classification record model: records, provenance, and the effective classification composed from records. Unknown, missing or malformed sensitivity fails closed to secret-bearing. The one module with no module-doc header and no `lib.rs` re-export — reach it as `atlas_core::classification`                                                                                                           |
| `classify`       | The classification operating procedure: the unclassified-remainder sweep and schema-change review. The `atlas-classify` binary is its front end                                                                                                                                                                                                                                                                 |
| `detect`         | Source detection: passes over plain trees, git trees and row stores, producing a versioned ledger of per-path and per-table verdicts                                                                                                                                                                                                                                                                            |
| `c8_fingerprint` | The C8 error-investigation fingerprint core: error-string normalisation, fingerprints, occurrence grouping and evidence bundles. Pure types and derivations; no telemetry search                                                                                                                                                                                                                                |
| `c8_profile`     | The C8 report and page metadata profile: source and page records, typed links and URL sanitising. Likewise seam-independent                                                                                                                                                                                                                                                                                     |
| `remediation`    | The secret-bearing remediation inventory and destination verifier (#8479, #12406): the read-only, no-write half of the classification-model remediation order — inventories reported chunks, maps source to destination with checksums, and verifies the secret-tier destination's content and vectors before any visibility switch; anything unverified is reported explicitly, never treated as safe to purge |

### Quality

| Module               | Job                                                                                                                                                                                                |
| -------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `eval`               | The offline retrieval evaluation harness: saved rankings from any signal scored against judged qrels (nDCG@10, Recall@100, Recall@1000, MRR@10), with dev/test split discipline                    |
| `feedback`           | Feedback capture in the `atlas-feedback/v1` schema: votes attached to a query and chunk identity, the append-only JSONL sink, and the derived supersede view                                       |
| `feedback_normalise` | The normalisation pass over raw feedback: categories, summaries, self-feedback detection and requester weighting, emitted as derived records — the raw store is never rewritten                    |
| `trace`              | Query-trace capture in the `atlas-query-trace/v1` schema. Every executed search mints a query id and persists one trace record before the response is released; a sink failure is a search failure |
| `trace_stats`        | Trace statistics and the weekly report: windowed aggregation over traces and feedback votes, with the requester weights (the owner and users at 1.0, agents at 0.25, anonymous at 0.1)             |

### Support

| Module            | Job                                                                                                                                                                                                                                                                                                  |
| ----------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `types`           | The shared value and row model between the Turso pipeline encoding and rusqlite: values, rows, statements, branch hits and pools, chunk details, and the canonical defaults (the default model and the default vector table, `vec_snowflake_m_v2_768_f32`)                                           |
| `error`           | The error type and the crate's `Result` alias; see the error vocabulary below                                                                                                                                                                                                                        |
| `aggregate`       | The staging aggregator: embed-at-promotion. It reads one staging unit, embeds every chunk document-side, writes one new core scan shard, stamps provenance and promotes the unit through the registry                                                                                                |
| `owner_inventory` | The owner inventory matcher (`atlas-owner-inventory/v1`): a narrow, fail-closed component whose findings carry a class and an opaque record id, never a matched value                                                                                                                                |
| `shingles`        | Token shingles and the Jaccard rescore signal: a third signal applied only at rescore time over the fused candidate union, never a retrieval scorer. Off by default in the shipped parameters                                                                                                        |
| `version`         | The version strings for the Atlas CLI family (#13279 part 2): every Atlas binary reports `<package version>+g<source revision>` from `--version`, with the revision baked by atlas-core's build script (`ATLAS_SOURCE_REV`); a build with no git history reports `unknown` rather than inventing one |

Four binaries live in this crate's `src/bin/` — `atlas-classify`,
`atlas-eval`, `atlas-github-comments` and `atlas-sweep` — each a thin front
end over the module of the same job. Three examples (`live_smoke`,
`retrieval_proof`, `shard_bench`) exercise the library against live stores.

## Key public types

The types a caller actually constructs or matches on:

| Type                                                      | Module       | What it is                                                                                                                                                                                                                                                                                                                                                           |
| --------------------------------------------------------- | ------------ | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `SearchOptions`                                           | `search`     | One search request: the query, the mode, `k` (default 10), the per-branch `pool` (default 100), grouping, `expand` (default 0), the model, the truncation width (0 = full), the vector backend (default scan), the query parameters, and the disclosure context (default `PublicSurface` — fail-closed at the library layer). Built with `SearchOptions::new(query)` |
| `SearchMode`                                              | `search`     | `Fts`, `Vector` or `Hybrid` (the default)                                                                                                                                                                                                                                                                                                                            |
| `Filters`                                                 | `filters`    | The classification filters and the caller ceiling: kind, topic, project and client lists, the maximum sensitivity, and the since/until dates. Construction validates, with the Python's fatal texts                                                                                                                                                                  |
| `Sensitivity`                                             | `filters`    | The ceiling ladder `Shareable &lt; Personal &lt; LocalOnly &lt; SecretBearing`; the ordering _is_ the security ordering. Default `Personal`                                                                                                                                                                                                                          |
| `DisclosureContext`                                       | `disclosure` | The destination context — `FleetInternal`, `HostedPerUser`, `PublicSurface`, `LocalOwner` — each with its own ceiling; `effective_ceiling(grant)` is the minimum of the two                                                                                                                                                                                          |
| `SearchBackend` (trait)                                   | `backend`    | The data-source seam: `branch_pools`, `chunk_details`, `neighbors`, `message_chunks`, `message`. Implementations: `TursoBackend` and `LocalBackend`                                                                                                                                                                                                                  |
| `QueryEmbedder` / `DocEmbedder` (traits), `AtlasEmbedder` | `embed`      | The embedding seam. `AtlasEmbedder` wraps the `atlas-embed` embedder behind a mutex and implements both traits; fixed-vector embedders stand in for tests and fixtures                                                                                                                                                                                               |
| `BranchPools`, `BranchHit`, `ChunkDetail`                 | `types`      | What a backend returns: the FTS and vector pools (a vector hit carries its cosine distance; an FTS hit carries none), and the resolved detail row for a chunk, including its effective sensitivity                                                                                                                                                                   |
| `Error`, `Result`                                         | `error`      | The error vocabulary, below                                                                                                                                                                                                                                                                                                                                          |
| `Manifest`, `LoadedManifest`                              | `manifest`   | The classification manifest and its loaded form, with the source it came from                                                                                                                                                                                                                                                                                        |
| `Registry`, `StagingUnit`                                 | `stage`      | The staging registry handle and one unit's record                                                                                                                                                                                                                                                                                                                    |
| `GateRecord`                                              | `gate`       | One batch's gate audit record. The gate types are public in their module but are not re-exported at the crate root                                                                                                                                                                                                                                                   |

Most workhorse types are re-exported at the crate root (`atlas_core::run_search`,
`atlas_core::Filters`, `atlas_core::TursoBackend` and so on). The recorded
exceptions are `classification`, the `gate` types, `github_comments`, `sweep`
and the `shards` types, which are reached through their module paths.

Two crate-level constants bound every caller: `MAX_EXPAND_NEIGHBORS = 100`,
the ceiling on neighbour expansion and on `get-chunk` neighbours, and the
message text cap of 50,000 characters applied to message payloads.

## The error vocabulary

The library has exactly three errors, in `atlas_core::Error`:

| Variant                  | Meaning                                                                                                                                                     | Exit code at the `atlas-search` binary |
| ------------------------ | ----------------------------------------------------------------------------------------------------------------------------------------------------------- | -------------------------------------- |
| `Error::Fatal(String)`   | A fatal that mirrors a Python `die()` text: validation failures, unresolvable configuration, a missing model or token. Constructed with `Error::fatal(...)` | `2`                                    |
| `Error::Backend(String)` | A backend or transport failure: the pipeline request, the database, the local file. Constructed with `Error::backend(...)`                                  | `1`                                    |
| `Error::Embed(String)`   | An embedding failure: the model, the runtime, a zero query vector                                                                                           | `1`                                    |

`atlas_core::Result&lt;T&gt;` is the crate-wide alias over this error. The
exit-code column is the binary crate's mapping, not the library's — the
library returns the variant and the caller decides what it costs. One
special case rides on top of it in the binary: a `get-chunk` not-found
arrives as a `Fatal` with the exact message `chunk &lt;id&gt; not found` and
is remapped to exit 1, matching the Python.

## Embedding a search in another binary

The shape of the call, taken from the real call sites in the `atlas-search`
crate's `main.rs`: open a backend, build the filters, build the options,
call `run_search`, and print or serve the returned JSON envelope. The
envelope — not a result struct — is the return value: a
`serde_json::Value` whose shape, field order and notes mirror the Python
exactly.

```rust
use atlas_core::{Filters, LocalBackend, SearchOptions, TursoBackend, run_search};

// Standalone shape: a backend, an embedder, filters, options.
// The Turso backend resolves its read-only token itself:
let backend = TursoBackend::new(vec_table)?;
// …or the local publish file:
// let backend = LocalBackend::new(db_path, vecdb_arg, vec_table, trunc)?;

let filters = Filters::new(/* kind, topic, project and client lists,
    the max-sensitivity ceiling, and the since/until dates */)?;
// construction validates, and its fatals carry the Python's texts

let mut opts = SearchOptions::new(query.to_string());
opts.k = 10;          // defaults already: k 10, pool 100, expand 0,
opts.context = ctx;   // hybrid mode, scan vector backend

let envelope = run_search(&backend, &embedder, &filters, &opts)?;
// envelope is the JSON result document, ready to print or return.
```

What the call sites around that shape add:

- **The embedder is lazy.** FTS-only searches and `get-chunk` never load the
  model. The binary builds the real embedder only when the mode uses the
  vector branch; library callers follow the same rule, passing a fixed or
  absent embedder where no vector work can occur.
- **One chunk, by id:** `run_get_chunk(&backend, chunk_id, neighbors)`
  returns the chunk envelope with that many same-message neighbours attached
  (at most 100). `run_get_chunk_with_context` is the form that also applies
  a disclosure context.
- **Traced searches:** `run_search_traced` (and
  `run_search_with_executor_traced` for a custom branch executor) take a
  `TraceJob` carrying the trace sink, the trace context and the query id,
  and persist the trace before the envelope is released. The one-shot binary
  always searches traced, through a JSONL sink; a sink failure fails the
  search.
- **Custom branch execution:** `run_search_with_executor` accepts any
  `BranchPoolExecutor` in place of the backend's own branch pools — the seam
  the shard fan-out composition uses, serving the vector branch from the
  shard set while the wrapped backend keeps FTS, details and neighbours.

Worked example — the smallest complete embedding, in the shape of the
crate's own examples: open a `LocalBackend` on a publish file, construct
`Filters` with no dimension filters at the default `personal` ceiling,
construct `SearchOptions::new("backup retention")`, and call `run_search`
with a fixed embedder for an FTS-only mode so no model is needed. The
returned envelope names the backend, the filters in force and the ranked
results, exactly as the CLI would print it.

## Failure shapes

- `expand` above 100, a non-scan vector backend on a non-Turso backend, and
  hydration combined with grouped output are all `Fatal` before any branch
  runs.
- In FTS mode, a query with no FTS-searchable terms (every word too short)
  is `Fatal`; in hybrid mode the same query simply runs the vector branch
  alone.
- A shard fan-out in which the scanned shards carry mixed embedding models
  is `Fatal`: vectors from different embedding models are not comparable.
  A failing shard likewise fails the whole query — a partial answer that
  silently omits a shard is treated as worse than an error.
- A missing or empty `HOME` environment variable panics loudly in the path
  expansion used by the Turso and local backends, by design; it is never
  silently replaced with a relative path.

For the workspace this crate sits in, see [The crates](crates.md). For the
stores its backends read, see
[Data stores and schemas](../reference/data-stores.md).
