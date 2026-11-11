# Atlas memory integration — design record (#8567)

Status: design record for the #1300 Phase-2 decision. Decided by Drew
(pending-questions ledger `fleet-vector-store-1300`, corrected
2026-11-10 18:18 CST, recorded on issue #1300): **integrate fleet/agent
memory into Atlas** — everything queryable across all agents and
corpora. Not local-only silos, not a separate new store.

Author: queue task 14895 (lane any-c), 2026-11-11. Parent: #1300
(closed, decision recorded). This record: #8567 acceptance item 1.

## 1. What is indexed

One unified Atlas index over the fleet's own corpora. Each corpus
keeps its existing owner and source of truth; Atlas holds the unified
search index (chunks + vectors + metadata), never a fork of the
content.

| Corpus | Source of truth today | Content indexed |
|---|---|---|
| Agent memory (OpenClaw) | memory-lancedb store behind `plugins.slots.memory` (Phase 1, #1300/#6167) | Memory entries the gateway writes/recalls |
| Fleet memory files | The Muse memory tree (`MEMORY.md`, topic files, daily logs) | Memory notes, decisions, standing facts |
| Zettelkasten / Emanote notes | The notes vault | Notes, links, backlinks context |
| Repo docs | The four fleet repos' doc trees (`doc/`, `docs/`, package docs) | Design records, runbooks, READMEs |
| Research reports | Prior fleet research outputs (goal files, briefs) | Reports and briefings |

Later corpora (post-#6052, see §5) join by the same connector
contract; the corpus set above is the complete pre-ratification scope.

Chunking and metadata: every indexed unit carries `corpus`, `agent`
(originating agent/session family where known), `source_ref` (path or
URL), `updated_at`, `sensitivity` (the caller's ceiling vocabulary
atlas-core already enforces), and `zone` (§5). Embeddings are produced
by one pipeline (§2) so vectors are comparable across corpora — a
cross-corpus ranking over incomparable embeddings is noise, and the
eval harness (§6, slice 5) is the oracle that proves comparability.

## 2. Where the index lives inside Atlas

Inside Atlas, as a set of named corpora in the existing atlas-core
store — the same libsql/Turso-backed storage the search service
already runs (atlas-core `LocalBackend`/`TursoBackend`), reached
through the existing stage/registry ingestion path
(`schema/atlas_stage_v1.sql`, `schema/stage_registry_v1.sql`):

- Ingestion is a connector per corpus (§1) that stages chunks into
  the stage tables; promotion into the live corpus follows the
  existing registry discipline (provenance, evidence, rollback) —
  memory content gets the same assurance path as every other Atlas
  corpus, not a side door.
- Embeddings come from the one stable endpoint Phase 1 established
  (the openclaw-embedding service, fixed loopback port 18791) or
  atlas-embed where a process is Atlas-internal; the embedding model
  identity is recorded per corpus so a model change is an explicit
  re-embed slice, never silent drift.
- No new vector store is created. LanceDB remains OpenClaw's local
  store (its Phase-1 role); the Atlas index is built from it by the
  connector like any other corpus. If the LanceDB store and the
  Atlas index disagree, the source of truth wins and the index is
  rebuilt — the index is always disposable, the corpora never are.

## 3. The cross-agent query surface

One surface: atlas-core `run_search` over the union of corpora, with
the caller's sensitivity ceiling and corpus filters applied **inside
the branch queries** (the mechanism atlas-core already implements —
filters before ranking, never after), and reciprocal-rank fusion
across the per-corpus branches under the versioned query contract
(atlas-proto `query-parameters-v1.json`).

Every agent family — OpenClaw, OpenCode sessions, OpenChamber, Muse
lanes — queries the same surface with its own identity and ceiling.
A query names corpora (default: all the caller may see) and gets one
fused, provenance-carrying result list: each hit names its corpus and
`source_ref`, so an agent can always open the source, not just the
chunk. There is no per-agent index and no agent-specific ranking
path; differences between agents are identity, ceiling, and corpus
selection only.

## 4. Access-pattern decision: MCP first

**Decision: the query surface is exposed as an MCP server (an mcpx
namespace), and that is the only agent-facing access pattern. Native
library access is reserved for Atlas-internal processes (ingestion,
the eval harness, the search service itself).**

Reasons:

1. **It is the one protocol every fleet agent already speaks.** All
   four agent families reach tools through mcpx today. An MCP
   namespace is one integration, built once; native per-tool access
   is four integrations (four SDK surfaces, four upgrade cadences)
   delivering the same queries.
2. **One policy enforcement point.** Sensitivity ceilings, corpus
   scoping, and audit are enforced in the MCP layer in front of
   `run_search`, once. Native access per tool would re-implement —
   or, worse, skip — that enforcement in each consumer, and the
   ceiling-inside-the-branch-query guarantee is exactly the kind of
   invariant that dies by re-implementation.
3. **Identity travels with the protocol.** The MCP layer carries the
   calling agent's identity into the query (agent, ceiling, corpora),
   which the audit trail and the fused-provenance results both need;
   a native library call inside an agent's process has no natural
   place to stamp that.
4. **Atlas stays a service, not a library fork.** Consumers pin a
   protocol contract (atlas-proto's versioned query parameters), not
   a crate version; Atlas internals (backends, staging, embedding
   model) can change without touching any agent.

Rejected alternative: per-tool native access (each agent embedding
atlas-core or a client SDK). It optimizes latency we have not
measured a need for, at the cost of reasons 1–3. If a measured
latency case appears, it is revisited as its own design record
against this one — not by quietly growing a second surface.

## 5. Composition with the #6052 zone policy

Issue #6052 (external-content zone policy) is deferred by Drew behind
the #6066 networking epic, and bulk external-content ingestion
additionally gates on the #6024 licensing decision. This design
composes with that track as follows:

- **Before ratification (now):** the in-scope corpora are exactly the
  fleet/own corpora of §1 — agent memory, fleet memory files, the
  notes vault, the fleet repos' docs, fleet research reports. No
  bulk external content is ingested into any corpus this integration
  exposes. Nothing in this design's implementation slices touches
  external corpora.
- **After ratification:** external corpora join as additional
  corpora under the zone policy's rules (zone labels, licensing
  status, promotion discipline) through the same connector contract
  and stage/registry path. No redesign is needed: every indexed unit
  already carries a `zone` field — `fleet` for everything in §1 —
  and post-ratification corpora carry their zone from their first
  staged chunk. Query-time zone filtering rides the same
  inside-the-branch-query filter mechanism as sensitivity.
- The gate is therefore a data/policy gate, not an architecture
  gate: ratification changes what connectors may stage, never how
  querying works.

## 6. Implementation slices (filed as their own issues, linked from #8567)

1. **Corpus registry + memory connector:** register the §1 corpora
   in the stage registry; build the memory-lancedb → Atlas staging
   connector (agent memory corpus live end-to-end).
2. **Embedding unification:** route all corpus ingestion through the
   one embedding pipeline (port-18791 endpoint / atlas-embed);
   record embedding-model identity per corpus; re-embed job for
   existing staged content.
3. **MCP query surface:** the mcpx namespace in front of
   `run_search` — identity, sensitivity ceiling, corpus scoping,
   audit — per §4.
4. **Remaining corpus connectors:** fleet memory files, zettelkasten
   notes, repo docs, research reports (§1), each with provenance and
   incremental re-index on source change.
5. **Cross-agent eval:** extend the atlas eval harness (graded
   qrels, nDCG, recall — the #5741 harness and its successors) with
   cross-corpus query sets per agent family; a corpus or connector
   change ships only with eval evidence attached.

Slices 1–3 are the critical path (one corpus, one pipeline, one
surface); 4 widens coverage; 5 is the standing oracle. #6052-gated
work appears in no slice.
