# Design: automatic ingest of all sources

> **Design — not shipped.** This chapter describes a target system that does not exist yet. Nothing in it should be read as working behaviour. Tracking: #9097 Workstream E (automatic ingest, open and blocked), #5670 (migration of the ingest scripts into the code repo, open), #7969 (the recurring sweep runner, unbuilt). The pipeline stages it builds on are tracked separately: ingest-gate promotion #6057, per-batch attestation #6045, ID publication #6062. What actually exists today is stated only in the final section, [What exists today](#what-exists-today), and in [Ingest today: staging, classification, promotion](../concepts/ingest.md).

Atlas today is a snapshot: a corpus collected by hand-run scripts and frozen at its last publish. The design goal is the opposite property — a corpus that stays current by itself, because every source Drew produces flows in continuously, under the same classification and promotion discipline, with deletions propagating as reliably as additions. "All sources" means the target list below, not a rhetorical all: a source is in scope when it is named here or added by the sources-inventory pass, and each source enters only under the tier machinery — **classification before promotion, never after**.

## The target source list

### Queue and project-management data

The fleet queue and broker data (tasks, sessions, claims, parks, dispositions) and the project-management stack data (aggregator state, boards, reconcile records). Holder: the queue broker's database and the PM stack on the laptop. Designed shape: event hooks where the source supports them, scheduled sweeps otherwise, delta-only via broker watermarks. Content is operational data, internal tier by default; a session transcript attached to a task inherits the session classification rules, not the task's.

### All GitHub, for all orgs

All issues, pull requests, commits, and files, for all orgs and repos in scope — the estate is 102 admin orgs by fleet records, accessed only through the `gh` CLI and REST (the GitHub connector is banned by standing rule).

