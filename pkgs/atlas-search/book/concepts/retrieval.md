# Retrieval: hybrid full-text and vector search

Source: written from `pkgs/atlas-search/crates/atlas-core/src/search.rs`, `pkgs/atlas-search/crates/atlas-core/src/fusion.rs`, `pkgs/atlas-search/crates/atlas-core/src/filters.rs`, `pkgs/atlas-search/crates/atlas-core/src/turso.rs`, `pkgs/atlas-search/crates/atlas-core/src/shingles.rs`, `pkgs/atlas-search/crates/atlas-proto/query-parameters-v1.json`, and `pkgs/atlas-search/eval/gate-5733-results.md` under https://github.com/developing-today/code/blob/main/pkgs/atlas-search/ — provenance only; this chapter is complete without following it.

Retrieval is the act of turning a query string into a ranked list of chunks. Atlas retrieval is hybrid: two independent branches each produce a candidate pool for the same query, and a fusion step merges the pools into one ranking. This chapter states the shipped behaviour — the constants, the order of operations, and the parts of the parameter surface that are accepted but do nothing yet. The embedding branch's model contract is covered in [Embeddings: Snowflake Arctic and the parity contract](embeddings.md); how results are judged is covered in [Evaluation: golden sets and gates](evaluation.md).

## The two branches

The full-text branch searches `chunks_fts`, an FTS5 virtual table over chunk text — FTS5 is SQLite's full-text index, and its ranking function is BM25, the standard term-weighting score that rewards rare matching terms and penalises very long documents. The branch's statement joins chunks to their messages, sessions, and session labels, applies the filter and ceiling clauses described below, orders by `bm25(chunks_fts)`, and takes the pool.

Before that statement runs, the query string is turned into an FTS match expression by a heuristic builder. The rule, exactly as shipped in Rust: lowercase the query, split on anything that is not a lowercase letter or digit, keep words longer than 3 characters, and also keep short tokens of at least 2 characters that contain a digit — tokens like `r2` or `s3`, which are short but discriminating. Kept tokens are OR-joined as quoted phrases. If nothing is kept, there is no FTS expression. In full-text-only mode that is fatal, with a message telling the caller to try vector mode; in hybrid mode the search proceeds on the vector branch alone.

The vector branch embeds the query — prompt, truncation, and renormalisation as [Embeddings: Snowflake Arctic and the parity contract](embeddings.md) describes — and ranks chunks by cosine distance between the query vector and each stored chunk vector: smaller distance is nearer meaning. Its statement shape depends on the vector strategy, below.

The search mode selects the branches: `hybrid` (the default, both branches), `fts` (full-text only), or `vector` (vector only). The embedder is lazy: a full-text-only search never loads the model.

## Filter before rank

Filters and the sensitivity ceiling are applied inside each branch's SQL, before any ranking — never as a post-filter over a ranked list. This ordering is a security property, not an optimisation. A chunk above the caller's ceiling does not merely get removed from the results; it never enters the candidate pool, so it cannot displace an allowed chunk from the pool, leak its existence through a rank gap, or influence fusion. The fixture corpus pins the point: a chunk carrying the query's exact vector and exact terms must be absent at the default ceiling, not down-ranked, and present at the top when the ceiling is raised to include it.

The filter dimensions are kind, topic, project, and client (each a comma-separated list on the CLI), plus a date range over the session's creation date: `--since` is inclusive, and `--until` is inclusive by being implemented as an exclusive bound one day later. The sensitivity ceiling is a rung on the ladder `shareable`, `personal`, `local-only`, `secret-bearing`, in that order; the default grant is `personal`, and a chunk passes only when its effective sensitivity — its own recorded sensitivity, falling back to its session label's — is at or below the ceiling. A chunk with no recorded sensitivity never passes. The two branches construct this filtering differently in detail — the vector scan branch carries the session-level conditions as an allowed-id subquery rather than the full join, for cost — but the admitted set is the same set by construction.

A second, destination-side narrowing happens after fusion, when chunk details are resolved: the disclosure context of the surface the results are going to intersects its own ceiling with the caller's, and only the permitted set is rendered in any output shape. That mechanism belongs to [Sensitivity tiers and client ceilings](sensitivity.md); the retrieval point is that branch filtering is the first gate, not the only one.

## Fusion: reciprocal rank fusion with the shipped constants

