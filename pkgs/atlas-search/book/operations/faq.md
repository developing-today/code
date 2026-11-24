# FAQ

Source: `pkgs/atlas-search/crates/atlas-search/src/main.rs`, `pkgs/atlas-search/atlas_search.py`, `pkgs/atlas-search/worker/src/` (`index.ts`, `search.ts`, `hot_cache.ts`, `space.ts`), `pkgs/fleet/crates/fleet-search/src/cli.rs`, and `nixos/home/laptop-backups.nix` under https://github.com/developing-today/code/blob/main/pkgs/atlas-search/ — provenance only; this chapter is complete without following it.

Short answers, each pointing at the chapter that carries the full story. Operational symptoms — slowness, fallbacks, refusals — are diagnosed step by step in [Troubleshooting](troubleshooting.md).

### Why are there two CLIs, `atlas-search` and `fleet search`?

They are two front doors to one engine, not two engines. `atlas-search` is the standalone binary: the complete surface, including the service verbs `serve`, `init`, and `fetch-model`, and every engine flag. `fleet search` is the fleet binary's search domain — the client subset of that surface that translates to the wire request or to routing — and `fleet serve search` is the same serve role rehomed into fleet, which is what production actually runs. On the standalone route, `fleet search` delegates to the deployed `atlas-search` artifact and inherits its output byte-for-byte; the writers that render service envelopes are parity-guarded copies, so both routes print what `atlas-search` prints. Rule of thumb: script against whichever binary your seat already has; reach for `atlas-search` when you need a verb or flag the fleet subset deliberately omits. See [atlas-search: search and get-chunk](../reference/atlas-search.md) and [fleet search and fleet serve search](../reference/fleet-search.md).

### Why is search slow?

Because the vector branch is an exact scan. The production vectors are stored as plain BLOBs, `vector_top_k` does not work on them, and so every allowed chunk's cosine distance is computed — about 4 seconds for a plain scan, more under filters. FTS-only search (`--mode fts`) skips that cost entirely. There is no operator fix today; an approximate-nearest-neighbour path is programme work. The measurements and the reasons the index backends are not a workaround are in [Troubleshooting](troubleshooting.md).

### Why can't the Worker — or an MCP caller — see the secret tier?

Because that surface has exactly one identity and it is pinned at `personal`. The Worker presents the service's `cf-worker` client, whose cap is `personal`, sends ceiling `personal` and context `hosted-per-user` on every search, and verifies the reported effective ceiling before caching anything. No request field can ask for more, and the Worker holds no credential that could search higher — that absence is the design, not a missing permission. Higher ceilings exist only on the laptop service, under named client identities with their own caps; see [Sensitivity tiers and client ceilings](../concepts/sensitivity.md).

### Where do vectors live?

In the corpus stores, next to the chunks they describe — never in staging databases, and never only in one place. The production set is the table `vec_snowflake_m_v2_768_f32`: one 768-dimension float32 vector per chunk, in the Turso publish (typed `F32_BLOB(768)`, with a `libsql_vector_idx` index) and in each scan shard beside its `chunks` table. The local backend can also read vector side stores — separate files joined to the corpus **by `content_hash`, never by chunk id**, because the id spaces differ between stores and an id join would silently attach the wrong vectors to thousands of chunks. A dormant bge-small set (`vec_bge_small_384_f32`, 384 dimensions) remains in the publish as a fallback; it is a different, incomparable space, and the Worker refuses it outright. Staging databases store no vectors at all by design — vectors are written only at promotion, by the embed-at-promotion step. The store inventory is in [Data stores and schemas](../reference/data-stores.md).

### Why did setting `HOT_CACHE_ENABLED` not change D1 behaviour?

Because the D1 hot cache is not wired into the request path. The flag (only the literal `"true"` counts) plus the `HOT_CACHE` binding would construct the store — and then nothing would call it: the store's get / put / touch / evict primitives are exercised only by their own tests. Read-path integration is follow-on work (#13105), alongside freshness evaluation (#13101) and the eviction sweep (#13103). Hosted answers today are cached in the Durable Object cache, always. See [The service and the Worker](../concepts/service-and-worker.md).

### Is the corpus current?

