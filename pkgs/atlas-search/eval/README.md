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

## Python reference and v3 golden set (#5670)

The repository also keeps the production Python reference and its
corpus-specific retrieval runner beside the Rust tooling:

- `../atlas_search.py` — copied byte-for-byte from the laptop's
  `~/atlas/turso-atlas/scripts/atlas_search.py`
  (SHA-256 `1b805be87b2d08fd5c5f6ed65f45d1c40528c9acc074adeaf0a5665a4cec8d9f`).
- `eval3.py` — started from the source at
  `~/atlas/turso-atlas/scripts/eval3.py`; this copy updates only the usage
  example to point at the checked-in fixture. Source SHA-256:
  `d121c442566286b6f1ee3327693756bdf0f031f87f00e95ce486582d09d66360`.
- `fixtures/golden-311.json` — the 311-query v3 set copied byte-for-byte
  from `~/atlas/v3/golden-queries-v3.json`
  (SHA-256 `b64bc1905384405c9127c744d3874ffd149a9429b6710b0664c2198505bf291c`).
  Its header records the source corpus size and fingerprint used to build
  the set; the source corpus itself is not checked in here.

From this directory, run the corpus-specific FTS baseline with the laptop
corpus (the runner checks its frozen fingerprint before scoring):

```sh
~/atlas/v3-venv/bin/python eval3.py --mode fts --corpus ~/atlas/v3/corpus.db --queries fixtures/golden-311.json --out /tmp/golden-311-fts.json
```

`eval3.py` is the retrieval/reference runner for this frozen set;
`atlas-eval` above remains the generic offline rank-list scorer. The
synthetic `eval-smoke.json` fixture is not held-out evaluation evidence.

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
