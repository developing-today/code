# Architecture

Source: https://github.com/developing-today/code/blob/main/pkgs/atlas-search/crates/atlas-core/src/lib.rs, https://github.com/developing-today/code/blob/main/pkgs/atlas-search/crates/atlas-core/src/backend.rs, https://github.com/developing-today/code/blob/main/pkgs/atlas-search/crates/atlas-core/src/shards/mod.rs, https://github.com/developing-today/code/blob/main/pkgs/atlas-search/worker/src/index.ts, https://github.com/developing-today/code/blob/main/pkgs/atlas-search/worker/src/service.ts, https://github.com/developing-today/code/blob/main/pkgs/fleet/crates/fleet-search/src/lib.rs

Atlas is a unified search index over every corpus the user produces: AI-client
conversations first, and — through the same staging discipline — notes, repo
docs, GitHub comments, query traces, and research. One logical pipeline carries
every source from raw material to a searchable chunk, and one retrieval core
serves every consumer. This chapter draws the whole system on one map. The
corpus itself is covered in [The corpus: provenance and content addressing](corpus-and-provenance.md);
the access rules that cut across every box on the map are covered in
[Sensitivity tiers and client ceilings](sensitivity.md).

One honesty note before the map: the pipeline below is the system's logical
shape. The left half of it is not yet an automatic, standing pipeline — ingest
today is manual and script-driven, and the staging-to-core promotion path
exists as tools a producer runs, not as a service that runs itself. Where a
step is a tool rather than a daemon, this chapter says so.

## The map

```
 SOURCES                    laptop, ~/atlas/ and client stores
   AI-client conversations (muse, chatgpt, claude, opencode, ...)
   GitHub comments - notes and repo docs - query traces - research
        |
        v
 STAGING                    per-unit atlas_stage v1 databases + registry
   one small DB per unit of work; free, unmediated writes;
   sensitivity recorded, not enforced; staging DBs never hold vectors
        |
        v
 CLASSIFICATION             scheme-1.0 labels + per-chunk records
   kind / topics / projects / sensitivity per session;
   chunks cut only at declared source boundaries
        |
        v
 PROMOTION                  the ingest gate + the aggregator
   write -> classify -> dedup -> provenance-check -> publish;
   chunks embedded document-side at promotion, one model only
        |
        +------------------------------+
        v                              v
 CANONICAL STORE                 SHARD SET
 laptop atlas.db:                atlas-core-<tier>-<topic>-<nn>
 sessions -> messages            scan shards + per-tier content
 -> chunks; Turso publish        stores, discovered by manifest
 monolith (read-only token)
        |                              |
        +--------------+---------------+
                       v
 EMBEDDING             Snowflake arctic-embed-m-v2.0, 768 dims,
                       in-process ONNX; queries get the "query: " prompt,
                       documents are embedded prompt-free
                       |
                       v
 RETRIEVAL CORE        atlas-core run_search: FTS branch + vector
                       branch, ceiling and filters inside the branch
                       queries (before ranking), RRF fusion, then a
                       disclosure check at the output boundary
                       |
        +--------------+---------------+
        v              v               v
 CLI              SERVICE          WORKER + MCP
 atlas-search /   fleet serve      Cloudflare Worker:
 fleet search     search, job      REST + one MCP search
 (standalone or   protocol on      tool, per-query cache;
 via-service)     loopback :18760  holds NO DB credentials
        |              |               |
        +--------------+---------------+
                       v
 CONSUMERS             Drew, fleet agents and sessions, hosted
                       agents via mcpx, offline evaluation
```

## Stage by stage

**Sources.** A _source_ is anything that produces text worth searching again:
a client's on-disk conversation store, a GitHub comment thread, a batch of
notes. Sources are the truth; everything downstream is derived from them and
can in principle be rebuilt. The extracts store that preserves source material
content-addressed is part of this layer — see
[The corpus: provenance and content addressing](corpus-and-provenance.md).

**Staging.** Producers write into a _staging unit_: one small database per
unit of work (a queue task, a session, an epic, a notes batch), provisioned
with the versioned `atlas_stage` v1 schema — `items`, `chunks_raw`, `notes`,
`kv`, and a schema-version row. A registry database maps each unit id to its
staging database, owner, sensitivity, and state (`open`, `closing`,
`merged`). Writes at this stage are deliberately free and unmediated:
sensitivity is recorded on each item, not enforced, and a staging database
never stores vectors, which keeps it cheap to write and cheap to branch.

