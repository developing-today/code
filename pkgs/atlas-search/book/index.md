# atlas

Atlas is the user's unified search system: one search engine over the corpora the user
and his agents produce — AI-client conversations first, and, through the same
staging discipline, notes, repository documents, GitHub comments, query traces,
and reports. Every stored unit keeps its _provenance_ (where it came from: the
session, client, source reference, and commit or observation date it was taken
from), every unit carries a _sensitivity_ classification, and retrieval is
_hybrid_: a full-text branch and a vector branch run over the same filtered
candidate set and their rankings are fused into one result list. The engine is
a Rust library, `atlas-core`; everything else in this book is a surface over
it.

Source: https://github.com/developing-today/code/blob/main/pkgs/atlas-search/Cargo.toml

## What is in the corpus

The corpus model is three levels: _sessions_ (one conversation or source
unit) contain _messages_, and messages are split into _chunks_ — the
searchable unit, roughly 3,500 characters each, deduplicated by content hash
so re-ingesting the same material adds nothing twice. Each session is
classified with a kind, topics, and projects, and each chunk carries an
effective sensitivity (its own, falling back to its session's label).

On **2026-11-23** the canonical laptop database contained **18,325 sessions,
122,115 messages and 107,439 chunks**. A separate read of the October 31
publish-source file returned 18,325 sessions, 122,124 messages and 107,639
chunks. Each count belongs to the selected local file; the live service and
Worker require separate publication receipts.
[Backup, restore and the census](operations/backup-and-census.md).

Two properties define the system, and both are enforced before ranking, never
after:

- **Provenance first.** A search hit is never bare text. Every result names
  its chunk, its session, the client that produced the session, and its
  classification, and a follow-up fetch returns one chunk with that same
  provenance attached. A result that cannot say where it came from is not a
  result Atlas serves.
- **Tiered sensitivity.** Sensitivity is a four-step ladder —
  canonically `public < internal < private < sensitive`, spelled in code
  today `shareable < personal < local-only < secret-bearing`. A caller searches
  under a _ceiling_ — the highest sensitivity it may see — and the ceiling is
  applied inside the branch queries, so material above the ceiling is absent
  from the results, not merely down-ranked. The default ceiling is
  `personal`; seeing `secret-bearing` material requires spelling that ceiling
  out, and a service can clamp a caller's ceiling down to the cap recorded
  for its client identity, never up. See
  [Sensitivity tiers and client ceilings](concepts/sensitivity.md).

Retrieval itself is one-stage hybrid search. The full-text branch is an FTS5
(BM25) match; the vector branch compares the query's embedding against chunk
embeddings by cosine distance; the two ranked pools — 100 candidates per
branch by default — are fused with reciprocal rank fusion (RRF constant 10)
into a final list of 10 by default. All chunks are embedded with one model,
Snowflake `snowflake-arctic-embed-m-v2.0` at the full 768 dimensions, because
vectors from different embedding models are not comparable. See
[Retrieval: hybrid full-text and vector search](concepts/retrieval.md) and
[Embeddings: Snowflake Arctic and the parity contract](concepts/embeddings.md).

## Why Atlas exists

Before Atlas, each AI client's history lived in that client's own on-disk
store, in that client's own shape. Finding "what did we decide about X last
month" meant knowing which client the conversation happened in and searching
that silo — or not finding it at all. Agents had no shared memory of prior
sessions, and a human had no single place to ask.

Atlas replaces the silos with one normalised store and one query surface:

- **One index over every client.** Conversations from every supported client
  are normalised into the same sessions → messages → chunks shape, so one
  query searches all of them. A filter can still narrow to one client, kind,
  topic, or project, and to a date range over session creation.
- **Search as a service, not as SQL.** Callers use a search command, an HTTP
  job protocol, or an MCP tool. They never write queries against the corpus
  tables, and they cannot widen their own sensitivity ceiling: the ceiling
  travels with the request and is clamped by the surface that serves it.
- **An index that is disposable; corpora that are not.** The searchable store
  is derived from sources that keep their own homes. Embeddings, shards, and
  caches can be rebuilt; the sources of record are the truth. That is what
  makes re-embedding under a better model, or re-sharding the store, a
  mechanical operation instead of a migration of record.

## Who it serves

- **The user, at the command line.** The `atlas-search` binary is the owner's
  tool: standalone search and chunk fetch, the service's provisioning verbs
  (`init`, `fetch-model`), and the operator verbs for staging,
  classification, evaluation, and trace handling that the reference part of
  this book covers. At this surface the disclosure context defaults to the
  owner's, and the ceiling can be raised explicitly to `secret-bearing`.
- **Agents, through fleet and through MCP.** An agent seat normally searches
  through `fleet search`, the fleet binary's search domain, which either
  delegates to the standalone `atlas-search` artifact or talks to the laptop
  service, depending on the seat. Agents connected through the mcpx daemon
  use the Atlas MCP server hosted on the Cloudflare Worker: one tool,
  `search`, under a pinned `personal` ceiling. See
  [Consumers: fleet, agents, memory and ID](concepts/consumers.md) and
  [The Atlas MCP server](reference/mcp.md).
- **The laptop service's named clients.** The service keeps a client table —
  in version 1: the user's CLI (ceiling cap `secret-bearing`), OpenChamber
  sessions (cap `personal`), fork/OpenCode agents (cap `personal`), and
  sandbox scripts (cap `personal`) — plus the Cloudflare Worker as the
  `cf-worker` client (cap `personal`). Each client presents its own token;
  the server clamps every request to that client's cap.
- **ID, as a future consumer.** ID is the user's identity and graph system. An
  Atlas → ID publication format — corpus objects with their provenance and
  lineage relations, published as ID-graph objects — is designed, and the
  search-integration design work continues in this book's design part. ID is
  not a live search client today; where this book describes that
  relationship it is describing a design, and it says so. See
  [Consumers: fleet, agents, memory and ID](concepts/consumers.md).

## The surface map

Atlas is one engine with several surfaces. The map below is the whole system
on one page; [The shape of the system](getting-started/shape.md) explains how
the pieces fit together.

| Surface                         | What it is                                                                                                                                                                                                                                                                                                                                                                             | Where it is covered                                                                                    |
| ------------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------ |
| Engine crates                   | `atlas-core` (search semantics: filters, ceilings, both branches, fusion, the result envelope, the Turso and local backends), `atlas-embed` (the Snowflake embedder over a dynamically loaded ONNX Runtime), `atlas-proto` (the wire contract: job-protocol types and the versioned query-parameter defaults), `atlas-search` (the binary crate: CLI, service, and via-service client) | [The crates](library/crates.md), [atlas-core: the engine as a library](library/atlas-core.md)          |
| `atlas-search` binary and tools | Six installed binaries: `atlas-search` (search, `get-chunk`, `serve`, `init`, `fetch-model`, `traces`, `feedback`), `atlas-stage` (staging units and promotion), `atlas-classify` (the classification sweep), `atlas-eval` (the offline evaluation runner), `atlas-sweep` (the ingestion sweep runner), `atlas-github-comments` (the GitHub-comments extraction feed)                  | [atlas-search: search and get-chunk](reference/atlas-search.md)                                        |
| Laptop service                  | The same search semantics behind a loopback HTTP job protocol on `127.0.0.1:18760`, with one resident embedder shared by every client and per-client ceiling caps. The live unit runs it as `fleet serve search`; `atlas-search serve` is the same service and the kept rollback                                                                                                       | [The service and the Worker](concepts/service-and-worker.md)                                           |
| Cloudflare Worker and MCP       | A Worker at `https://atlas-search.developing-today.workers.dev` exposing gated REST search plus a stateless MCP endpoint with one `search` tool, a per-query Durable Object cache, and no Turso credentials of its own: it calls back to the laptop service for embeddings and fan-out                                                                                                 | [The service and the Worker](concepts/service-and-worker.md), [The Atlas MCP server](reference/mcp.md) |
| `fleet search` client           | The fleet binary's search domain: the agent-facing client surface, with seat-dependent routing between the service and standalone delegation                                                                                                                                                                                                                                           | [fleet search and fleet serve search](reference/fleet-search.md)                                       |
| Python reference                | `atlas_search.py`, the original implementation, kept permanently as the parity oracle and evaluation instrument; it is also the only surface that still offers the dormant bge-small fallback embedder                                                                                                                                                                                 | [atlas_search.py: the Python reference implementation](reference/python-implementation.md)             |

## How to read this book

- **First run:** [Installation and provisioning](getting-started/installation.md),
  then [Quickstart](getting-started/quickstart.md). You will have the
  binaries installed, the model and service provisioned, and will have run
  the same search three ways.
- **Understanding the system:** [The shape of the system](getting-started/shape.md),
  then [Architecture](concepts/architecture.md), then the concept chapter
  for the part you are working in — corpus, sensitivity, embeddings,
  retrieval, ingest, the service, consumers, evaluation.
- **Looking up an exact surface:** the reference part gives, for every
  command and endpoint, its invocation shape, every flag with its default,
  its behaviour notes, a worked example, its failure shapes, and its exit
  codes.
- **Building or operating:** the library part covers the crates as a Rust
  API; the tutorials walk through complete tasks; the development and
  operations parts cover changing Atlas and running it.
- **Design work:** the design part is explicitly design. Those chapters
  describe systems that do not exist yet — automatic ingest, query planning,
  layered search, reranking, the recommender, fleet and ID integration —
  and each one ends by grounding itself in what exists today.

## Conventions used in this book

- **Defaults are stated literally.** When a flag, parameter, or setting has
  a default, the default is given as the value itself — for example "final
  results (default 10)", "pool 100 per branch", "ceiling `personal`".
- **Source wins over help.** Where a binary's help text and its source or
  live behaviour disagree, this book states the source behaviour and says
  that it is doing so. Help text is never silently treated as truth.
- **Secrets are named, never valued.** This book gives secret names (for
  example the Bitwarden name `turso.atlas-ro-token`), token file paths, and
  environment variable names. It never prints a secret value, a client
  token, or corpus content from a restricted tier.
- **Examples are toy examples.** Search examples use invented queries and
  invented results. No real corpus text — from any tier — is reproduced in
  this book.
- **Design is marked as design.** A chapter in the design part, and any
  designed-but-unshipped behaviour mentioned elsewhere, is labelled as a
  design with its tracking issues named. Designed behaviour is never
  written as if it works today.
- **Numbers are dated.** Census counts and other frozen snapshots carry
  their measurement date (the corpus census is 2026-10-31). A number
  without a date in this book is a constant of the code — a default, a
  dimension, a limit — not a measurement.
- **Chapters are self-contained.** Each chapter defines its terms where it
  first uses them and cross-links instead of assuming you have read the
  chapters before it.
