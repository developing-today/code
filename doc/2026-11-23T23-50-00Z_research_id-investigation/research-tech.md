# ID Project — Distributed-Systems & Platform Technology Survey

Research for the ID program-planning prompt. Compiled 2026-11-23 (America/Chicago)
from public sources via web search / page fetch. Claims marked **[unverified]**
were not confirmed against a primary source in this pass and should be re-checked
before being baked into a design.

**ID, for context in every "Relevance" line:** a distributed, federated,
capability-based, content-addressed graph platform. Components named in the
brief: CAS (content-addressed storage), signing, iroh transport, capability
security (id-caps in the existing codebase), CRDTs, an ECS for simulations
(Bevy slices already landing in `pkgs/id`), embeddings/search (the Atlas
project), and MCP exposure of parts as libraries/tools in other apps.

---

## 1. OCapN — Object Capability Network (Spritely Institute)

**What it is.** A suite of protocols for secure distributed object programming
where *holding a reference is the authority*: "if you don't have it, you can't
use it." No ambient authority, no ACL check at call time — capability security
as ordinary reference-passing. The stack, per the spec repo, is four documents:

- **CapTP** — Capability Transport Protocol: message passing between distributed
  objects, promise pipelining (send to a promise's eventual result), distributed
  garbage collection (`op:gc-export` / `op:gc-answer`), third-party handoffs.
- **Data model / Syrup** — canonical serialization for passable values.
- **Locators** — `ocapn://` peer URIs and *sturdyrefs* (durable, unguessable
  object locators with a "swiss number").
- **Netlayers** — transport abstraction (Tor onion services, TCP/TLS, etc.).

Lineage: Mark Miller's E language → CapTP → Waterken, Cap'n Proto RPC,
Agoric SwingSet/Endo, now Goblins.

**Maturity.** **Pre-standardization — explicitly not yet a standard and subject
to change.** The Goblins manual itself warns CapTP support "is still in its
early days; do not rely on the protocol being stable." The pre-standardization
group is consensus-based, driven primarily by Spritely, with Agoric and
Cap'n Proto as regular contributors and MetaMask in an advisory role
[unverified beyond the group's own docs]. Design whitepaper: *The Heart of
Spritely* (Lemmer-Webber, Farmer, Sims, May 2025) lays out Goblins' vat/turn
model (vats = containers, turns = cheap atomic local transactions), promise
pipelining, and time-travel debugging.

**Implementations / libraries.**

- **Spritely Goblins** — the reference implementation, in Guile Scheme and
  Racket Scheme. Guile↔Racket OCapN interop was demonstrated under the NLnet
  grant (2023). No production Rust implementation was found in this pass
  **[unverified — worth a dedicated check; a Rust CapTP would be a build, not
  an adoption]**.
- **Endo / `@endo/captp`** (Agoric ecosystem, JavaScript) — CapTP over
  SmallCaps/JSON encoding rather than Syrup.
- **Cap'n Proto RPC** — a cousin, not OCapN itself: capability RPC with its own
  wire format, very performance-oriented, Rust crate `capnp` exists.

**vs. a hand-rolled capability system.** OCapN buys: a shared wire protocol
(promise pipelining, distributed GC, certified third-party handoffs are *hard*
to get right by hand), cross-language interop ambitions, and a security model
reviewed by the E/Agoric community for decades. It costs: spec instability,
Scheme-first implementations, and a programming model (vats/turns) that does
not map 1:1 onto a Rust event-sourced graph. A hand-rolled system (signed
capability tokens / macaroon-style attenuation over your own transport) keeps
full control and Rust-native ergonomics but must re-derive handoffs,
revocation, and GC — and gets no interop.

**Canonical links.**

- https://ocapn.org/ — project home (spec group)
- Spec repo: `github.com/ocapn/ocapn` — `draft-specifications/` (CapTP, Model,
  Locators, Notation)
- https://spritely.institute/news/introducing-ocapn-interoperable-capabilities-over-the-network.html
- https://files.spritely.institute/docs/guile-goblins/0.13.0/OCapN.html — Goblins manual, OCapN chapter
- https://lwn.net/Articles/962303/ — LWN overview of Spritely/Goblins/OCapN

**Relevance to ID:** OCapN is the closest published design to ID's capability
layer — study CapTP's handoff/GC/pipelining semantics and sturdyref locators
even if ID keeps its own Rust implementation; adopting the wire protocol itself
is a bet on a pre-standard spec with no Rust reference implementation.

---

## 2. CRDTs — Loro, Yrs, Automerge

**What they are.** Conflict-free Replicated Data Types: replicas edited
independently converge without coordination. Two families matter here:
*operation/event-graph* designs (history is a DAG of operations; state is a
fold) and *state/delta* sync (exchange state vectors + diffs). All three below
are Rust-core libraries.

| | Loro | Yrs | Automerge |
|---|---|---|---|
| Core | Rust, WASM/JS + Swift bindings | Rust port of Yjs (JS) | Rust core, JS-first API |
| Text algorithm | **Fugue** (maximal non-interleaving) + Eg-walker-inspired event graph | YATA | RGA + Peritext-style marks |
| Types | Text, Rich Text, List, **MovableList**, Map (LWW), **MovableTree**, Counter | YText, YArray, YMap, YXml, etc. (Yjs shared types) | JSON-like: maps, lists, text, counters |
| History | Full event DAG; fork / merge / time-travel; shallow snapshots | Update-based; history is not a first-class DAG | Full change DAG; git-like fork/merge is the headline feature |
| Sync model | Export/import updates or snapshots (`ExportMode`) | State-vector diff protocol, byte-compatible with Yjs | Change sync protocol; sync or manual change transfer |
| Maturity | **1.0 released 2024** (stable data format); actively developed | Stable; inherits Yjs's huge deployed base | Mature; **3.0 (Aug 2025)** columnar format cut memory ~10–100× vs 2.x **[per project/research notes; verify against release notes]** |

**Fit notes for an event-sourced graph with signed events.**

- **Loro** is architecturally closest to "event-sourced": its Replayable Event
  Graph *is* a history DAG, and it uniquely offers a movable-tree CRDT
  (Kleppmann et al.) for hierarchies. But its event graph is internal — not
  exposed as a content-addressed (e.g. BLAKE3-linked) DAG, and it has **no
  general graph type** (edges must be modelled as Map-of-Maps by hand). Signing
  is not built in; you'd sign at your own event layer.
- **Yrs** wins on ecosystem gravity: Yjs is the default collaborative-editing
  stack on the web (~920k weekly npm downloads per PkgPulse 2026
  **[unverified, secondary source]**), with editor bindings (ProseMirror,
  CodeMirror, Monaco) that work against the same wire format. Best when ID
  needs collaborative *text/document* editing surfaces rather than graph merge.
- **Automerge** wins when *versioning is the product*: per-change attribution,
  branching/merging, diffs between versions. Its model is document-centric
  JSON, not an arbitrary property graph.
- None of the three signs operations natively or enforces capability checks —
  in all cases ID's signed-event envelope and cap checks would wrap the CRDT,
  and the CRDT would handle convergence of the *content* layer only.

**Canonical links.**

- https://loro.dev/blog/v1.0 — Loro 1.0 announcement (Fugue, Eg-walker, movable tree)
- https://github.com/loro-dev/loro
- https://github.com/y-crdt/y-crdt — Yrs (and sibling ports)
- https://automerge.org/ and https://github.com/automerge/automerge

**Relevance to ID:** Use a CRDT for collaborative content (text, lists, trees)
inside ID objects; do not expect any of them to replace ID's signed event log
or to model the general graph — convergence of signed graph events still needs
ID's own fold/merge rules (or an iroh-docs-style reconciliation, §3).

---

## 3. iroh — blobs, gossip, docs

**What it is.** A Rust peer-to-peer networking stack by n0 (n0-computer):
dial peers **by public key (NodeId), not IP**, over QUIC with NAT traversal
(hole-punching with relay fallback) built in. Deliberately much smaller surface
than libp2p. **iroh 1.0 was released 2026-06-15** after ~4 years / 65
pre-releases, per a detailed third-party deep-dive **[date/version from
secondary source; the ID codebase already builds on iroh, so the pinned
version in `pkgs/id` is the ground truth]**. Since ~0.90 the companion
protocols live in separate repos/crates from core transport.

**The three companion protocols.**

- **iroh-blobs** — content-addressed transfer: BLAKE3 hashing, verified
  streaming (hash checked as bytes arrive), resumable/range requests, blob and
  collection ("hashseq") storage. This is a CAS transfer + store layer.
- **iroh-gossip** — topic-based broadcast over epidemic broadcast trees
  (Plumtree / HyParView-style). Broadcast-only: no request/response, no
  catch-up for late joiners — peers learn *new* things live; old state must
  come from elsewhere (docs reconciliation or blob fetch).
- **iroh-docs** — a multi-dimensional key-value store: a *Document/Replica* is
  identified by a `NamespaceId` (public key gating write capability); entries
  are `(namespace, author, key) → BLAKE3 hash + size + timestamp`, signed by
  both the namespace key (write capability) and an author key; actual bytes
  live in iroh-blobs. Sync is **range-based set reconciliation** (Meyer 2022):
  peers recursively compare range fingerprints and exchange only differing
  entries — in-sync peers confirm with a single fingerprint. Conflict rule for
  same `(author, key)`: last-write-wins by timestamp, hash tiebreak. Storage
  backend is `redb` (single file). Sharing is by `DocTicket` strings.

**Maturity / Rust API.** Pure Rust, Apache-2.0/MIT. Core reached 1.0 (see
caveat above); blobs/gossip/docs version independently and have had breaking
API changes across the 0.x line — pin and expect upgrade work. The pattern of
composing Endpoint + Router + the three protocol handlers is stable and
documented with runnable examples (incl. a Tauri todo app using docs
end-to-end).

**Canonical links.**

- https://iroh.computer/ and https://docs.iroh.computer/
- https://docs.iroh.computer/protocols/documents — iroh-docs concepts (verified by direct fetch 2026-11-23)
- Repos: `github.com/n0-computer/iroh`, `…/iroh-blobs`, `…/iroh-gossip`, `…/iroh-docs`
- Range-based set reconciliation paper: https://arxiv.org/abs/2212.13567

**Relevance to ID:** This is already ID's transport/CAS/sync substrate —
blobs = CAS bytes, docs = a working signed-entry + reconciliation reference
design (and its LWW limits show exactly where ID's richer graph merge must go
beyond it), gossip = live notification only, never the source of truth.

---

## 4. Bevy ECS

**What it is.** Bevy is a Rust game engine; **bevy_ecs** is its Entity
Component System crate: entities are IDs, components are plain Rust structs,
systems are plain Rust functions whose parameters declare their data access,
plus Resources (singletons), Schedules (ordered/parallel system sets), Events,
and Queries with filters (`With`/`Without`/`Changed`). It is explicitly
designed to be usable **as a standalone crate in other projects** (docs.rs
wording), without the renderer/windowing stack.

**Using it headless / as a library.**

- Depend on `bevy_ecs` alone, or on `bevy` with `default-features = false` +
  `MinimalPlugins` and a `ScheduleRunnerPlugin` loop for a server tick —
  a pattern in production use for authoritative game servers (ECS systems run
  the simulation; networking stays outside).
- Strengths for a simulation platform: massively parallel system scheduling,
  cache-friendly archetype storage, deterministic system ordering controls,
  change detection — good bones for Dwarf-Fortress/Space-Invaders-class 2D
  sims and larger ECS simulations, with rendering as just another system set
  that pure/server builds omit.
- Costs: Bevy's release cadence is fast and breaking (0.15→0.16→… API churn is
  normal); pinning bevy_ecs means scheduled upgrade work. ECS also imposes a
  data layout discipline — arbitrary graph traversal is *not* its home turf;
  keep the graph store as the source of truth and project into ECS for
  simulation ticks.

**Canonical links.**

- https://docs.rs/bevy_ecs — crate docs ("can easily be used as a standalone crate")
- https://bevyengine.org/ and https://github.com/bevyengine/bevy

**Relevance to ID:** bevy_ecs is the right shape for ID's simulation layer
(server-authoritative and client-pure builds from the same systems) — the
existing `pkgs/id` Bevy render slices (scene/audio/widget) already bet on it;
the ECS should consume projections of the graph, never own it.

---

## 5. Embeddings & vector search

### 5a. Index / storage engines

| Option | What it is | Maturity / shape | Notes |
|---|---|---|---|
| **sqlite-vec** | SQLite extension (pure C, zero deps), `vec0` virtual tables | v0.1.x; stable line is **brute-force exact KNN**; IVF/DiskANN only in alpha **[per 2026 surveys; verify]** | Single-file with your SQLite DB; metadata filtering is plain SQL joins. Practical ceiling ~100k vectors for brute force **[secondary-source estimate]**. Cannot be loaded by remote sqld/Turso servers (no dynamic extensions there). |
| **libSQL / Turso native vectors** | `F32_BLOB` columns + `vector_distance_cos`, optional `vector_top_k` DiskANN index | Built into the libSQL engine; works identically local file / self-hosted sqld / Turso Cloud; embedded replicas supported | Same code path local↔remote is the selling point vs sqlite-vec. The newer Rust rewrite ("Turso Database"/Limbo) is pre-1.0/beta — do not confuse the two. |
| **usearch** (Unum) | Single-header C++ HNSW library with Rust bindings (`cargo add usearch`) | Mature library, *not* a database: no metadata store, persistence is index save/load you manage | HNSW approximate search, many metrics (cosine, L2, Hamming, Jaccard, Haversine), f32/f16/i8 quantization. For when you outgrow brute force and want an in-process ANN index. |
| **LanceDB** | Embedded vector DB on the Lance columnar format, Rust core | Apache-2.0; IVF-PQ / HNSW indexes, MVCC versioning, native Tantivy BM25 + vector **hybrid search** with reranking | Heavier than sqlite-vec; directory of files, not single-file. Strong when multimodal/large and hybrid lexical+semantic ranking matters. |

### 5b. Local embedding models (Rust)

- **fastembed** (`fastembed-rs`) — the standard Rust answer: ONNX Runtime
  (`ort`) inference, synchronous API, large model catalogue out of the box —
  BGE small/base/large, all-MiniLM-L6-v2, multilingual-e5, Nomic Embed,
  BGE-M3 (dense+sparse+ColBERT), CLIP image embeddings, SPLADE sparse, and
  cross-encoder rerankers (bge-reranker). Newer models (Qwen3-Embedding,
  Nomic v2 MoE) run on its **candle** backend behind feature flags. Models
  download to a local cache on first use; user-defined local model files
  supported — important for offline/air-gapped ID nodes.
- **candle** (Hugging Face's Rust ML framework) — the lower-level route:
  run BERT-family/Qwen/etc. transformers natively in Rust (CPU/CUDA/Metal)
  when you need custom architectures or want no ONNX runtime dependency.
  More work: tokenization, pooling, normalization are yours.

### 5c. How a search index over a content-addressed graph is usually structured

The recurring pattern (Atlas-shaped): the **graph/CAS stays canonical**; the
vector index is a **derived, rebuildable projection** —

1. Each indexable object contributes one or more *chunks* (text spans,
   property serializations, captions) keyed by the object's content hash.
2. A row/record per chunk: `(content_hash, chunk_id, model_id, dimensions,
   embedding, metadata…)` — `model_id` is mandatory; embeddings from different
   models are incomparable, and a model change means a re-embed pass.
3. Query path: embed the query with the same model → ANN/exact KNN → map chunk
   hits back to graph objects → expand/traverse the graph from those entry
   points → optionally rerank (cross-encoder) and fuse with lexical (FTS5 /
   BM25) results via reciprocal-rank fusion.
4. Because the index is derived, it can live wherever is convenient
   (sqlite-vec beside the metadata DB at small scale; usearch/LanceDB at
   larger) and be dropped/rebuilt from CAS without data loss; sync between
   peers ships *content*, and each peer builds its own index — embeddings
   themselves need not federate.

**Canonical links.**

- https://github.com/asg017/sqlite-vec
- libSQL vector docs via https://turso.tech/ / https://github.com/tursodatabase/libsql
- https://github.com/unum-cloud/usearch
- https://lancedb.com/ / https://github.com/lancedb/lancedb
- https://github.com/Anush008/fastembed-rs and https://docs.rs/fastembed
- https://github.com/huggingface/candle

**Relevance to ID:** Bake Atlas in as a derived index over the CAS graph —
fastembed for local embedding, sqlite-vec (or libSQL native vectors, matching
ID's existing Turso use) first, usearch/LanceDB only when corpus size forces
ANN; never let the index become a second source of truth.

---

## 6. Graph visualization, search UI, and structured-data representations

### Visualization / graph data layer (web)

- **Cytoscape.js** (MIT, Canvas 2D) — the analysis-first choice: rich built-in
  layouts and graph algorithms (BFS, PageRank, centrality), CSS-like
  stylesheet styling, compound nodes, headless use for pure analysis.
  Interactive ceiling commonly cited ~5k elements **[secondary-source
  estimate; benchmark with real data]**.
- **Sigma.js v3 + Graphology** (MIT, WebGL) — the scale-first choice:
  Graphology is the in-memory graph data structure + algorithms library
  (Louvain, ForceAtlas2 layout, metrics); Sigma is only the renderer.
  Handles tens of thousands of nodes; no compound nodes; custom node shapes
  need shader work.
- Rule of thumb from 2026 comparisons: Cytoscape when the graph *is* an
  analysis object; Sigma+Graphology when it's a large network to explore;
  React Flow when it's an editable node diagram (~1k DOM-node ceiling).

### Representations for text/property objects

- **Property graph (LPG)** — nodes and edges both carry key/value properties
  and labels. The pragmatic developer model (Neo4j lineage); maps naturally to
  "object with properties + typed edges." No global standard; every store's
  dialect differs.
- **RDF** — everything is subject–predicate–object triples with IRI identity;
  the interoperable/semantic model, with formal entailment and a big standards
  stack (SPARQL, OWL). More ceremony; properties of edges need reification
  workarounds (or RDF-star).
- **JSON-LD** — a JSON serialization *of* RDF: ordinary JSON plus an `@context`
  mapping keys to IRIs. W3C Recommendation (1.1, 2020). Lets an API speak plain
  JSON to app code and linked data to machines from the same payload.
- Practical stance for ID: keep the native model a signed property graph
  (objects + typed, propertied edges — matches the existing event design), and
  offer **JSON-LD as an export/interchange view** where federation with
  outside tools matters, rather than adopting RDF as the internal model.

**Canonical links.**

- https://js.cytoscape.org/ — Cytoscape.js
- https://www.sigmajs.org/ and https://graphology.github.io/
- https://json-ld.org/ and the W3C JSON-LD 1.1 Recommendation

**Relevance to ID:** Sigma+Graphology for the big-graph explorer, Cytoscape
for analysis views; property-graph internally, JSON-LD at the boundary.

---

## 7. MCP — Model Context Protocol

**What it is.** An open protocol (introduced by Anthropic, Nov 2024) for
connecting AI applications to external tools/data — "USB-C for AI apps."
JSON-RPC 2.0; hosts contain one client per server; servers expose three
primitives: **Tools** (model-callable functions with JSON-Schema inputs),
**Resources** (application-read data, URI-identified), **Prompts**
(user-picked templates). Client-side features: sampling, elicitation, roots.
Transports: stdio (local subprocess) and Streamable HTTP (remote; the older
HTTP+SSE transport was deprecated in the 2025-03 revision).

**Spec state.** Versioned by date. 2025-06-18 added OAuth 2.1 authorization,
elicitation, structured tool outputs. A **2025-11-25** revision is the latest
referenced in current guides **[verify against modelcontextprotocol.io before
citing in a design]**. In Dec 2025 Anthropic donated MCP to the Agentic AI
Foundation under the Linux Foundation **[per secondary reporting; verify]**.
Adoption: Claude, ChatGPT, Gemini, Cursor, VS Code, and many others; official
SDKs for TypeScript and Python, with Rust SDKs community-led (e.g. `rmcp`)
**[Rust SDK status unverified in this pass]**.

**Using a system's parts as MCP tools.** The pattern ID cares about: any
library-grade component (CAS lookup, graph query, search, capability-checked
mutation) can be wrapped once as an MCP server and then consumed by *any* MCP
host — AI assistants, IDEs, other apps — without bespoke integrations per
consumer. The discipline it imposes is useful even without AI: small, named,
schema-described operations with explicit read/write character. The security
note that matters for ID: MCP's own auth story is young (OAuth 2.1 for remote
servers) and tool hints are explicitly untrusted — **MCP must sit behind ID's
capability layer, not replace it**: an MCP tool call should arrive *holding*
a capability, and ID checks it exactly as for any other caller.

**Canonical links.**

- https://modelcontextprotocol.io/ — spec + docs
- https://github.com/modelcontextprotocol — SDKs and server registry

**Relevance to ID:** MCP is the cheapest "every part usable from other apps"
story available — expose ID components as MCP tools/resources *through* the
capability system, so agents and external apps get least-authority access by
construction.

---

## 8. Falcor vs GraphQL vs path-based graph query

**Falcor** (Netflix, open-sourced 2015) models *all* remote data as one
virtual **JSON Graph**. Clients fetch by **paths** — `["users", 0, "name"]`,
ranges like `users[0..9]["name","email"]` — with `get`/`set`/`call` operations;
references inside the graph are traversed automatically; a client-side Model
cache batches, dedupes, and normalizes. There is no schema and no type system.
**Status: effectively dormant** — v2.x, no significant releases in years;
Netflix-internal legacy. Its *ideas* (single virtual graph, path addressing,
cache normalization) are the durable part.

**GraphQL** (Meta, 2015) is the opposite trade: a strongly-typed schema,
a declarative query language where the response mirrors the query shape,
introspection/codegen, and **GraphQL Federation** for composing graphs across
services — actively evolving and the industry default for client-driven
fetching.

**For a federated graph like ID's, the comparison that matters:**

- Path-based (Falcor-style) addressing fits content-addressed objects
  unusually well: a path through named references *is* how you navigate a
  graph of signed objects, paths compose, cache keys are obvious, and no
  global schema must be agreed before federation can start. Its weakness is
  Falcor's own: no types, weak ad-hoc filtering/search.
- GraphQL fits when consumers are heterogeneous apps needing typed contracts
  and field-level selection — at the cost of a schema governance process that
  a federated, multi-owner graph makes genuinely hard.
- The synthesis most such systems land on: **path/reference navigation as the
  native query primitive** (plus ID's own traversal/search), with a typed
  query surface (GraphQL or similar) offered as a *view* for app developers
  where a stable schema exists. Do not adopt Falcor the library; adopt its
  model consciously.

**Canonical links.**

- https://netflix.github.io/falcor/ — Falcor docs (JSON Graph model)
- https://graphql.org/ and https://spec.graphql.org/ — GraphQL spec

**Relevance to ID:** Steal Falcor's virtual-graph + path model for native
graph navigation; skip the dormant library; add typed querying as a layer,
not the foundation.

---

## 9. Token / market mechanics — fuel & credit accounting tied to capabilities

Brief, factual references only; no price speculation.

- **Agoric — ERTP + Zoe.** The most developed *capability-native* asset
  design in production: ERTP (Electronic Rights Transfer Protocol) represents
  fungible and non-fungible assets as objects — **Mint** (sole creation
  authority), **Issuer**, **Brand** (asset-type identity), **Purse**,
  **Payment** — with conservation of supply enforced by the framework, not by
  contract code. Zoe is the contract framework on top: it escrows offer assets
  and guarantees *offer safety* (a participant gets what they asked for or
  their assets back, even from buggy contracts). Contracts are metered;
  Agoric's chain charges execution fees in its stable token (IST), staking in
  BLD. Audited by Least Authority (2021). Links:
  https://agoric.com/whitepaper · ERTP/Zoe under
  https://github.com/Agoric/agoric-sdk
- **Metering as the primitive beneath any "fuel" token: Wasmtime fuel.**
  Wasmtime's `consume_fuel` instruments compiled Wasm so each operation costs
  fuel from a per-`Store` budget; exhaustion traps deterministically
  (`OutOfFuel`), and consumed fuel is readable afterwards — the standard
  pattern for deterministic, cross-node-identical resource accounting and
  per-invocation billing in Wasm plugin/contract systems (WasmEdge's
  `--gas-limit` is the analogue; Ethereum's gas is the same idea at chain
  scale). Overhead is real (commonly cited ~5–15% **[secondary-source
  estimate]**); epoch interruption is the cheaper wall-clock alternative when
  determinism is not required. Link:
  https://docs.rs/wasmtime — `Config::consume_fuel`.
- **Spritely / OCapN world.** Capability systems in this lineage typically
  treat *attenuated, revocable* rights (a purse-like object that can only
  spend up to X, a caretaker that can be revoked) as the accounting unit
  rather than a transferable token — i.e., "fuel" as a capability budget
  object. No single canonical published design was verified in this pass
  **[unverified — candidate topic for the design phase, cf. ERTP purses and
  the "caretaker" pattern in the E literature]**.

**Design shape these references converge on for ID:** (1) execution is
*metered deterministically* (fuel) at the runtime layer; (2) fuel is *paid
from an asset object* governed by conservation rules (ERTP-style
mint/issuer/purse); (3) access to the purse is itself a *capability* —
attenuable (per-app, per-session budgets) and revocable — so in-app items,
app fuel, and market trades are one mechanism, not three.

**Relevance to ID:** ERTP's mint/issuer/purse vocabulary + Wasmtime-style
fuel metering is the concrete, prior-art-backed route to tying tokens to app
fuel and in-app items inside a capability system.

---

## Cross-cutting takeaways for the planning prompt

1. **Capability layer:** OCapN/CapTP is the reference design (handoffs, GC,
   pipelining, sturdyrefs); a Rust implementation is a build decision, and
   ERTP shows how assets live inside a capability world.
2. **Convergence has three distinct layers — don't conflate them:** iroh-docs
   range reconciliation (entry sets), a CRDT (document content), and ID's own
   signed-event fold (graph semantics). Each layer has a different winner.
3. **Search is a projection:** embeddings (fastembed locally) + a rebuildable
   index (sqlite-vec / libSQL vectors → usearch / LanceDB at scale) over the
   CAS graph; the graph remains the only source of truth.
4. **Simulation is a projection too:** bevy_ecs consumes graph state per tick;
   server and client run the same systems with different plugin sets.
5. **Exposure surfaces:** MCP for tools/other apps (behind capabilities),
   Sigma/Cytoscape for graph UI, JSON-LD at the interchange boundary,
   Falcor-style paths as the native navigation model.
6. **Fuel:** deterministic metering (Wasmtime fuel pattern) + conserved asset
   objects (ERTP pattern) + capability-held budgets.