Each branch returns a pool of candidates: by default `pool = 100` per branch. Fusion is reciprocal rank fusion (RRF), which scores a chunk by its ranks, ignoring the branches' incompatible raw scores — a BM25 score and a cosine distance cannot be added, but ranks can. The shipped formula, per branch, is:

`score += weight / (rrf_k + rank)`, with rank 1-based.

The shipped constants, from the versioned contract file `query-parameters-v1.json` that every default is read from, are: **`rrf_k = 10`**, **`fts_weight = 1.0`**, **`vector_weight = 1.0`**, pool 100, and final result count `k = 10`. Three worked decompositions are pinned in the fusion tests: a chunk ranked 1 in both branches scores 1/11 + 1/11 = 0.181818; FTS rank 2 with vector rank 1 scores 1/12 + 1/11 = 0.174242; a single-branch rank 1 scores 1/11 = 0.090909. The fused list is ordered by descending score, then by best (smallest) rank in any branch, then by chunk id, so the ordering is total and deterministic. Fusion also retains the per-chunk vector distance for every vector-pool chunk, not only the survivors, for later stages and traces.

One lineage warning, because the numbers recur in older records: the Python evaluation lineage — `eval3.py` and the scripts it was copied from — fuses at **k=60**, over branch pools of 30. That is the bake-off-era configuration in which several recorded golden-set numbers were measured. It is not the shipped configuration. Where this book states an RRF constant, it states the shipped behaviour, k=10; evaluation records that used k=60 are labelled with their own configuration in [Evaluation: golden sets and gates](evaluation.md), and comparing a k=60 number against a k=10 number as if they were one system is an error the records themselves invite and this chapter declines.

Every parameter above is settable per query through the versioned parameter contract — `--rrf_k`, `--fts_weight`, `--vector_weight`, `--shard_top_k`, `--global_top_k` and their siblings on the surfaces that expose them — with validation before any backend work: pool must be between 1 and 400, weights must be finite and non-negative and not both zero, and `rrf_k` must be between 1 and 10000.

## Vector strategies

The vector branch has three strategies, selected by `--vector-backend`. They differ in how candidate distances are obtained, not in what they return: a pool of chunk ids with exact cosine distances, already filter-admitted.

- **Scan** — the default, and the current production path. An exact cosine computation over the whole allowed set: `vector_distance_cos` over the hosted table, or an in-process scan over the loaded vector matrix on the local backend. Exact means exact — no approximation error is possible — and the price is a full pass, about 4 seconds at corpus scale for an unfiltered query, and more when session filters force the expensive form of the query. Scan is the default because the stored vectors are plain blobs without the index the other strategies need, as [Embeddings: Snowflake Arctic and the parity contract](embeddings.md) explains.
- **IndexFixed** — one fixed-depth lookup against the libSQL vector index: a single `vector_top_k` call at pool depth against the index named from the vector table, then the survivors are filtered to the allowed-id set, then exact distances are computed for the survivors. One lookup is the whole strategy; if filtering discards most of the depth, the pool comes back short.
- **IndexFill** — the same index lookup with escalating depth until the pool fills after filtering: the depth starts at the pool size and doubles while it is below 3200, then stops at 3200. For a pool of 3 the depth sequence is 3, 6, 12, 24, 48, 96, 192, 384, 768, 1536, 3072, 3200, pinned in a test. IndexFill pays repeated lookups to avoid IndexFixed's short-pool failure.

Both index strategies are fatal on a non-Turso backend — index modes need the Turso backend — and which strategy should become the default is a decision reserved for A/B evidence at the cutover that introduced them, not a settled fact. Scan remains the default in code.

A related composition exists behind an experimental, hidden flag: the vector branch can be served from the core scan shards, each shard scanned exactly, per-shard top-k merged by distance — exact overall, because the global top-k is contained in the union of the per-shard top-ks. Full-text, details, and expansion stay on the wrapped backend in that composition, because the TursoDB shards carry no `chunks_fts`; and a query carrying session-level filters stays on the monolith for both branches, with a diagnostic note, because shards carry no sessions or labels to filter on.

## The optional Jaccard rescore — off by default