The corpus you can search is current as of its last publish — and the last **verified census is frozen at 2026-10-31: 18,325 sessions / 122,124 messages / 107,639 chunks** (#3125). The underlying store has since grown to a ~7.3 GB monolith with no verified recount, and ingestion is still manual and script-driven, so no newer count exists to quote and no consumer recounts on your behalf. A fresh census is an early programme deliverable; how one would be taken, and why the October numbers are the ones consumers still see, is in [Backup, restore and the census](backup-and-census.md). Treat any undated Atlas size claim — including one in an older chapter or design — as that October snapshot until a dated replacement exists.

### Can ID search Atlas today?

No. There is no shipped path by which the ID system queries Atlas: no ID-side Atlas client, no publication feed from the corpus into the ID graph, and no MCP tool pair joining them is live today. What exists is design — an Atlas-to-ID publication format (corpus objects plus provenance and lineage relations as ID-graph objects, #6062) and the wider federation work — and it is written up as design, with its grounding in what exists today, in [Design: search integration for fleet and ID](../design/fleet-and-id-integration.md). Anyone who needs Atlas results in ID work today copies them there by hand; the book will not pretend otherwise.

### What does the Python file do?

`pkgs/atlas-search/atlas_search.py` is the original implementation and the permanent parity oracle. The Rust engine is its semantics port: wording, defaults, error texts, and exit codes are mirrored from it, the parity fixtures were recorded from it, and it remains the evaluation instrument of record — every parity gate runs against Python behaviour. It is also still the only surface with the bge/fastembed embedder path and with non-default embedding models for vector search; the Rust binary refuses both, loudly. It is kept alongside the Rust binary on the laptop as `atlas-search-py`, renamed but retained — never removed first. Its own chapter is [atlas_search.py: the Python reference implementation](../reference/python-implementation.md); the places its behaviour deliberately differs from the Rust port are listed in [Troubleshooting](troubleshooting.md).

### Why did my search fall back to standalone?

Because the service route failed or was disqualified, and the fallback is loud by contract: a warning on stderr, a note in the result envelope, standalone results, exit 0. The usual causes are the service being down, the seat having no client token configured, a `--backend local` or non-default `--vec-table` request (which the service cannot serve), or the verb being `get-chunk`, which is always standalone. It is a degradation, not an error — but it is never silent, so if you see the warning and did not expect it, [Troubleshooting](troubleshooting.md) walks the causes in order.

### A cached answer is wrong now. How do I clear it?

Purge the one key. The Worker cache has no TTL (#6805), so waiting does nothing; `POST /v1/cache/purge` on the Worker (gated) with `{ "query": …, "k": … }` deletes exactly that entry, and the next search re-runs. Remember that `k` is part of the key — purge at the depth the answer was cached at. There is no bulk or prefix purge, by design. The inspect route (`GET /v1/cache/inspect`) proves an entry exists before you purge it; both routes are in [The Worker HTTP API](../reference/worker-api.md).

### Why are there two caches with opposite lifetimes?

Because they protect different things. The service **temp store** holds one job's fused ranking — ceiling-gated snippets fetched for one client — so it is in memory, lives 10 minutes, and dies with the process: perishability is its security property. The Worker **Durable Object cache** holds whole hosted query answers so repeated agent queries never re-pay the multi-second laptop fan-out, so it persists indefinitely: retention is its cost property. They share the nickname "the Atlas cache" in older documents and nothing else — different machines, different stores, opposite reasons. [The service and the Worker](../concepts/service-and-worker.md) keeps them apart throughout.

### What happens to running searches when the service restarts?

They are gone, and that is the designed behaviour. Jobs, their rankings, and the queue live in the serve process's memory; a restart returns the service empty — config, model, traces, and every corpus store survive on disk — and clients simply submit again. If a client reports a job handle failing after a restart, that is the expected shape, not data loss. See [Running the service and the Worker](services.md).

### Can I change the embedding model to a better one?

Not as a configuration change. Atlas runs **one embedding model for every tier** — Snowflake `snowflake-arctic-embed-m-v2.0` at 768 dimensions — because vectors from different models are not comparable, and the whole corpus, every shard manifest, the promotion gate, and the Worker all pin that one identity. A shard or staging unit stamped with a different model is refused at discovery and at promotion rather than silently mixed in. Changing models means re-embedding the corpus into a second vector set beside the first and cutting over on evaluation evidence — a programme, with its own gates — not a flag. The contract is in [Embeddings: Snowflake Arctic and the parity contract](../concepts/embeddings.md).

### Which numbers can I trust for evaluation?

The ones whose configuration is named. Golden-311's production row — 189 hits@10, MRR 0.3894 — is the one-stage hybrid at RRF k = 10, pool 100 per branch. The eval harness's hybrid is a different configuration (RRF k = 60, top-30 branches, trunc-384) and produces different numbers for the same set; both are labelled "production hybrid" somewhere in the older records, which is exactly why this book names the configuration with the number. The harness, the splits, and the held-out discipline are in [Evaluation: golden sets and gates](../concepts/evaluation.md).
