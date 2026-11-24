# Atlas — Modern Search & Retrieval Technology Survey

**Purpose:** Research backing for a program-planning prompt to fully investigate Atlas — Drew's personal corpus search system (Snowflake embeddings, Turso/libSQL storage, FTS + vector search, golden-set evaluation, a recommendation component, CAS-backed corpus from the #3125 provenance program) — and to plan: automatic ingest of all data sources, query-planning improvements, search integration for fleet and ID, MCP + programmatic access, and richer search characteristics layered on top of the Snowflake embedding base (hybrid signals, reranking, recommender, etc.).

**Method:** Public web sources via search + fetch, November 2026. Claims that could not be confirmed against a primary source are tagged **[unverified]**. Benchmark numbers are quoted from the cited source; MTEB/BEIR numbers are not comparable across papers unless the dataset split and metric match — treat cross-source deltas as directional.

**Atlas context assumed throughout** (from the parent survey): ~18,325 sessions / 107,639 chunks at last verified state; storage in Turso/libSQL; embeddings keyed by content hash + `model_id` as a derived, rebuildable index; tiered-sensitivity machinery from the #10412 review; Atlas MCP server already live and wired into mcpx; `fleet search` documented as a consumer; ID-graph publication format design is #6062 (open, design-only) with projector set #11182–#11186 unstarted.

---

## 1. Snowflake Arctic Embed — the embedding base

### What it is

Snowflake's Arctic Embed is a family of open-weight (Apache 2.0) encoder embedding models optimised for retrieval, released in successive generations:

- **v1 / v1.5 (English-first):** a size ladder (xs / s / m / l) trading latency for quality; the medium and large v1 models were widely deployed defaults in 2024.
- **Arctic Embed 2.0 (Dec 2024):** the multilingual generation. Two published sizes:
  - **arctic-embed-m-v2.0** — 305M total params, 113M non-embedding, **768 dimensions**, built on GTE-multilingual-base, 8,192-token context via RoPE.
  - **arctic-embed-l-v2.0** — 568M total params, 303M non-embedding, **1,024 dimensions**, built on a long-context XLM-R-Large variant, 8,192-token context.
- Both 2.0 models are trained with **Matryoshka Representation Learning (MRL)** at a 256-dimension truncation point plus quantization-aware training: truncating m-v2.0 to 256 dims retains ~98% of its MTEB Retrieval score, and 4-bit quantization reaches ~128 bytes/vector with small further loss — i.e. the same stored embedding can serve a cheap first pass at 256 dims and a full-quality pass at 768/1024 dims.