A third signal exists in the code, at rescore only: after fusion, the fused candidate union can be rescored by Jaccard similarity — the size of the overlap of two token-shingle sets divided by the size of their union — between the query and each candidate chunk. A shingle here is a run of 5 consecutive normalised tokens; query and chunk sides share one normaliser, versioned, and sets from different normaliser versions never compare. The rescore has two intended effects: a near-exact boost, promoting a candidate whose overlap passes the near-exact threshold (0.60 by default) decisively above every non-near-exact candidate, and a tie-break blend for the rest, `final = 0.85 * rrf_norm + 0.15 * J` at standard tier. It is bounded by depth: only the top `jaccard_depth` fused entries are considered, and shingle sets are computed from chunk text at query time in the shipped slice.

It ships off: `jaccard_depth = 0` in the contract defaults, and 0 means no rescore runs. That default is a measured verdict, not an oversight. The held-out gate for this signal, gate 5733, ran on 2026-11-17 against a 54-query near-exact set and the 311-query general set, with the rescore pinned against the Rust implementation's own test vectors before any number was trusted. The near-exact bar was a gain of at least +0.10 in MRR@10; the measured gain was +0.0361, from 0.5635 to 0.5997 — a fail. The general-set bar was met (nDCG@10 regressed by 0.0017 against an allowed 0.01), but the signal as designed cannot deliver its headline effect: against a long chunk's full shingle set, a short passage's Jaccard score is capped far below the 0.60 threshold by set-size geometry alone, and the near-exact tier fired in only 4 of the 54 near-exact queries. So the code stays, the flag stays, and the default stays 0. Turning it on is a per-query choice a caller can make knowingly; making it the default requires a redesigned signal and a new gate.

## Accepted but unimplemented: the planner surface

The versioned parameter contract carries four controls that the implementation refuses to run. Setting any of them is a validation error, before any backend work, with a "not implemented yet" message:

- `--planner_enabled` — a query planner/rewriter stage.
- `--spell_enabled` — spell correction of the query.
- `--query_class_hint` — a caller-supplied query-class hint, of any value.
- `--variant rewritten` or `--variant both` — submitting a rewritten form of the query, or both forms. Only the shipped default, `raw`, runs.

These are not hidden gaps; they are a contract surface defined ahead of its implementation, carried in every request and every cache key so that landing the stages does not change the wire shape. The work is tracked as #5773 (the query-understanding front end: spell correction and an LLM query planner/rewriter as toggleable stages, with dual submission and provenance) and #5774 (the parameter surface itself). Until those land, the honest statement is the one the validator makes: the flags are accepted by the parser, representable in the contract, and rejected by validation. A related switch, `--raw`, is implemented: it marks the query as verbatim, with planner and spell stages and expansion disabled — and combining it with a non-zero `--expand` is likewise a validation error.

## Output shapes: grouping, expansion, hydration

The fused ranking is rendered in several shapes, selected per query:

- **Flat** — the default: the top `k` chunks in fused order, each with its provenance and, when requested with `--with-message`, its parent message text, truncated at 50,000 characters.
- **Grouped by session** — `--group-by-session` rolls chunk hits up to their sessions: a session's score is its best chunk's score, with a hit count attached, and `k` then counts sessions, not chunks.
- **Expansion** — `--expand N` attaches up to N neighbouring chunks from the same message around each hit, in chunk order. The ceiling is 100 (`MAX_EXPAND_NEIGHBORS`); asking for more is fatal. Expansion is context for reading a hit, not additional ranking.
- **Hydration** — `--hydrate N` replaces the top N results with their full chunk and message content fetched from the content store. Hydration is not expansion: expansion walks neighbours in the corpus database, hydration fetches complete content from the tier content stores, and the two flags keep their separate meanings and limits (hydration is capped at 1000, and requesting it together with grouped output in the default raw mode is fatal, because grouped output has no flat top-N to hydrate).
- **Raw rows** — an explicit raw mode can render the flat fused rows ungrouped, or the pre-fusion branch rows exactly as each branch produced them, each row tagged with its branch. No fusion is applied in the branch-rows form; it exists for diagnosis, and it is distinct from `--raw`, which concerns the query text, not the output.

Every executed search, in every shape, writes a query-trace record before its response is released, and a failure to write the trace fails the search. The served set recorded in the trace is collected from the rendered envelope, so the disclosure filtering described above is exactly what the trace attests to — a property [Evaluation: golden sets and gates](evaluation.md) and the statistics built on traces rely on.