**Classification.** Before anything is promoted, it is classified. Sessions
carry scheme-1.0 labels — `kind`, `topics`, `projects`, and `sensitivity` —
and classified artifacts carry a classification record per chunk. Chunking
respects source boundaries: classification may change only at a declared
boundary (a message or turn, an authored block, a code symbol, a log record),
and text whose boundaries are undeclared or uncertain becomes a single
`secret-bearing` chunk marked uncertain. The vocabulary and the enforcement
are the subject of [Sensitivity tiers and client ceilings](sensitivity.md).

**Promotion.** Promotion is the only door from staging into the searchable
store, and it is gated twice. The batch ingest gate judges each batch through
five ordered stages — write, classify, dedup, provenance-check, publish —
recording verdicts, reason codes, and hashes, never content; a batch that
fails stops at the failing stage or is quarantined. The aggregator then does
the one embed-at-promotion step: it reads a staging unit's raw chunks, embeds
every chunk document-side with the mandated model, writes one new scan shard,
stamps the unit's provenance, and promotes it through the registry. A unit
whose recorded embedding model is not the mandated one does not promote —
vectors from different embedding models are not comparable.

**Canonical store and shards.** The canonical store is the laptop database
`~/atlas/turso-atlas/data/atlas.db`, shaped `sessions -> messages -> chunks`,
with a published copy — the _monolith_ — hosted on Turso. Beside it sits the
shard set: scan shards named `atlas-core-<tier>-<topic>-<nn>`, each a small
database holding chunks and their vectors for one tier and topic, plus one
content store per tier holding the messages and sessions that shard-served
results hydrate from. Shards join the serving set by existing and describing
themselves in a manifest table, never by a code change; discovery cross-checks
each candidate's manifest against its name and records every rejection with
its reason. Which of the monolith and the shard set serves a given query is a
backend decision inside the retrieval core; the production default is still
the monolith scan, with shard fan-out behind it.

**Embedding.** There is one production embedding model:
`Snowflake/snowflake-arctic-embed-m-v2.0`, 768 dimensions, run in-process as a
dynamically loaded ONNX session — nothing is downloaded at build or run time.
The query side and the document side are deliberately asymmetric: documents
are embedded prompt-free, at promotion; a query is embedded at search time
with the `query: ` prompt the model's configuration specifies. The details
and the bit-parity contract with the Python reference pipeline are in
[Embeddings: Snowflake Arctic and the parity contract](embeddings.md).

**Retrieval core.** All search semantics live in one library function,
`run_search` in atlas-core. A hybrid search runs a full-text branch (FTS5,
BM25-ranked) and a vector branch (exact cosine scan) and fuses the two
candidate pools with reciprocal rank fusion. The caller's sensitivity ceiling
and every filter are applied _inside_ the branch queries, before any ranking
— a chunk above the ceiling is never retrieved, counted, or leaked by its
absence. After fusion, a disclosure check at the output boundary intersects
the caller's ceiling with the destination surface's ceiling, and only the
permitted set is rendered. [Retrieval: hybrid full-text and vector search](retrieval.md)
walks the pipeline in order.

## The crates and their roles

The Rust workspace is `pkgs/atlas-search/`, four crates:

| Crate          | Role                                                                                                                                                                                                                                                                                                                                                                                             |
| -------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| `atlas-core`   | The engine as a library: search orchestration and the result envelope, the FTS and vector branches and their fusion, filters and sensitivity ceilings, the Turso and local backends, staging, classification, the promotion aggregator, shards, evaluation, traces and feedback. It also carries the helper binaries `atlas-classify`, `atlas-eval`, `atlas-github-comments`, and `atlas-sweep`. |
| `atlas-embed`  | The Snowflake embedder: dynamically loaded ONNX Runtime, the model's own Unigram tokenizer, CLS pooling, full-768 output. It knows nothing about queries or prompts; that policy lives in atlas-core.                                                                                                                                                                                            |
| `atlas-proto`  | The wire contract: serde types only — no HTTP, no storage, no search policy — shared by the service and the via-service CLI client so their definitions cannot drift. It also carries the versioned query-parameter defaults both sides read.                                                                                                                                                    |
| `atlas-search` | The binary: the one-shot CLI, the via-service client, and `serve` mode, plus the `atlas-stage` staging tool.                                                                                                                                                                                                                                                                                     |