Sources: [arctic-embed-m-v2.0 model card](https://huggingface.co/Snowflake/snowflake-arctic-embed-m-v2.0), [Arctic-Embed 2.0 paper (arXiv:2412.04506)](https://arxiv.org/pdf/2412.04506), [Snowflake engineering blog](https://www.snowflake.com/en/blog/engineering/snowflake-arctic-embed-2-multilingual/).

### MTEB standing (from the 2.0 paper, nDCG@10 averages)

| Model                 | Params              | Dim  | MTEB-R (BEIR 15) | MIRACL (4) | CLEF            |
| --------------------- | ------------------- | ---- | ---------------- | ---------- | --------------- |
| arctic-embed-m-v2.0   | 305M (113M non-emb) | 768  | **55.4**         | 55.2       | **51.7 / 53.9** |
| arctic-embed-l-v2.0   | 568M (303M non-emb) | 1024 | **55.6**         | 55.8       | **52.9 / 54.3** |
| bge-m3                | 568M                | 1024 | 48.8             | **56.8**   | 40.8 / 41.3     |
| multilingual-e5-base  | 560M                | 1024 | 51.4             | 54.0       | 43.0 / 34.6     |
| gte-multilingual-base | 305M                | 768  | 51.1             | 52.3       | 47.7 / 53.1     |

Reading: Arctic 2.0 leads its size class on English retrieval and CLEF while staying competitive on MIRACL — its design goal was explicitly "multilingual without sacrificing English," which is where earlier multilingual models (including BGE-M3 on English) give ground. BGE-M3 still edges it on MIRACL average.

### How it compares for a local deployment

|                     | Arctic Embed 2.0 (m / l)                                                                                                                  | BGE-M3 (BAAI)                                                                                           | Multilingual E5 (large)                                    | Nomic Embed (v1.5 / v2-MoE)                                                         |
| ------------------- | ----------------------------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------- | ---------------------------------------------------------- | ----------------------------------------------------------------------------------- |
| License             | Apache 2.0                                                                                                                                | MIT                                                                                                     | MIT                                                        | Apache 2.0                                                                          |
| Dim                 | 768 / 1024, MRL→256                                                                                                                       | 1024                                                                                                    | 1024                                                       | 768, MRL→256                                                                        |
| Context             | 8,192                                                                                                                                     | 8,192                                                                                                   | **512** (chunking burden)                                  | 8,192 (v1.5) / 512 (v2-MoE) **[unverified for v2 context in all secondary tables]** |
| Languages           | ~74 [from model-card metadata, secondary]                                                                                                 | 100+                                                                                                    | 94–100+                                                    | English-first (v1.5); ~100 (v2-MoE)                                                 |
| Distinct capability | Best English retrieval per parameter; MRL + quantisation-aware compression                                                                | **One pass emits dense + learned-sparse + ColBERT multi-vector** — hybrid search without a second model | Broadest tooling/prefix ecosystem; the safe baseline       | Fully open training data/code; smallest footprint (137M for v1.5)                   |
| Local inference     | ONNX exports exist (incl. `onnx-community` INT8 ports for the reranker sibling); runs via fastembed-rs / `ort` / candle BERT-family paths | Via fastembed-rs `Bgem3Embedding` (dense+sparse+ColBERT in one type)                                    | Everywhere (sentence-transformers, fastembed, TEI, Ollama) | Everywhere; Ollama default                                                          |

Sources: [embedding comparison survey](https://github.com/thorwhalen/equate/blob/HEAD/docs/research/13-llm-and-modern-embedding-matching.md), [2026 embedding model comparison](https://resources.rework.com/tools/ai-data-infra/best-embedding-models-2026), [fastembed-rs docs](https://docs.rs/fastembed/latest/fastembed/).

**Practical read for Atlas:** staying on Arctic is defensible — it is not the weak link. The two credible reasons to evaluate alternatives on Atlas's own golden set are (a) BGE-M3, if Atlas wants learned-sparse + late-interaction signals _from the embedding model itself_ instead of from separate components, and (b) a long-context or instruction-tuned model (Qwen3-Embedding family, via fastembed's candle backend) if chunking artefacts dominate the failure taxonomy. Any swap is a full re-embed (vectors from different models are not comparable) — see §7 for the versioning pattern that makes that safe.

### What "more search characteristics on top of Snowflake" means in practice

The phrase, as the field uses it, means: the dense embedding is the _base signal_, and quality comes from layering additional, complementary signals over it rather than from a bigger embedding model:

1. **Lexical signal** — BM25/FTS over the same chunks (§2), fused by rank.
2. **Learned sparse / late interaction** — SPLADE or ColBERT-style scoring (§2).
3. **Metadata & structural signals** — source, type, time, sensitivity tier, graph neighbourhood (Atlas-specific: provenance, occurrence counts, ID-graph links).
4. **Behavioural signals** — clicks/opens, session co-occurrence, recency (the recommender's raw material, §5).
5. **A precision stage** — cross-encoder reranking over the fused top-N (§3).
6. **Query-side intelligence** — planning, expansion, routing (§4).

Each layer is independently measurable on the golden set (§6); that measurability, not any single layer, is the actual best practice.

**Relevance to Atlas:** Arctic 2.0's MRL truncation gives Atlas a free two-resolution index (256-dim first pass, full-dim rescore) inside its existing model choice — the cheapest "more characteristics" step is using capabilities of the base model Atlas already stores, before adding new models.

---

## 2. Hybrid search — BM25 + dense, and the signals beyond both

### What it is

Hybrid search runs two or more retrievers in parallel over the same corpus and merges their ranked lists. The canonical pairing is **BM25** (sparse lexical: term frequency × inverse document frequency over an inverted index; in Atlas's stack, SQLite FTS5) and **dense ANN** (bi-encoder embeddings; in libSQL, `vector_top_k` over a DiskANN index, or exact scan — §7/§8 notes). The two fail in opposite directions:

- BM25 nails identifiers, error codes, names, version strings, rare jargon — and cannot connect "drop connections" to "connection reset."
- Dense retrieval handles paraphrase and (with a multilingual model) cross-lingual queries — and squashes rare tokens into low-information vectors, so it misses exactly the strings BM25 is best at.

Sources: [Hybrid search and reranking: a field guide](https://www.aiagentichandbook.com/blog/hybrid-search-and-reranking-field-guide/), [Hybrid search (Masst docs)](https://docs.masst.dev/ai/retrieval/hybrid-search).

### Fusion: RRF is the default; weighted fusion is the calibrated alternative

- **Reciprocal Rank Fusion (RRF)** — `score(d) = Σ 1/(k + rank_r(d))`, `k = 60` from Cormack et al. (SIGIR 2009). It fuses _ranks_, not scores, which sidesteps the core problem: BM25 scores are unbounded and cosine similarities live in [−1, 1], with per-query distributions that make score normalisation fragile. Appearing in both lists roughly doubles a document's fused score — consensus is rewarded automatically. Lower `k` (10–40) sharpens top-rank differences when one retriever is known-stronger. RRF is shipped natively by Elasticsearch, OpenSearch, Qdrant, Weaviate, Vespa and Azure AI Search; in a SQL stack it is ~15 lines over two `LIMIT k` result sets.
- **Weighted / alpha fusion** — `α·dense + (1−α)·sparse` over normalised scores (Pinecone's model; Weaviate's relative-score fusion is a variant). More expressive, but needs per-corpus calibration and silently breaks when either retriever changes. Best practice: start with RRF; move to weighted only when a golden set can tune α per query class, and pin the fusion type explicitly across upgrades (Weaviate changed its default fusion in v1.24 — the cautionary tale).
- Rule-of-thumb gains quoted across the field guides: hybrid over best single leg ≈ **+5–15% nDCG / +8–15 points Recall@10** on mixed-intent corpora, at ~10–20 ms added latency. **[Directional, per-source; measure on Atlas's golden set.]**

### The two in-between signals

- **SPLADE** (Formal et al., 2021) — _learned sparse_ retrieval. A BERT-style MLM head emits a weight for every vocabulary term (~30k), including terms not present in the text, so a document about "connection reset" also lights up _drop_ and _socket_. The output is still a sparse vector served from an ordinary inverted index: most of dense's paraphrase handling with BM25's infrastructure and exact-match behaviour. Costs: an encoder pass at indexing time, larger posting lists. fastembed-rs exposes it as `SparseTextEmbedding`; BGE-M3 emits an equivalent learned-sparse vector natively.
- **ColBERT / late interaction** (Khattab & Zaharia, 2020; ColBERTv2, 2022) — every token gets its own embedding; relevance is MaxSim over query-token × document-token pairs. Quality sits between bi-encoders and cross-encoders (BEIR nDCG@10 ≈ 0.50 vs BM25 ≈ 0.43, SPLADEv2 ≈ 0.47 in the cited decomposition), at 10–100× bi-encoder compute. Production placement is as a **mid-stage reranker over the top ~100**, not a first-stage retriever; ColBERTv2's centroid+residual compression brings storage to ~20–36 bytes/token-vector. Jina's `jina-colbert-v2` (137M, Apache 2.0) is the small open option.

### Metadata filtering — the unglamorous multiplier

Filtering (source, document type, time range, sensitivity tier, tenant) should be a _first-class part of the query plan_, not a post-filter over an already-truncated top-k (post-filtering starves recall: the ANN returns k results, the filter discards most, and relevant items ranked k+1 never surface). Patterns:

- **Pre-filter** where the engine supports filtered ANN (libSQL's exact-scan path supports arbitrary `WHERE` naturally; the DiskANN `vector_top_k` path needs over-fetch + join, with the over-fetch factor tuned per filter selectivity).
- **Partitioned indexes** — separate indexes/collections per source or tier, with the query planner routing (§4) — filtering becomes routing.
- In Atlas's case the sensitivity tier is also a _security_ boundary (the #10412 tier machinery): a filter that is merely post-hoc is a leak risk if any consumer path forgets it; tier enforcement belongs in the retrieval layer itself.

**Relevance to Atlas:** Atlas already has the two base legs (FTS5 + libSQL vectors). The ordered upgrades are: (1) verify fusion is RRF with a tuned `k` and per-leg depth, measured; (2) add metadata pre-filtering tied to source/tier; (3) evaluate a learned-sparse leg (SPLADE via fastembed, or BGE-M3's sparse head if the embedding evaluation in §1 favours it) — only if the golden-set failure taxonomy shows paraphrase-miss failures that BM25+dense both miss.

---

## 3. Reranking — the precision stage

### What it is

A **cross-encoder reranker** scores each (query, document) pair jointly through a transformer — no pre-computed vectors, full attention between query and text. It is far more precise than bi-encoder cosine and far too slow to run over a corpus, so the universal pipeline shape is:

```
retrieve (hybrid, top 50–200) → fuse → cross-encoder rerank → top 5–20
```

The reranker can only reorder what retrieval found; it fixes _precision_ (positions 1–5), never _recall_. Typical quoted gain over the fused bi-encoder ranking: **+5–15 nDCG@10**. Latency: roughly 50–500 ms for a top-100 pool on CPU depending on model size; ~50–150 ms for 20 candidates with a mid-size model on CPU in the cited local deployments.

Sources: [Reranking & cross-encoders guide (2026)](https://localaimaster.com/blog/reranking-cross-encoders-guide), [hybrid field guide](https://www.aiagentichandbook.com/blog/hybrid-search-and-reranking-field-guide/), [local RAG reranker measurements](https://www.bigiron.cc/guides/adding-a-reranker-to-local-rag-when-a-cross-encoder-pays-for-itself).

### Leading models

| Model                                  | Params            | License                                                                                                                                                                                                                                 | Notes                                                                                                                                                                                                                                                                            |
| -------------------------------------- | ----------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| **BGE-Reranker-v2-m3** (BAAI)          | 568M              | MIT                                                                                                                                                                                                                                     | The default open pick: multilingual (100+), 8,192-token pairs, strongest quality/size balance (MTEB-rerank ≈ 60.4 in cited table). ONNX INT8 ports ~570 MB. Scores are unbounded logits — normalise within the pool before blending with other signals.                          |
| **Jina Reranker v2 base multilingual** | 278M              | Apache 2.0 per cited table; **note:** some deployments report CC-BY-NC terms for earlier Jina reranker releases — verify the current model card before redistributing **[unverified — licence history is inconsistent across sources]** | Latency-critical pick; half the size of v2-m3, slightly lower quality (≈56.8). Ships custom modelling code (`trust_remote_code`) — matters for the candle route below.                                                                                                           |
| BGE-Reranker-v2-Gemma / -MiniCPM       | 9B / 2.7B         | Apache 2.0                                                                                                                                                                                                                              | Top-of-leaderboard quality when GPU latency budget exists (≈64.5 / 62.3).                                                                                                                                                                                                        |
| mxbai-rerank (large/base/xsmall)       | 435M / 184M / 33M | Apache 2.0                                                                                                                                                                                                                              | English fast tier; xsmall is the edge option.                                                                                                                                                                                                                                    |
| gte-reranker-modernbert-base (Alibaba) | 149M              | —                                                                                                                                                                                                                                       | Best accuracy/size ratio in one cited production stack; 8,192 context, INT8 ≈150 MB via fastembed.                                                                                                                                                                               |
| **Cohere Rerank 3 / 3.5** (hosted)     | closed            | API                                                                                                                                                                                                                                     | The commercial reference point: 4K+ context, 100+ languages, ≈62.0 in the cited comparison table; Voyage rerank-2.5 is the other hosted reference. Useful as an _offline quality ceiling_ to measure a local reranker against on the golden set, even if production stays local. |

### Local inference routes (Rust-first)

- **fastembed-rs (`TextRerank`)** — the main Rust route: ONNX Runtime via `ort`, tokenizers via HF `tokenizers`, synchronous API (no Tokio dependency). Supports bge-reranker-base (default), **bge-reranker-v2-m3**, jina-reranker-v1-turbo-en and v2-base-multilingual out of the box, plus user-defined ONNX models. Models cache to `./.fastembed_cache` (`FASTEMBED_CACHE_DIR`).
- **candle** (HF's pure-Rust framework) — the alternative when ONNX Runtime is a liability: prebuilt `ort` binaries assume AVX-class CPUs and have SIGILL-crashed on older NAS CPUs in a documented deployment, which moved that project to candle with runtime CPU-feature dispatch. Caveats: candle needs a _standard_ architecture implementation — bge-reranker-v2-m3 is a plain `XLMRobertaForSequenceClassification` and ports; jina-reranker-v2's custom code does not. fastembed itself uses candle as the backend for its Qwen3 embedding/reranker features, so the two routes coexist in one crate.
- **Serving shape:** in-process library for a CLI/local tool; a small HTTP inference service (embed + rerank endpoints) when several consumers (fleet, ID, MCP server) should share one warm model instead of each loading ~0.5–1 GB.

Sources: [fastembed-rs README](https://github.com/anush008/fastembed-rs/blob/HEAD/README.md), [fastembed docs.rs](https://docs.rs/fastembed/latest/fastembed/), [candle-vs-ONNX ADR](https://github.com/huulanka/hippocampus/blob/HEAD/docs/adr/0008-candle-not-onnxruntime.md).

### Trade-offs & placement rules

- **Pool size is the latency knob:** reranking top-10 was ~3× faster than top-30 in local measurements with no quality loss in that corpus; top-50–100 is the common production compromise. Tune pool size on the golden set, not by feel.
- **Rerankers can hurt:** in the cited homelab measurement, two of four rerankers scored _below_ plain hybrid retrieval on that corpus, and the largest was both slowest and worse. The lesson is procedural: a reranker is adopted per-corpus on golden-set evidence, never on leaderboard standing alone.
- **Blend, don't replace (optional):** some stacks min-max-normalise reranker scores within the pool and blend with the fused score rather than taking the reranker's order wholesale — protects against reranker pathologies on out-of-distribution queries.

**Relevance to Atlas:** Atlas's pipeline (per the parent survey) fuses FTS + vector; a cross-encoder stage over the fused top-N is the single highest-expected-value quality addition, and fastembed-rs's v2-m3 support makes it a same-language, in-process change. The golden set decides pool size and whether blending beats pure rerank order.

---

## 4. Query planning & understanding

### What it is

Query planning is everything that happens between the raw user/agent query and retrieval: deciding _what_ to search, _where_, and _in what form_. Production systems structure it as a small planner with a fast classification step and conditional transforms — not as "always run every trick," because each transform costs an LLM call and latency.

### The component techniques

- **Conversational rewriting** — resolve pronouns/ellipsis against session history into a standalone query. Near-universal in chat-shaped interfaces; for Atlas, this is what makes multi-turn fleet/ID agent sessions searchable ("that file from before" → a real query).
- **Query expansion** — add synonyms/related terms (LLM-generated, or pseudo-relevance feedback from a first retrieval pass). Helps lexical legs most; dense legs already model some synonymy.
- **Multi-query / RAG-Fusion** — generate 3–5 query variants, retrieve for each, merge with RRF. Directly reuses Atlas's existing fusion machinery on the query side.
- **HyDE (Hypothetical Document Embeddings)** — Gao et al. 2022 (arXiv:2212.10496): an LLM writes a hypothetical _answer document_; embed that instead of the query, retrieve real documents near it, discard the fake. Fixes query–document asymmetry for short/vague queries. Cost: one generation + the risk that a hallucinated hypothesis drags retrieval off-topic; gate it to underspecified queries.
- **Decomposition** — split a multi-part/multi-hop query into sub-queries, retrieve per sub-query, merge. The only way conjunctive questions ("X and how it interacts with Y") retrieve both halves.
- **Step-back prompting** — add a higher-level conceptual query alongside the specific one; pairs with decomposition for analytical questions.
- **Routing** — classify the query and send it to the right index/collection/filter set: by source (code vs notes vs sessions), by intent (exact-identifier lookup → BM25-heavy; conceptual → dense-heavy), by sensitivity tier. In multi-collection systems this is the highest impact-per-cost component: a fast classifier (<10 ms, small model or rules) gates everything downstream.
- **Structured filter extraction** — parse dates, sources, authors, types out of natural language into metadata filters (§2) instead of leaving them as embedding fodder: "the deploy doc from October" → `source=… AND created_at∈October` + a much shorter semantic query.
- **Classification / intent detection** — the planner's switchboard: factoid → direct retrieval; ambiguous/short → HyDE or expansion; multi-part → decomposition; conversational → rewrite first; non-retrieval intents → skip search entirely.

### How production systems structure the planner

The reference shape from the surveyed production patterns:

```
query → classify (fast, <10 ms) → route + select transforms conditionally
      → retrieve per (sub-)query from routed collections
      → fuse (RRF) → rerank → answer/return
```

With two disciplines on top: **adaptive selection** (DMQR-RAG's contribution: choose the rewriting strategy per query rather than stacking all of them — stacking multiplies latency and can compound errors), and **per-strategy evaluation** (each transform is an experiment gated on the golden set, sliced by the query class it targets — HyDE should win on short queries and is allowed to lose, unused, elsewhere).

Sources: [Pre-retrieval optimization survey](https://github.com/mehmetaliyilmaz0/ragfactory/blob/HEAD/research/pre-retrieval-optimization-techniques.md), [Agentic RAG pattern catalog](https://www.neurals.ca/agents/agentic-rag/pattern-catalog/), [Query rewriting chapter](https://www.kunwar.page/chapter/063-query-rewriting-hyde-multi-query-query-decomposition).

**Relevance to Atlas:** Atlas's consumers are increasingly _agents_ (fleet, ID, MCP clients) whose queries are conversational, multi-part, and source-specific — exactly the traffic a planner exists for. The minimal viable planner for Atlas is routing + filter extraction + conversational rewriting (all cheap, no generation); HyDE and decomposition are second-wave, golden-set-gated additions for the query classes that measurably fail today.

---

## 5. Recommender systems for a personal corpus

### What changes in a single-user system

Classic collaborative filtering ("users who liked X also liked Y") needs many users. A personal corpus has **one user but rich behavioural exhaust**: search sessions, opens/clicks, dwell, saves, re-finds, and the documents' own structure (provenance, co-occurrence in sessions, graph links). The realistic recommender is therefore **content-based + session-based + recency-aware**, with collaborative machinery repurposed: "items that co-occur" means items that appear together in _this user's sessions, projects, and time windows_, not across users.

### The signal inventory (all available to Atlas)

- **Content similarity** — the embedding index itself: user profile = weighted centroid of embeddings of documents the user opened/saved/searched-for, weights decaying with age. This is "more like what I've been working on" with zero new infrastructure.
- **Session co-occurrence** — item-to-item collaborative filtering over sessions: documents opened or cited in the same session/task are related _for this user_, even when their text isn't similar (a spec and the incident log it caused). Precompute item→item neighbours offline from session logs; this is the strongest single-user analogue of Amazon's item-to-item CF.
- **Recency & frequency** — trending-in-my-corpus: recent opens, repeated re-finds (a document the user keeps returning to is a hub), and fresh ingest (new documents matching active interests should surface before they are ever searched for).
- **Graph signals** — provenance/occurrence links (Atlas's CAS + occurrence model; ID-graph edges where #6062 lands): neighbours of known-relevant documents, documents sharing rare chunks/entities.

### Architecture: the same three stages as search

Production recommenders and production search have converged on one shape — **candidate generation → ranking → re-ranking** — which means Atlas's recommender can reuse its search stack:

1. **Candidate generation (recall):** union of cheap nominators — embedding kNN against the user-profile vector, item-to-item neighbours of recent items, recency/popularity within active sources, graph neighbours. Each nominator is biased; the union covers the biases.
2. **Ranking (precision):** score candidates with the richest affordable model — for Atlas, the same cross-encoder reranker (§3) with the "query" being a profile/session summary, or a learned model over features (similarity, recency, co-occurrence count, source).
3. **Re-ranking (the surface):** enforce what raw relevance ignores — **diversity** (MMR: penalise near-duplicates of already-picked items), **freshness quotas**, deduplication against already-seen, and exploration slots (a small ε of deliberately novel items, without which the system collapses into a filter bubble of the user's last topic).

### Two-tower models — where they fit and where they don't

A two-tower model trains a user tower and an item tower into a shared embedding space so retrieval is kNN of the user vector. It is the right tool at multi-user scale. For a single user it is usually **overkill as a trained model** — there is one user vector to learn — but the _pattern_ is exactly Atlas's content-based profile above: the "user tower" is a hand-built, decaying weighted average of item embeddings, and it can be upgraded later (learned weights per source/signal) without changing the serving shape. Sequential models (SASRec/BERT4Rec-style, "what's next in this session") become interesting only with long, dense session logs. **[Judgement call, not a sourced benchmark claim.]**

### What it can realistically do for Atlas

- **"More like this"** on any document/session (pure embedding kNN + tier filter) — day one.
- **Related results** alongside search: session-co-occurrence neighbours of the top hits.
- **A "resurface" feed:** documents matching active interests that were recently ingested or recently became relevant (new link, new occurrence), ranked with recency + profile similarity + MMR diversity.
- **Search assistance:** query suggestions and zero-result recovery from profile + co-occurrence ("people — i.e. you, last month — also needed…").
- Evaluation mirrors search: precision@k / NDCG on held-out sessions (predict the held-out opens of a session from its first half), plus coverage/diversity measures so the recommender doesn't just echo search.

Sources: [Recommendation engine architecture](https://gortex.ai/blog/recommendation-engine-architecture), [Recommender systems lesson](https://chaduvuko.com/learn/ai-ml/classical-ml/recommender-systems).

**Relevance to Atlas:** Atlas already logs the raw material (searches over 18k sessions) and already owns every stage of the serving stack; the recommender is primarily a _logging + offline neighbour computation + one blended surface_ project, not a new ML platform. Its outputs also feed back into search as behavioural ranking signals (§1, layer 4) — the two systems should be designed as one loop.

---

## 6. Evaluation — golden sets, metrics, and regression gating

### What it is

Search quality is invisible without measurement: every change in §§1–5 (model swap, fusion change, reranker, planner transform) silently shifts results. The discipline that makes iteration safe is a **frozen golden set + per-stage metrics + a CI gate that blocks regressions**.

### The golden set

- A set of real queries, each labelled with its relevant documents/chunks and **graded relevance** (e.g. 0–3: irrelevant / somewhat / relevant / exact). 50–200 query–document pairs is the commonly cited starting size; quality of labels beats quantity.
- Build it from **real production traces** (Atlas: real fleet/ID/user searches), and harvest gold contexts from real retrieval runs — the documented failure mode is a golden set built from hand-picked chunks the live retriever never surfaces, which read 0.91 recall offline against 0.40 in production.
- Include **no-answer probes** (queries with an empty relevant set) to measure noise/threshold behaviour, plus adversarial slices: keyword distractors (same words, different meaning), semantic twins (same meaning, different words), alias-only matches, and long-document probes (relevant fact buried at the end).
- Grow it by **error analysis**: open-code real failures, group into a failure taxonomy, stop at saturation; prefer binary pass/fail judgements where graded labels create annotator drift. Every production failure that gets fixed becomes a golden-set entry — the set is a regression suite, not a benchmark poster.

### Metrics — pick per stage, per failure mode

| Metric          | What it rewards                                                     | Use for                                                                                                            |
| --------------- | ------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------ |
| **Recall@k**    | gold doc anywhere in top-k                                          | First-stage retrieval ceiling — a miss here is unrecoverable downstream. The must-pass gate for retriever changes. |
| **Hit rate@k**  | at least one relevant in top-k (binary per query)                   | Single-answer corpora; simplest starting metric.                                                                   |
| **MRR**         | rank of the _first_ relevant result (mean of 1/rank)                | Single-right-answer tasks; reranker top-of-list quality.                                                           |
| **nDCG@k**      | graded relevance, position-discounted, normalised to ideal ordering | The workhorse for multi-relevant ranking and reranker comparison.                                                  |
| **Precision@k** | share of top-k that is relevant                                     | Context-noise / token-budget control.                                                                              |

The senior pattern: evaluate **retrieval and ranking separately** (recall@k for stage 1, nDCG/MRR for the reranked order), so a failure localises to a stage instead of grading the pipeline as one black box.

### Offline vs online, and the CI gate

- **Offline** (the gate): frozen golden set, run in CI on _every_ embedding-model, chunker, parser, fusion, reranker, or planner change; fail the build if any gated metric regresses past a set margin against the frozen baseline. Embedding swaps are the highest-risk change class and should never ship on "the new model is better on MTEB" alone.
- **Online** (the check): shadow/canary evaluation on real traffic, watching long-tail behaviour — offline sets over-sample easy queries and can flatter a change that collapses the tail. For a personal system, "online" is lightweight: log real queries + clicks, periodically mine them for failures, fold them into the golden set.
- Tooling shapes worth copying: a small eval runner that boots the _same_ retrieval stack the server uses (no transport, no reimplementation), prints a per-configuration metrics table, and is invoked identically locally and in CI — the Kioku MCP-server eval harness is a worked example at personal-corpus scale.

Sources: [RAG evaluation — metrics & golden sets](https://www.dataaihub.co/learn/retrieval-evaluation), [RAG engineer evaluation notes](https://github.com/landedjobs/rag-engineer-interview-questions/blob/HEAD/content/05-rag-evaluation.md), [Kioku retrieval evaluation](https://kioku.sandovaldavid.com/retrieval-eval.html).

**Relevance to Atlas:** Atlas already has a golden set — the program risk is not its absence but its _coverage and gating_: whether it slices by source and query class (so planner/reranker changes can be judged per class), whether it gates CI for the fleet/ID consumers, and whether real fleet/ID query failures flow back into it. Every other section of this survey should be read as "adopt only where this section's machinery shows a win."

---

## 7. Incremental ingest & indexing

### What it is

Keeping the index in agreement with a corpus that never stops changing, without re-processing what didn't change. Four sub-problems: change detection, dedupe, embedding versioning, and deletion.

### Change detection: CDC over crawl, and the cost ladder

- **Change-data-capture first:** subscribe to the source of record's change feed (database logical replication / WAL, filesystem events, API webhooks, git commits) rather than re-crawling; keep a periodic full reconciliation crawl as the safety net for what CDC missed. This converts ingest from batch-shaped to stream-shaped and is what makes minute-level freshness affordable.
- **The detection ladder** (stop at the first proof of no-change): (1) metadata compare — size/mtime/etag from a listing, no file read; (2) content hash — one streaming SHA-256 pass; identical hash after a touch/resave → update metadata, skip; (3) full process — parse → chunk → embed → upsert. Store per document: `external_id, size, mtime/etag, content_hash, parser_version, chunking_config_version, embedding_model_version, last_processed_at`.
- **Chunk-level hashing** is the big win for large documents: hash each chunk's text and re-embed only changed chunks — a one-paragraph edit to a 200-page manual re-embeds one chunk.
- **Config-driven reprocessing:** if any stored version (parser / chunking / embedding model) ≠ current, the document needs reprocessing _even if its hash matches_ — as a separate, explicit, rate-limited job, never as a surprise inside the incremental path.

### Content-addressed dedupe

Atlas's CAS design is the textbook shape: the content hash _is_ the identity. Exact dedupe falls out (identical normalised text → identical hash → one stored object, many occurrences — the #3125 chunk_occurrences model). For **near-duplicates** (drafts, template variants, format twins like a PDF and its HTML export), exact hashing is blind; the standard ladder is MinHash/LSH over shingles to nominate candidate pairs, confirmed by real similarity (or embedding cosine) before merging — and dedupe _within a scope_: two near-identical documents from different sources/regions may both be legitimately needed, distinguished by metadata.

### Embedding versioning — the model-upgrade pattern

Model upgrades are the one event that genuinely requires re-embedding everything (vectors across models are incomparable; mixing them degrades results silently). The production pattern:

1. Tag **every stored vector with its `model_id`** (+ dimension + chunking version) — Atlas already keys its derived index by content hash + model_id, which is this pattern.
2. Run the new model into a **shadow index** (new table/collection), backfilled incrementally with the dedupe cache suppressing unchanged chunks.
3. **Cut over on an eval result, not on completion:** run the golden set (§6) against both indexes; flip reads when the new index wins or ties with acceptable cost; keep the old index until the flip is proven, then drop it.
4. A **two-tier variant** for freshness during long backfills: a stable base index + a small continuously-updated delta index, queried together and fused, with the delta merged into base on a schedule.

### Sharding, tenancy, and deletion

- **Sharding by source/tenant** maps naturally onto Atlas: per-source (or per-tier) tables/collections in libSQL, routed by the query planner (§4). Benefits: independent re-indexing, tier isolation as a security property, per-source freshness SLAs. Cost: cross-shard queries need federation + fusion (RRF again).
- **Deletion is the hard part:** vector indexes tombstone rather than truly delete; recall degrades and memory doesn't return until compaction. Track the tombstone ratio and compact on it, not on a calendar. In a CAS design, deletion is reference-counted: removing an occurrence must not remove an object other occurrences still reference — and the sensitivity-tier machinery adds a second deletion class (purge-for-policy) that must reach the derived indexes, the FTS index, and any recommender neighbour tables, not just the source store.

Sources: [Change detection ladder](https://www.mnsai.io/learn/06-rag/apply/1-ingestion-parsing/change_detection), [Freshness & incremental re-embedding](https://vfaraji89.github.io/applied-ai-simplified/problems/incremental-re-embedding/), [Embedding pipelines at scale](https://scalablebook.icsgen-ai.org/part-8-case-studies/module-40-case-distributed-llm-agentic/section-40.3.html), [Incremental ingestion guide](https://medium.com/@priyanshigupta1701/stop-re-embedding-everything-a-step-by-step-guide-to-incremental-ingestion-359a447f3ae5).

**Relevance to Atlas:** Atlas's CAS + hash-keyed derived index already implements the strongest parts of this pattern; the planning gaps are (a) CDC-style automatic ingest per source (vs batch runs), with the ladder + reconciliation crawl per connector, (b) a written shadow-index cutover procedure so the next embedding-model evaluation (§1) is routine rather than a project, and (c) tombstone/compaction + purge propagation across FTS, vector, and recommender tables as one designed operation.

---

## 8. Programmatic access — MCP, APIs, and fleet/ID/agent consumers

### MCP for search — the protocol surface

The Model Context Protocol (Anthropic, Nov 2024; now under the Linux Foundation's Agentic AI Foundation as of Dec 2025 **[per secondary sources]**) is JSON-RPC 2.0 with three server primitives:

- **Tools** — model-callable functions with JSON-Schema inputs (`search_docs`, `read_section`, …), annotated (read-only / destructive hints) and, since 2025-06-18, able to return structured output.
- **Resources** — URI-addressed read-only data (`file:///…`, `db://…`) the host app reads/subscribes to; resource templates parameterise them.
- **Prompts** — user-invoked templates surfaced as commands.

Transports: **stdio** (local subprocess — the personal/local-first default) and **Streamable HTTP** (remote; replaced the deprecated HTTP+SSE transport in the 2025-03-26 revision). The **2025-06-18** revision made servers OAuth 2.1 resource servers (RFC 9728 metadata, RFC 8707 resource indicators binding tokens to one server), added elicitation and structured tool output, and dropped JSON-RPC batching. A **2025-11-25** revision and a **2026-07-28** stateless revision (no `initialize` handshake, `server/discover`) are reported by secondary sources — **[verify against modelcontextprotocol.io before designing against them]**; the safe design target today is 2025-06-18 compatibility with no reliance on protocol sessions.

Sources: [MCP (Masst docs)](https://docs.masst.dev/ai/edges/mcp), [MCP term guide](https://www.aiagentichandbook.com/terms/mcp/), [MCP security guide](https://apexlytech.com/blog/model-context-protocol-crm-erp-security).

### Existing search-MCP shapes worth copying

- **The docs-search pattern** (small, production-shaped): four tools — `list_documents`, `search_docs` (query + limit → ranked sections with scores and snippets), `read_section` (by stable id), and optionally `answer_question` (RAG over top-k with citations, using MCP sampling when the client offers it, else returning citation-ready context). Tools return _references + snippets_, not full content; full text is a separate read. This two-step (search → read) keeps agent context windows clean and maps exactly onto Atlas's chunk/occurrence model.
- **Tool-based resource discovery at scale:** enterprise MCP guidance is explicit that hosts should discover via search tools rather than enumerating resources — a corpus of 100k+ chunks must never be a `resources/list`.
- **Security posture from the spec ecosystem:** tool descriptions are untrusted input to the model; tokens are audience-bound and never passed through; a human can deny tool calls. For Atlas, the load-bearing point is §2's: **sensitivity-tier enforcement lives in the retrieval layer behind the tool**, so no MCP client behaviour can widen access.

### REST/gRPC APIs and CLI consumers

MCP is the agent-facing surface, not the only surface. The conventional trio:

- **MCP (stdio + Streamable HTTP)** for LLM hosts/agents (Claude, Cursor, mcpx-wired consumers — Atlas's MCP is already live per the parent survey).
- **REST/JSON** for scripts, services, and the ID/fleet backends: `POST /search` (query, filters, limit, explain flag), `GET /documents/{id}`, `POST /similar`, `POST /recommend`. Versioned, with the response carrying per-result signal provenance (which legs matched, reranker score) behind an `explain` flag — debugging search quality without it is guesswork.
- **CLI** (`fleet search` is the existing consumer shape) as the human + scripting surface over the same API — never a separate implementation.

### How consumers typically integrate

- **Caching:** query-result caching keyed by (normalised query, filters, index version) with short TTLs at the consumer or a shared cache; embedding caching by content hash at the producer (already Atlas's model). Cache invalidation rides the index version from §7's versioning — a cutover bumps the version and stale entries die naturally.
- **Batching:** batch embed at ingest (the embedding service boundary: continuous batching behind one service, not library calls scattered through ingest code); batch search endpoints for eval/recommender jobs.
- **Offline fallback:** for local-first consumers (fleet CLI on the laptop, ID nodes), an embedded replica of the libSQL database is the structural answer — libSQL's embedded-replica mode syncs a local file from a remote sqld/Turso primary, so the same SQL (FTS5 + `F32_BLOB`/`vector_top_k`) runs offline and online with no code change. This is also why the libSQL-native vector path beats the `sqlite-vec` extension path for Atlas's consumers: loadable extensions cannot be used over remote sqld connections, while native vectors behave identically local, self-hosted, and hosted.
- **Capability discipline (Atlas/ID-specific):** MCP's own auth is young and tool hints are advisory. In the ID design language, an Atlas MCP server should sit _behind_ the capability layer — calls arrive holding a capability, tier checks happen server-side in retrieval, and the API/MCP surfaces are views over one retrieval core, not parallel implementations.

Sources: [libSQL vector search walkthrough](https://layerbase.com/blog/libsql-relational-and-vector-database), [Turso native vector search](https://turso.tech/blog/turso-brings-native-vector-search-to-sqlite), [libSQL migration issue (docs-mcp-server #435)](https://github.com/arabold/docs-mcp-server/issues/435), [docs-search MCP example](http://glama.ai/mcp/servers/erickmacielsoto/mcp-server-example).

**Relevance to Atlas:** Atlas already has the hardest piece (a live MCP server wired into mcpx). The planning work is convergence: one retrieval core serving MCP + REST + CLI with identical ranking, index-version-aware caching for fleet/ID consumers, embedded-replica offline search for laptop/ID nodes, and capability/tier enforcement in the core so every surface inherits it.

---

## Cross-cutting takeaways for the planning prompt

1. **The base is fine; the layers are the program.** Arctic Embed 2.0 is competitive per parameter; Atlas's quality roadmap is hybrid completeness (RRF tuning, metadata pre-filtering, a possible learned-sparse leg), a reranker stage, and a query planner — in that order, each golden-set-gated.
2. **One stack, three surfaces.** Search, recommender, and "more like this" are the same candidate→rank→re-rank pipeline with different candidate nominators; MCP, REST, and CLI are the same retrieval core with different transports. Design them as one system or maintain them as three.
3. **Evaluation is the licence to change anything.** Golden-set coverage by source/query class + CI gating is what turns §§1–5 from opinions into a sequence of safe, reversible steps — including the embedding-model question, which should be settled by a shadow-index bake-off on Atlas's own set, not by MTEB.
4. **Ingest is a streaming system with a batch safety net.** Per-source CDC + the metadata→hash→process ladder + chunk-level hashing; model upgrades via shadow index + eval-gated cutover; deletion/purge as one designed operation across CAS, FTS, vector, and recommender tables.
5. **Local-first Rust routes exist for every layer** — fastembed-rs (embeddings incl. BGE-M3 triple output and SPLADE, reranking incl. v2-m3), candle where ONNX Runtime is a liability, libSQL native vectors + FTS5 in the store Atlas already runs. No layer requires a hosted service; hosted rerankers (Cohere/Voyage) are useful only as offline quality ceilings.
6. **Security is a retrieval-layer property.** Sensitivity tiers and (for ID) capabilities must be enforced inside the retrieval core, pre-filtered rather than post-filtered, so that MCP/API/CLI consumers — present and future — inherit them by construction.
