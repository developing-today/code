# The shape of the system

Atlas has one engine, three access surfaces, and two execution modes. This
chapter is that sentence expanded: what the engine is, what the surfaces
are, how a query moves in each mode, and where the data lives while all of
that happens. It ends with the discipline this book follows for telling
shipped behaviour from designed behaviour, because Atlas has a large
design archive and the difference matters.

Source: https://github.com/developing-today/code/blob/main/pkgs/atlas-search/crates/atlas-core/src/lib.rs and https://github.com/developing-today/code/blob/main/pkgs/atlas-search/crates/atlas-proto/query-parameters-v1.json

## One engine: `atlas-core`

All search semantics live in one Rust library crate, `atlas-core`:
classification filters and sensitivity ceilings, the full-text branch, the
vector branch, reciprocal-rank fusion, and the result envelope. It is a
semantics-exact port of the production Python implementation, and the
Python remains its parity oracle. The engine's shape is a trait and a
pipeline:

- A `SearchBackend` gathers the two branch pools. Two implementations
  ship: `TursoBackend` (the hosted corpus over the libsql HTTPS pipeline
  API, read-only) and `LocalBackend` (a local SQLite publish file, with
  vector side stores joined by content hash). Filters and the caller's
  sensitivity ceiling are applied **inside the branch queries** — before
  any ranking — in both implementations.
- `run_search` then fuses the pools: an FTS5 (BM25) ranked list and a
  cosine-distance ranked list, combined by reciprocal rank fusion and
  rendered as the one envelope every surface returns. The defaults are
  not scattered through the code: they are read from a single versioned
  contract file in `atlas-proto`, `query-parameters-v1.json` — final
  results `k` = 10, per-branch pool = 100, RRF constant = 10, branch
  weights 1.0 and 1.0, expansion 0. The same file is the source the CLI
  defaults are derived from, so the library, the CLI, and the service
  cannot quietly drift apart on what "a default search" means.

Around the engine sit three sibling crates, and the boundaries are the
architecture:

| Crate          | Role                                                                                                                                                                                                                                                                                                                                                                                                             |
| -------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `atlas-core`   | Search semantics as a library, plus the write-side machinery the tools expose: staging and the registry, the ingest gate, classification, traces, feedback, evaluation, and the shard fan-out. Synchronous; the backends are shareable across threads.                                                                                                                                                           |
| `atlas-embed`  | The Snowflake embedder: a dynamically loaded ONNX Runtime session plus the model's own tokenizer, reproducing the production Python embedding pipeline bit-identically (the Gate A parity contract). Output is the full 768 dimensions; query/document handling — the `"query: "` prompt on queries, prompt-free documents, CLS pooling, L2 normalisation — is pinned here and in `atlas-core`'s embed wrappers. |
| `atlas-proto`  | The wire contract only: serde types for the service job protocol and the versioned query-parameter defaults. No HTTP, no storage, no search policy — the server and the via-service client both depend on it so their wire definitions cannot drift. Sensitivity clamping lives in the server; this crate merely carries the requested and effective ceilings.                                                   |
| `atlas-search` | The binary crate: the `atlas-search` CLI, its via-service client, the service implementation, and `fetch-model` — plus the `atlas-stage` binary. The four remaining tools (`atlas-classify`, `atlas-eval`, `atlas-sweep`, `atlas-github-comments`) are binaries of `atlas-core` itself.                                                                                                                          |

The engine is deliberately not a service and not a CLI: it is the library
both are built from, which is what makes "the same search" a checkable
claim rather than a hope. The library part of this book covers the crates
in full, starting with [The crates](../library/crates.md) and
[atlas-core: the engine as a library](../library/atlas-core.md).

## Three access surfaces

Every caller reaches the engine through one of three surfaces. They share
the engine, the envelope, and the ceiling rules; they differ in where the
engine runs and what identity the caller carries.

**1. The command line.** The `atlas-search` binary (and its Python
reference twin, `atlas_search.py`, which offers `search` and `get-chunk`
only) runs the engine in its own process. `fleet search` is the same
surface wearing the fleet binary's routing: it either delegates to the
standalone `atlas-search` artifact or speaks the job protocol to the
service, and renders the same envelope either way.