Two further implementations shadow these. `atlas_search.py`, kept in the same
package, is the production Python implementation the Rust was ported from; it
remains the permanent evaluation and parity reference. And
`pkgs/fleet/crates/fleet-search/` is a flag-for-flag copy of the atlas-search
client and service inside the fleet binary — `fleet search` and
`fleet serve search` — held byte-for-byte against the original by a parity
guard in CI. The Cloudflare Worker is TypeScript, not a crate; it is covered
under surfaces below.

## Three surfaces, one execution path each

**The CLI** (`atlas-search`, or `fleet search`) is standalone-capable: with a
local or Turso backend and a local model it needs no service at all. It can
also route through the service (`--via-service`). The seat decides the
default: the sandbox seat prefers the service with a loud standalone fallback;
the laptop seat — the serving host — defaults to standalone.

**The service** (`fleet serve search`, also reachable as `atlas-search
serve`) is the same engine behind a loopback HTTP job protocol on port 18760:
submit a job, poll its status, page its immutable fused ranking. Serve mode's
value is resident state — one lazily loaded embedder session shared by every
job, instead of a model load per invocation. Its guarantees are covered in
[The service and the Worker](service-and-worker.md).

**The Worker and MCP surface** is a Cloudflare Worker in
`pkgs/atlas-search/worker/`. It exposes a gated REST search route and a
stateless MCP endpoint with exactly one tool, `search`; both call a single
execution function, so ceiling pinning and cache keying cannot drift between
them. Results are cached per query in a Durable Object. The laptop's mcpx
connects to the MCP endpoint as its remote `atlas` server.

## Where the trust boundaries sit

Atlas is a single-user system, but it is not a single-trust-zone system. The
boundaries, in the order data crosses them:

1. **Source to staging.** Staging writes are unmediated by design. Nothing in
   a staging database is trusted to be correctly classified, deduplicated, or
   safe to serve; that is what the next boundary is for.
2. **Staging to core.** The promotion gate is the boundary: classification,
   dedup, and provenance verdicts, the embedding-model contract, and a gate
   record that holds counts, codes, and hashes — never content.
3. **Laptop to hosted publish.** The canonical database lives on the laptop;
   the Turso monolith is a published copy, and Turso never holds the only copy
   of anything. The service reaches it with a read-only token and issues
   nothing but SELECT statements — it holds no write credential for any Turso
   database, and there is no write path to opt into.
4. **Caller to service.** Every service client is a named identity presenting
   a per-client token. The _server_ clamps the requested sensitivity ceiling
   to that identity's cap; a client cannot widen its own access by asking.
5. **Service to shard set.** Tier ceilings are physical: a shard above the
   ceiling, or of a tier with no credential, is never connected, never
   probed, never scanned. Each tier has its own read-only token, and the
   secret tier's is never derived from another tier's credential.
6. **The Worker boundary.** The Worker holds no database credentials at all —
   no Turso token, no shard list. Fan-out, embedding, and merging live in the
   fleet search service, which the Worker calls over a tunnel hostname with a
   Cloudflare Access service token, presenting its own client token as the
   `cf-worker` identity (ceiling `personal`). The Worker pins that ceiling
   and its disclosure context as compile-time constants and verifies the
   service's reported effective ceiling before any result enters its cache.
7. **The output boundary.** Even after retrieval, results pass the disclosure
   check: the destination surface's ceiling intersects the caller's, and
   cached or paged payloads inherit the filter because the served set is
   collected from the rendered envelope.
8. **The embedder boundary.** Embedding happens in-process, on hardware the user
   controls. Secret-tier text is never sent to a hosted embedder — a rule
   stated in full in [Sensitivity tiers and client ceilings](sensitivity.md).

Consumers sit outside every boundary above and inherit the strictest one they
cross: The user's own CLI is the only identity granted the `secret-bearing`
ceiling; agents on the sandbox seat search at `personal`; hosted callers
arrive through the Worker at `personal` under the `hosted-per-user`
disclosure context. Who consumes Atlas, and by which path, is the subject of
[Consumers: fleet, agents, memory and ID](consumers.md). How new material
crosses the first three boundaries in practice is
[Ingest today: staging, classification, promotion](ingest.md).