State against that target, precisely: comments are in — the extraction for issue and pull-request comments across the estate is done (#6023 closed, the ingestion writer #10688 landed, and the `atlas-github-comments` binary ships in atlas-core). What is missing is everything else as first-class objects — issues and pull requests themselves, not only their comments; commits; repo files beyond a partial first pass — and any continuous delta mechanism in place of one-shot extraction (the designed shape is webhook-driven where possible, or ETag and `since`-cursor sweeps otherwise).

The external half of GitHub — repos Drew interacts with but does not own — is gated, and the gate is policy, not engineering: licensing research #6024 and the three-zone external-content policy #6052, both open. A candidate external set has been enumerated (31 candidate repos, from fork upstreams, account activity, and dependency manifests), and per-repo licence capture at ingest is already built — but no external content is ingested until the licensing and zone questions are answered. Design may proceed; external ingestion may not be built ahead of the gate.

### All Muse data

All Muse conversations and data: the main chat and side chats, the memory tree, and artifacts and documents produced for Drew. No connector issue exists for this source yet, and no export or API mechanism is recorded on any issue — designing that mechanism is part of this workstream, not a detail to assume. Until the mechanism is designed, Muse material reaches Atlas only insofar as transcripts happen to land in stores the existing client adapters already read. The sources-inventory pass (below) is where the mechanism gets decided.

### All OpenCode sessions

All OpenCode sessions from the laptop's session store, plus OpenChamber session data, which shares that store. Designed shape: the session-close hook (a designed trigger that stages a session when it ends — not built today) or a sweep over the session store. One question inside this source is explicitly undecided and must not be silently decided by an implementer: whether to ingest **thinking traces** — the model's internal reasoning recorded with a session. #9097 files it as an investigation requiring a recommendation covering volume, value, and classification implications. Drew decides it; this design records it as open.

### All ID objects

All objects from the ID system (Drew's identity and object-graph programme), whose stores are its own laptop and hosted databases. The designed direction today runs the other way — Atlas objects publishing *into* the ID graph, via the #6062 publication format and the projector set #11182–#11186, all open and unstarted. This workstream asks for the reverse flow as well: ID objects into Atlas search, with automatic vectorization of ID content feeding ID's own search. Neither direction is built. See [Design: search integration for fleet and ID](fleet-and-id-integration.md).

### The sources-inventory pass

A standing inventory whose job is to make "all" checkable: anything it finds that is missed gets its own connector issue. The first pass must disposition at least:

- **Zettel / notes** — the two Zettelkasten repos (Emanote-based) and Drew's notes generally.
- **Fleet Reports** — the Fleet Reports Space records.
- **The ask-ledger** — the ledger logs of questions asked and answered across the fleet.
- **Quota and burn records** — provider quota and usage records, which are also the statistics source class in embryo.
- **Laptop state stores** — the state directories of the laptop's services and agents, inventoried store by store rather than swept wholesale.

Two memory connectors from the memory-integration programme belong to this pass's family and are already filed: #8622 (corpus registry plus the memory-lancedb connector — OpenClaw's local memory store as the first external corpus connector) and #8626 (the remaining corpus connectors: memory files, notes, repo docs, research reports).

### Gmail — a future source only

Gmail is named here for exactly one reason: so that its absence from the pipeline is a recorded decision, not an oversight. Drew has paused all Gmail background checks (2026-11-22), regardless of connection state. This design therefore does not design a Gmail pipeline — no collection shape, no cadence, no classification rule beyond naming it a future source that would sit at the internal or private rung by default (code spellings `personal` / `local-only`) if it were ever unpaused. Any future mail ingest needs both reconnection and Drew lifting the pause, as its own decision.

### Standing constraints on every source

- Tier assignment happens at ingest, per source class, under the classification manifest; a manifest can narrow access, never widen it.
- Secret-tier content is held out of automated sweeps entirely until the local secret-tier embedder (#6059) lands. Secret text is never sent to a hosted embedder. A held source class is a queue, not a rejection — held items re-offer automatically once the route exists.
- The keep-all rule: keep all data; only byte-identical duplicates are cleaned. Every chunk records every place it was found (its occurrences); deduplication must never silently drop provenance.
- Adding a source class, loosening a class's inclusion test, or running a bulk backfill beyond the runbook bound is always a human decision, even inside an otherwise automatic system.
- Sweeps never delete. Retirement is a report a sweep surfaces; erasure happens only through the deletion mechanism below or a human decision.

## The pipeline shape

Every source, whatever its holder, flows through the same pipeline. The shape is the contract; a source-specific shortcut around any stage is a defect in the source's connector, not a simplification.

### Capture: source-of-record CDC/hooks plus a reconciliation crawl

Each source is captured from its source of record — the system that owns the data — by change-data capture (CDC: consuming the source's own change stream) or hooks where the source emits them: session-close for session stores, webhooks for GitHub, broker watermarks for queue data. Hooks alone are not trusted to be complete, so every hooked source also gets a periodic **reconciliation crawl**: a full or watermark-bounded re-read that finds anything a missed hook, a crashed consumer, or a restored backup would otherwise lose. The crawl is the correctness floor; the hooks are the latency optimisation.

### Detection: the ladder

A crawl or hook does not mean reprocessing everything. Each candidate passes a detection ladder, cheapest test first, and stops at the first rung that settles it:

1. **Metadata** — size, modification time, source watermark, or version marker unchanged: skip, at metadata cost only.
2. **SHA-256** — content hash unchanged: skip the parse and embed, at hashing cost only. This is the rung that makes continuous ingest affordable.
3. **Full process** — the content actually changed: parse, chunk, classify, and embed as below.

Delta detection is span-anchored where the source supports it: a small edit re-embeds the one changed chunk, not the document. Stored vectors are keyed by the hash of the exact window text they were computed from, because the boundary-drift evidence is that a vector is a function of its exact window — reuse only byte-identical windows, and re-embed on any context change.

### Per-document processing: parser, chunking, and model stamps

A changed document is parsed by its source class's parser, chunked under the chunk-boundary policy (structural boundaries first; every declared classification boundary splits; uncertain text becomes one secret-bearing chunk rather than a guessed split), and stamped, per document and per chunk, with the parser version, the chunking policy version, and the embedding model identity and revision used. Chunk-level hashing gives each chunk its stable identity (`content_hash` is the join key across stores), so occurrences of the same chunk in different sources or versions are recognised as one chunk with many occurrences, never as silent duplicates.

### Staging, classification, gate promotion, attestation

Processed material does not write to the corpus. It lands in a staging unit (a per-unit staging database, vectors not yet stored), where:

1. **Classification** assigns each chunk its effective sensitivity and disposition. Unknown, missing, or malformed classification fails closed to secret-bearing. This happens here, in staging — **classification before promotion, never after** is the pipeline's non-negotiable ordering, because promotion is the moment material becomes searchable and a post-promotion classification pass would leave a window in which unclassified material is served.
2. **Gate promotion** (#6057) judges the batch through five ordered stages — write, classify, dedup, provenance-check, publish — recorded as a durable, content-free audit record (counts, enumerated reason codes, row ids, hashes; never content). A stage with no recorded verdict has not passed; verdicts are recorded strictly in order; a batch that fails the same reason three times is quarantined for human review and stops auto-retrying. Quarantine is a state, not a deletion.
3. **Attestation** (#6045) binds the promoted batch to its sources with a per-batch, in-toto-shaped statement: which source versions produced which chunks, under which parser, chunker, classifier, and model stamps. The attestation is what makes a later "where did this chunk come from, and under what processing?" answerable without re-deriving it.

Only a batch that passes the gate and carries its attestation becomes corpus: embedded at promotion under the one mandated embedding model (vectors from different models are not comparable, and a unit that names the wrong model — or none — is refused), written as a new scan shard, and registered with its merge manifest.

### Deletion: tombstones that propagate as one operation

Deletion is a first-class flow, the mirror of ingest. When a source deletes an item — or a repo is made private, which is a deletion for access purposes — the pipeline writes a **tombstone**: a durable record that this content is gone, keyed by the same content identity ingest used. The tombstone propagates across every store that can serve or recommend the content, as **one operation**, not four eventual cleanups:

- the content-addressed store (CAS),
- the full-text index (FTS),
- the vector store,
- the recommender's tables.

A purge that removes a chunk from the vector store but leaves it findable by full-text search, or recommendable from a stale table, has not deleted it. Propagation covers replicas and caches on the same principle, and is policy-governed and distinct from ordinary retention retirement: retention decides how long Atlas keeps what it may keep; a tombstone records what it may no longer serve at all. The gate record and attestation for a deleted batch are not deleted — they are the audit trail that the deletion happened.

## What exists today

Grounding, so the design above is never mistaken for the present:

- **Ingestion today is manual and script-driven.** The only Atlas schedule on the laptop is the daily backup timer. The ingest scripts still live outside the code repo (#5670 migration open), and the recurring sweep runner is unbuilt (#7969: a runner core exists in atlas-core, but it creates and enables no schedule — cadence is an operator decision gated on Drew's go-ahead).
- **The production pipeline is the older five-step shape** — collect, normalize, index, publish, access — driven by `atlas-ingest.py` with per-client adapters, normalizing conversations into one local database that is the durable asset, building the FTS and vector indexes from it, and publishing a hot copy to Turso as distribution. Corpus counts consumers see are frozen at the 2026-10-31 census — 18,325 sessions, 122,124 messages, 107,639 chunks, in a monolith plus 29 topic shards (28 internal shards holding 107,001 chunks and 1 secret shard holding 638 chunks; zero public-tier shards) — while the store has since grown without a verified recount.
- **GitHub comments are the one completed new source.** Extraction and the ingestion writer landed (#6023, #10688); GitHub objects, commits, files, and continuous delta have not.
- **The staging foundation is built.** Per-unit staging databases with a versioned schema, a staging-unit registry, classification-at-staging with boundary-safe chunking, the batch gate's audit record and state machine (#7709, the record half of #6057), and a one-shot embed-at-promotion command all exist in atlas-core. What does not exist is the standing machinery around them: the aggregator as a service, the session-close extraction hook, and the gate's stage checks beyond the record they write into are later slices, not running code.
- **Traces are already a corpus source.** Atlas's own query traces, statistics, and weekly reports are ingested as a statistics-and-reports source class inside atlas-core. That is the one place where something like automatic ingest — system output flowing back into the corpus by construction — already happens.
- **Nothing else in the target list is connected.** No queue/PM connector, no Muse connector, no ID projector, and no memory connector (#8622, #8626) is built; no source is captured by hooks or CDC; deletions do not propagate as tombstones. The freshness gap this design closes is, today, the single largest consumer-facing gap in Atlas: every consumer in [Consumers: fleet, agents, memory and ID](../concepts/consumers.md) searches a frozen corpus.