**2. The service HTTP API.** `atlas-search serve` — in the live laptop
unit, the same service rehomed as `fleet serve search` — runs the engine
behind a loopback HTTP job protocol on `127.0.0.1:18760`, exposed on the
tailnet through a `tailscale serve` mapping. Its routes are a one-shot
`POST /v1/search`; the job protocol proper (`POST /v1/search/jobs`
returning 202 with a job id and a per-job capability token, then status,
paged results, chunk fetch, and delete under that job); `GET /health`
(no auth, no data: liveness, embedder state, queue depth); and two
post-design additions, `POST /v1/embed` and the feedback routes. The
point of serve mode is resident state: one lazy-loaded embedder session
shared by every client (about a second to load, versus roughly 35
seconds of Python cold start), a connection pool, and the job registry.
Clients are named in the service config, each with a bearer token and a
ceiling cap; the server computes
`effective = min(requested, token cap)` and reports the effective ceiling
in the envelope.

**3. The Worker: REST plus MCP.** The Cloudflare Worker is the hosted
surface: gated `POST /v1/search`, cache inspect and purge routes, and
`POST /mcp`, a stateless MCP endpoint with one `search` tool. It contains
no engine and no corpus access: it holds no Turso credentials and no
shard list, pins its disclosure context to `hosted-per-user` and its
ceiling to `personal`, and calls back to the laptop service — through its
tunnel hostname, under a Cloudflare Access service token — for the query
embedding and for the search fan-out itself. What it adds is placement
and caching: it is reachable wherever an MCP client is, and it caches
each query's result in a per-query Durable Object. See
[The service and the Worker](../concepts/service-and-worker.md).

## Two execution modes

Independent of the surface, a search executes in one of two modes:

- **Standalone** (the default, and complete). The engine runs in the
  caller's process: the query is embedded in-process and the backend is
  talked to directly. The CLI is never a thin client; a machine with no
  service running loses nothing but the service's shared state.
- **Via-service.** The caller submits the search to the service's job
  protocol and pages the immutable fused ranking back. `--via-service`
  (or `ATLAS_SEARCH_VIA=service`, or the fleet seat default) expresses a
  _preference_, not a requirement: on any service failure the client
  falls back to standalone **loudly** — a warning on stderr and a note
  spliced into the envelope's `notes`. Some requests cannot be served by
  the service at all — a local-backend request, a non-default vector
  table, or an experimental shard specification — and those take the
  same loud fallback without a service attempt being presented as a
  search failure. `get-chunk` is always standalone, in both clients,
  because chunk fetch in the protocol is scoped to an existing job.

The two-mode rule exists so that search semantics exist exactly once. The
rejected alternative — separate client and server implementations — would
have to pass the parity gate twice against two drifting codebases; with
one engine, the service is a deployment shape of the CLI, not a second
implementation of it.

## Where the data lives

