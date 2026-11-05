# Atlas retrieval evaluation harness (#5741)

The harness the #5741 test plan builds *first*, before any signal
(SPLADE included) is adopted or rejected: saved rankings from any
signal combination are scored offline against judged qrels, so the
ablation matrix — {dense} / {BM25} / {SPLADE} / {dense+BM25} (today's
design) / {dense+SPLADE} / {dense+BM25+SPLADE} / each ± Jaccard
rescore — is measured, not argued.

- Library: `crates/atlas-core/src/eval.rs` (`atlas_core::eval`)
- Runner: `cargo run -p atlas-core --bin atlas-eval -- <eval.json> <dev|test>`
- Smoke fixture: `fixtures/eval-smoke.json`

## Input format `atlas-eval/v1`

```json
{
  "format": "atlas-eval/v1",
  "queries": [{"id": "q1", "class": "code", "split": "dev", "text": "..."}],
  "qrels":   [{"query": "q1", "doc": "chunk-1", "grade": 2}],
  "runs":    [{"signal": "dense", "results": {"q1": ["chunk-2", "chunk-1"]}}]
}
```

- `class` — query class (`code`, `documentation`, `error`,
  `concept`, …); reports carry per-class summaries because the
  ablation is only meaningful per class.
- `split` — `dev` or `test` (below).
- Grades are unsigned; a doc is relevant iff grade ≥ 1. Unjudged
  docs in a ranking score grade 0. A run with no entry for a query
  scores that query as an empty ranking — incomplete runs are
  penalized, never skipped.
- Rank order is the whole contract for a run; raw scores are not
  needed by any metric here, which is what lets dense, BM25, SPLADE,
  and fused rankings share one format.
- Validation rejects: wrong format version, duplicate query ids,
  qrels or runs naming unknown queries, duplicate (query, doc)
  qrels, duplicate docs inside one ranking, duplicate signal names.

## Metrics

Per query: **nDCG@10** (graded gains `2^grade − 1`, log2 discount),
**Recall@100**, **Recall@1000**, **MRR@10**. Aggregates are means
over the queries with at least one relevant doc (`queries_scored`),
overall and per query class; each per-query record preserves the
run's top-100 doc ids so any number in a report can be recomputed
from the report alone. Reports are deterministic — sorted signals,
queries, and classes, floats rounded to 6dp (the crate's Python
parity convention) — so the same input serializes byte-identically
on every run.

## Split discipline (the held-out rule)

The dev split is for iteration and parameter selection
(`select_best_signal`). The test split is the held-out set: score it
**once**, at decision time. `select_best_signal` refuses a
test-split report outright, so no helper in this harness can
silently tune on held-out qrels; a test report exists to be read,
not selected from.

## The smoke fixture is not evidence

`fixtures/eval-smoke.json` is synthetic — hand-written queries,
qrels, and rankings that exercise the harness end to end (it backs
the determinism and selection tests). It is **not** a held-out set,
was never measured against a corpus, and must never be quoted as
evaluation evidence or as support for adopting SPLADE or any other
signal. The real held-out set (real query shapes against our
corpora, with judged chunks) is collected under #5741's test plan,
step 1; this harness is what will score it when it exists.

## What this does not do

No signal generation (no SPLADE encoder, no postings table, no
production query-path change), no latency measurement, no adoption
decision. Those are the later #5741 steps, gated on this harness's
numbers.
