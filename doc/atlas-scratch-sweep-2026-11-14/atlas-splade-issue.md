# Atlas search: SPLADE investigation — third retrieval signal, eval harness, and query-adaptive signal weighting

Promoted from a paragraph in #5733 to its own issue at Drew's direction (2026-11-04: "file SPLADE so the idea isn't lost"). Context: hybrid design = dense cosine scan + FTS/BM25 fused (RRF), Jaccard at ingest/promotion (dedup) and rescore (#5733), scan-over-shards on TursoDB (#5669 restore). This issue owns the SPLADE question end to end.

## Drew's framing (2026-11-04) — the constraints that shape this

- **Ingest time is not a problem** if results improve. The expensive-at-ingest objection is retired.
- **The bar is query time "sameish"**: SPLADE's query-side cost is one small-model encoding of the query plus an inverted-index lookup (BM25-class speed). If total query latency stays in the current band, the investigation is worth it.
- Open questions this issue must answer with measurements, not opinion: run BM25 _and_ SPLADE both? Does the full stack (dense + BM25 + SPLADE, Jaccard at both ends) actually score higher on _our_ corpora?

## Does SPLADE fit SQLite natively? (Yes — structurally.)

SPLADE retrieval **is** inverted-index retrieval: the model emits a sparse, vocabulary-sized vector per text (~30k dims, ~100 nonzero, with _learned_ term weights and _expanded_ terms not literally present). Scoring = Σ over shared terms of w_query(t) · w_doc(t) — a postings lookup and a sum.

- **Implementation A (portable): a postings table** `(term_id, chunk_id, weight)` in the same shard DB as the chunks. Pure SQL join + aggregate; works in libSQL/TursoDB and even D1. This is the same shape FTS5 already maintains internally — SPLADE is, in effect, an FTS index whose term weights come from a model instead of term statistics.
- **Implementation B (native, later):** TursoDB's sparse-vector support (its experimental sparse-IVF index is aimed at exactly this type). Not required for v1; A is the boring, shard-friendly route.

## Test plan — the eval harness comes first

No signal earns a permanent place on intuition. Build the harness, then ablate:

1. **Held-out eval set** from real query shapes against our corpora: code questions, documentation questions, pasted errors, concept questions — with judged relevant chunks. (Reuse the parity/eval methodology from the 2026-11-04 spike.)
2. **Ablation matrix** over signal combinations: {dense} / {BM25} / {SPLADE} / {dense+BM25} (today's design) / {dense+SPLADE} / {dense+BM25+SPLADE} / each ± Jaccard rescore. Metrics: recall@10, MRR, per query class.
3. **Latency budget**: p50/p95 per-shard and fan-out totals for each combination; the SPLADE configs must stay in the current query-time band (Drew's "sameish" bar). The query encoder's latency counts — measure it where queries will actually be served.
4. **Decision rule**: a signal stays only if removing it measurably hurts the fused result. Correlated signals (BM25 and SPLADE are both lexical) may not both earn their keep — let the ablation say.

## Query-adaptive weighting (Drew, 2026-11-04)

Fixed fusion weights assume every query is alike. They're not:

- **Explicit hint wins**: a caller-supplied target (code vs documentation vs all) sets the weighting profile directly.
- **Rules second**: cheap shape detectors — identifiers / paths / extensions / stack-trace shape → code or error profile (lexical signals up); plain prose question → documentation profile (dense up).
- **Classifier last**: when no hint and no rule fires, classify the query — the Jev classifier already in the façade stack is the natural home (a query-class task beside its intent work); a small LM is acceptable given the cost lands on ambiguous queries only.
- **Hard rule: weights are floored, never zeroed.** A misclassified query degrades gracefully to slightly-wrong weights, never to a missing signal.
- Log (query class, weights used) per query so the eval can score per-class quality and tune the profiles from evidence.

Expected profiles (to be validated, not assumed): code → BM25/SPLADE high, dense medium; documentation → dense high, lexical medium; error paste → exact/Jaccard features high, dense low.

## Query encoding + the cache tie-in

- The query SPLADE encoding is a pure function of (text, model) — exactly like a dense query embedding. It therefore belongs in the **query-embed cache from #5668** beside the dense one: cache both encodings under the same key discipline, and repeat/phrased-alike queries skip both inferences.
- Encoder placement follows wherever queries are served (laptop today; possibly Workers later per the read-distribution direction on #5681). The model is small (BERT-base class); CPU inference is tens of ms — inside the "sameish" budget, and cache hits erase it.

## Success bar

Adopt SPLADE (and any weighting profile) only on measured improvement over the shipped dense+BM25 hybrid on our eval set — recall@10 / MRR up by a margin that justifies a permanent query-time inference dependency, at query latency within the current band. Otherwise the finding "SPLADE doesn't earn it here" is itself the deliverable, recorded with its numbers.