| Store                 | What lives there                                                                                                                                                                                                                                                                                                                                        | Lifetime and rules                                                                                                                                                                                                                                                                                                                                                                                                                                                                                    |
| --------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Canonical Turso store | The published corpus: sessions, messages, chunks, session labels, the FTS5 index, and the vector table `vec_snowflake_m_v2_768_f32`. Read-only to every search path; no search component holds a write credential for it.                                                                                                                               | The hot, served copy. The durable asset is the local canonical database it is published from (`~/atlas/turso-atlas/data/atlas.db` on the laptop); Turso never holds the only copy of anything.                                                                                                                                                                                                                                                                                                        |
| Shards                | Topic shards named `atlas-core-&lt;tier&gt;-&lt;topic&gt;-&lt;nn&gt;`, each self-describing through an `atlas_manifest` table (tier, topic, expected chunk count, embedding model). At the 2026-10-31 census: 28 internal shards and 1 secret shard. Per-tier content stores hold the messages/sessions half that scan shards lack, for hydrating hits. | A shard joins the fan-out by existing and passing manifest discovery, never by a code change. Discovery is opened only up to the caller's ceiling: a shard above the ceiling is never connected, probed, or scanned, and a shard whose embedding model disagrees with the consumer's is refused — vectors from different models are not comparable. Shard fan-out for the vector branch is an experimental, off-by-default path in the CLI; FTS, details, and expansion stay on the monolith backend. |
| Stage databases       | One small SQLite file per unit of ingest work, plus the staging registry that maps unit ids to them. Staging databases store raw, un-embedded chunks and never store vectors, which makes them cheap to write and cheap to branch.                                                                                                                      | Written freely by producers, classified at staging, then promoted — embed-at-promotion into a new scan shard — through the registry's model gate, or not promoted at all. See [Ingest today: staging, classification, promotion](../concepts/ingest.md).                                                                                                                                                                                                                                              |
| Service temp store    | In-memory job state: submitted jobs, their branch pools, and their fused rankings, plus the result cache keyed by query, corpus snapshot, and effective ceiling.                                                                                                                                                                                        | Perishable by requirement: a completed job lives 600,000 ms (10 minutes) by default, with a 3,600,000 ms (60-minute) hard cap from submit, and nothing survives a service restart — job results hold ceiling-gated snippets, and persistence would be a liability.                                                                                                                                                                                                                                    |
| Worker cache          | One Durable Object per (normalised query, `k`, ceiling, context, embedding space) key, holding the query's embedding and then its result.                                                                                                                                                                                                               | Retained indefinitely in version 1 — no TTL and no eviction; the only deletion is the explicit per-key purge route. This is a different store from the service temp store, with the opposite TTL story, and the follow-up that owns revisiting it is issue #6805. A D1 tenant hot-cache storage layer also exists in the Worker package behind a flag committed as `"false"`; nothing in the request path reads or writes it today.                                                                   |
| State directory       | `~/.local/state/atlas-search/`: the service config, the model snapshot, and the append-only query-trace and feedback JSONL stores.                                                                                                                                                                                                                      | Traces are kept hot for 180 days before the retention pass moves them to a cold tier; the trace and feedback stores are themselves ingestible corpus sources (statistics and reports).                                                                                                                                                                                                                                                                                                                |

Two smaller caches sit inside the engine rather than beside it: an
in-service query-embedding cache (a query vector is a pure function of
the normalised query text, the model id, and the model revision, so it
needs no TTL or invalidation), and the embedding cache's optional shared
Turso tier, enabled only when both of its environment variables are set.

## The designed-versus-shipped discipline

Atlas accumulates design records faster than it ships code, so this book
follows one rule everywhere: **present tense means shipped, verified
against the source; anything else is labelled.** Concretely:

- Chapters in the design part open by stating that they are designs, name
  their tracking issues, and end with a "what exists today" grounding.
  Designed behaviour is never written as if it works today, anywhere in
  the book.
- Contract surface is not behaviour. The versioned query-parameter
  contract carries fields for a query planner, spell correction, and
  rewritten query variants; setting any of them is a validation error
  today ("not implemented yet"), and this book says so at the flag, not
  just in a design chapter.
- Storage layers are not integrations. The Worker's D1 hot cache exists
  as schema and store primitives with tests; with no caller in the
  request path, this book describes it as exactly that.
- Ingest has two generations and the book labels them: the canonical
  script pipeline (collect, normalise, index, publish) is production
  today and is still substantially manual and script-driven, while the
  staging/registry/gate machinery in `atlas-core` is the built foundation
  the automatic-ingest design builds on. Neither is described as the
  other.
- Where help text and source disagree — and they do, in named places,
  from a hidden experimental flag to a fleet help paragraph that calls a
  subset surface a mirror — the book documents the source behaviour and
  calls out the discrepancy until the help is fixed.

That discipline is also the book's maintenance rule: a change to a
command, flag, default, surface, or procedure updates the matching
chapter in the same change, and a designed spelling is never promoted to
a shipped claim without the code to back it. [Architecture](../concepts/architecture.md)
takes this shape and fills in the component detail.
