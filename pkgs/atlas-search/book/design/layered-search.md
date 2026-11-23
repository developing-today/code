# Design: layered search characteristics on Snowflake

> **Design — not shipped behaviour.** This chapter describes a proposed design. Nothing in it should be read as how Atlas works today; the final section, "What exists today", states the shipped behaviour it would build on. Tracking: the Atlas programme epic [#13241](https://github.com/developing-today/code/issues/13241) and book issue [#13251](https://github.com/developing-today/code/issues/13251); the second-embedder evaluation is [#9092](https://github.com/developing-today/code/issues/9092); SPLADE and the signal ablation matrix belong to [#5741](https://github.com/developing-today/code/issues/5741); the Jaccard rescore is [#5733](https://github.com/developing-today/code/issues/5733).

Source: written from `pkgs/atlas-search/crates/atlas-core/src/search.rs`, `fusion.rs`, `filters.rs`, `turso.rs`, `shingles.rs` and `crates/atlas-proto/query-parameters-v1.json` (paths under https://github.com/developing-today/code/blob/main/pkgs/atlas-search/), plus the programme's search-technology research survey. Figures quoted from that survey are research figures, not measurements of this system, unless a section says otherwise.

## The idea: the embedding is the base, not the whole

Atlas's dense signal comes from one embedding model, Snowflake `snowflake-arctic-embed-m-v2.0` (768 dimensions) — see [Embeddings: Snowflake Arctic and the parity contract](../concepts/embeddings.md). "More search characteristics on top of Snowflake" means treating that vector as the *base signal* and layering complementary signals over it, each of which catches failures the layer below misses:

1. **Lexical** — BM25 over full-text search (FTS5). Exact on identifiers, error codes, names and rare strings; blind to paraphrase.
2. **Dense** — the Arctic vector. Handles paraphrase and cross-lingual queries; weak on exactly the rare tokens the lexical leg is best at.
3. **Learned sparse and late interaction** — the in-between signals (§ below), which recover part of each leg's blind spot.
4. **Metadata and structure** — source, kind, time, sensitivity tier, provenance and graph neighbourhood, applied as routing and pre-filtering, not decoration.
5. **Behavioural** — what the user actually opened, re-found and worked with (the raw material of [Design: the single-user recommender](recommender.md)).
6. **A precision stage** — cross-encoder reranking over the fused candidates, covered in [Design: the reranking stage](reranking.md).

The discipline that makes this a design rather than a wish list: every layer is independently measurable on the golden set — see [Evaluation: golden sets and gates](../concepts/evaluation.md) — and a layer is adopted only where that measurement shows a win on Atlas's own queries. The current pipeline, which already fuses the first two layers, is described in [Retrieval: hybrid full-text and vector search](../concepts/retrieval.md).

## Layer 0: use the resolution the base model already has

The cheapest layer costs no new model at all. Arctic Embed 2.0 is trained with Matryoshka Representation Learning (MRL — the model is trained so that a prefix of its vector is itself a usable embedding) with a truncation point at 256 dimensions: truncating the stored 768-dimension vector to its first 256 dimensions retains roughly 98% of the model's MTEB Retrieval score. That figure is from the model's own published evaluation (research survey §1), not a measurement on Atlas's corpus.

That gives a **two-resolution index from the vectors Atlas already stores**:

- a cheap first pass at 256 dimensions over the whole permitted set, then
- a full-quality rescore at 768 dimensions over the first pass's survivors.

Atlas's query side already supports the mechanics — a query vector can be truncated and renormalised per request (the shipped `--trunc` parameter) — but the stored corpus is served today as a single-resolution scan. The design step is to make the two resolutions a deliberate serving shape: first-pass depth and rescore depth become named parameters, and the recall cost of the 256-dimension pass is measured against the full-width scan on the golden set before it becomes a default. If the measured cost is near the published ~2%, this is close to a free 3× reduction in first-pass vector work, and it stacks with the ANN work below rather than competing with it.

## The in-between signal: learned sparse (SPLADE)

SPLADE (learned sparse retrieval) is a BERT-style model whose output is a weight for every term in a ~30k-term vocabulary, including terms *not present* in the text: a chunk about a "connection reset" also lights up for *drop* and *socket*. The result is still a sparse vector served from an ordinary inverted index, so it keeps BM25's infrastructure and exact-match behaviour while recovering much of dense retrieval's paraphrase handling.

In this design SPLADE is a third retrieval leg, fused by rank with the existing two:

- an encoder pass at ingest time produces the sparse vector per chunk (a stored postings table, keyed like the other derived indexes);
- at query time the sparse leg returns its own ranked pool, and reciprocal rank fusion (RRF) merges three lists instead of two;
- BGE-M3's sparse head (§ "The second-embedder decision") is the alternative source of the same signal, if that evaluation favours it — the leg's place in the pipeline is identical either way.

Adoption is gated, not assumed. The failure taxonomy from the golden set has to show paraphrase-miss failures that *both* existing legs miss; if it does not, the leg is cost without benefit. See "What exists today" for SPLADE's current gate status: it has never been gated.

## The mid-stage: late interaction (ColBERT) over the top ~100

ColBERT-style late interaction gives every token its own embedding and scores a pair by MaxSim — for each query token, its best-matching document token, summed. Quality sits between bi-encoders (one vector per text) and cross-encoders (joint scoring of the pair, [Design: the reranking stage](reranking.md)), at roughly 10–100× bi-encoder compute (research survey §2, directional figures from the cited decomposition: BEIR nDCG@10 ≈ 0.50 for ColBERT against ≈ 0.43 for BM25 and ≈ 0.47 for SPLADEv2).

That cost profile decides its placement: **a mid-stage over the top ~100 fused candidates, never a first-stage retriever**. The fused pool is rescored by late interaction, and the rescored order feeds the reranker (or the result list, if no reranker is enabled). ColBERTv2-style centroid-plus-residual compression (~20–36 bytes per token-vector, research figure) is what makes storing per-token vectors for a corpus of Atlas's size a serious option rather than an absurd one; the storage estimate for Atlas's chunk count is part of the design work, not a number this chapter asserts. As with SPLADE, BGE-M3 is the alternative supplier — it emits ColBERT multi-vectors in the same pass as its dense and sparse outputs.

## Metadata is routing, never a post-filter

A filter applied *after* retrieval — take the top `k`, then discard what does not match — starves recall: the retriever returns `k` results, the filter throws most of them away, and the relevant item at rank `k + 1` never surfaces. Worse, in Atlas one of the "filters" is the sensitivity tier, which is a **security boundary**: a tier check that is merely post-hoc is a leak waiting for the one consumer path that forgets it.

So in this design, metadata — source, kind, topic, project, client, time range, and above all sensitivity tier and the caller's ceiling — is part of the query plan:

- **Pre-filter inside the retrieval layer.** Each leg's query is constructed with the permitted set already applied, so every candidate any leg returns is already permitted. (This is the direction the shipped code already takes for the ceiling and filters — see "What exists today".)
- **Partition and route.** Where the corpus is physically partitioned — per-tier shards, per-source collections — the planner routes the query to the partitions it is allowed and intended to search, and filtering becomes routing. A query that cannot touch the secret tier is never *sent* to the secret partition, rather than sent and trimmed afterwards.
- **Over-fetch where the engine forces it.** An approximate index that cannot filter internally (the DiskANN path below) is queried deeper than the pool size, the survivors are joined against the permitted set, and the over-fetch factor is tuned per filter selectivity and validated on the golden set — the permitted-set check still happens inside the retrieval layer, before fusion, never in a consumer.

## Behavioural signals

The last retrieval layer is evidence from use: which results were opened, which documents are re-found repeatedly, which items co-occur in the user's sessions. In this design those signals enter as a bounded adjustment over the fused candidates — a boost with a named cap, never a replacement for the content signals — and they are the same signals the recommender is built from, designed as one loop: see [Design: the single-user recommender](recommender.md). Two cautions are part of the design, not afterthoughts: behavioural boosts are exactly the kind of change that flatters easy queries while collapsing the tail, so they are golden-set-gated per query class like every other layer; and any learning from behaviour waits on the feedback loop described in [Design: the reranking stage](reranking.md) — impressions and clicks without recorded judgements are not training data.

## Fixing the serving physics: from exact scan to an ANN path

Today the vector leg is an **exact cosine scan** over the permitted set — correct by construction, and slow: the programme record puts it at roughly 4 seconds per query against the production store, and the stored vectors are plain BLOBs on which `vector_top_k` does not work. Exactness is a fine correctness reference and a poor serving plan. The design replaces the scan with an approximate nearest-neighbour (ANN) path, in preference order:

1. **libSQL native vectors where the store supports them** — type the vector column `F32_BLOB(768)` with a DiskANN-backed `libsql_vector_idx`, and retrieve with `vector_top_k`. This keeps one store, one SQL surface, and the pre-filter discipline above (with over-fetch where the index path needs it).
2. **An embedded-replica strategy** — libSQL's embedded replicas sync a local file from the remote primary, so the same FTS5 + native-vector SQL runs on the laptop offline and against the hosted store online with no code change. This is also the offline story for fleet and ID consumers.
3. **A dedicated index** — a separate ANN store beside libSQL, only if the first two fail the measurements; it costs a second system to keep in agreement with the corpus, so it has to win clearly, not marginally.

Whichever path is chosen, the acceptance test is the same: **recall of the ANN path measured against the exact scan on the golden set**. The exact scan is the oracle — for each golden query, the ANN path's top-`k` is compared with the scan's top-`k`, and the recall loss is a named, budgeted number, traded explicitly against the latency it buys. An ANN default that has not been measured against the scan does not ship; the scan remains available as a backend option and as the eval oracle.

## The second-embedder decision (#9092)

Staying on Arctic is defensible — per the research survey it leads its size class on English retrieval, and it is not the weak link. The one challenger with a structural argument is **BGE-M3**: a single encoder pass that emits all three signals this chapter layers separately — a dense vector, a learned-sparse vector, and ColBERT multi-vectors. If BGE-M3 wins on Atlas's golden set, adopting it collapses the SPLADE and ColBERT layers from separate components into properties of the embedding pass. If it does not win, the layered design above stands on the Arctic base unchanged. (On the survey's published numbers the two are close and split by benchmark — Arctic ahead on English retrieval and CLEF, BGE-M3 ahead on the MIRACL average — which is precisely why the decision belongs to Atlas's own set, not to a leaderboard.)

Any model change follows migration discipline, because vectors from different models are not comparable and mixing them degrades results silently:

1. **Every stored vector is tagged with its `model_id`** (and dimension), so no query ever fuses candidates from two spaces unknowingly.
2. **The new model is built into a shadow index** — a new table or collection, backfilled incrementally, with unchanged chunks suppressed by the content-hash dedupe cache.
3. **Cutover is decided on an eval result, not on completion.** The golden set runs against both indexes; reads flip when the new index wins, or ties at acceptable cost. The old index is kept until the flip is proven, then dropped.
4. During a long backfill, a stable base index plus a small continuously-updated delta index, queried together and fused, keeps fresh content searchable.

## What exists today

- **Shipped:** hybrid retrieval with two legs — FTS5/BM25 and the Arctic dense vector — fused with RRF. The shipped fusion constant is **RRF `k = 10`** with equal leg weights (`fts_weight = 1.0`, `vector_weight = 1.0`), a default pool of 100 candidates per branch and a default `k` of 10 results, all pinned in `crates/atlas-proto/query-parameters-v1.json`. (The Python eval lineage fuses at `k = 60`; that is the eval harness's configuration, not the service's.)
- **Shipped:** filters and the sensitivity ceiling are applied **inside the branch queries, before any ranking** — the pre-filter half of this chapter's metadata design is already the shipped behaviour for the monolith backends. Partition routing by tier exists in the shard fan-out (a shard above the caller's ceiling is never connected or scanned).
- **Shipped, off by default:** query-side truncation (`trunc`, default 0 = full 768 dimensions) — the mechanics for the two-resolution design exist on the query side, but no 256-dimension first pass is run. Also shipped and off: the Jaccard shingle rescore (#5733), a third signal applied over the fused union. Its held-out gate **failed** — on the 54-query near-exact set, MRR@10 improved by +0.0361 against a pre-agreed bar of +0.10 (the general set passed its no-regression bar) — so `jaccard_depth` stays 0 and no production ranking change was made.
- **Not shipped:** SPLADE. There is no SPLADE encoder, no postings table and no SPLADE run in the repository; it exists only as a named signal in the #5741 ablation matrix. **No gate has run for SPLADE** — the Jaccard verdict is about set-overlap geometry and neither validates nor undermines it.
- **Not shipped:** ColBERT/late interaction, behavioural ranking signals, and any reranker — see [Design: the reranking stage](reranking.md).
- **Serving physics today:** the default vector backend is the exact scan (`Scan`). Two index strategies exist in code behind it — `IndexFixed` (one fixed-depth `vector_top_k` fetch) and `IndexFill` (escalating depth until the pool survives filtering) — but neither is the default; which becomes the default is to be decided at the #5356 cutover from A/B evidence. The sharded backend likewise serves its vector branch by exact per-shard scan. The ~4-second exact scan against the production store is the programme's recorded figure, not a benchmark run for this book.
- **Second embedder today:** an open evaluation (#9092), not a decision. A legacy BGE-small vector space (384 dimensions) coexists in the publish store as a dormant fallback; it is not a challenger in the #9092 sense and its vectors are not comparable with the Arctic space.
