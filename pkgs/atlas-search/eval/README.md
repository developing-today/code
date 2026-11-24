# Atlas retrieval evaluation harness (#5741)

The harness the #5741 test plan builds _first_, before any signal
(SPLADE included) is adopted or rejected: saved rankings from any
signal combination are scored offline against judged qrels, so the
ablation matrix — {dense} / {BM25} / {SPLADE} / {dense+BM25} (today's
design) / {dense+SPLADE} / {dense+BM25+SPLADE} / each ± Jaccard
rescore — is measured, not argued. Two cautions about that matrix
as a record: the SPLADE arms have never been scored — no SPLADE
encoder or postings exist (see "What this does not do"), so every
SPLADE row above is design intent, not a measured signal — and the
golden-311 numbers recorded in this directory come from two
different configurations, named under "The recorded golden-311
rows" below.

- Library: `crates/atlas-core/src/eval.rs` (`atlas_core::eval`)
- Runner: `cargo run -p atlas-core --bin atlas-eval -- <eval.json> <dev|test>`
- Smoke fixture: `fixtures/eval-smoke.json`

## Python reference and v3 golden set (#5670)

The repository also keeps the production Python reference and its
corpus-specific retrieval runner beside the Rust tooling:

- `../atlas_search.py` — copied from the laptop's
  `~/atlas/turso-atlas/scripts/atlas_search.py`
  (source SHA-256 `1b805be87b2d08fd5c5f6ed65f45d1c40528c9acc074adeaf0a5665a4cec8d9f`).
  The in-repo copy has since diverged from that source: concurrent
  hybrid branches (#12900 thread 1), a fail-closed chunk-labels
  fix, a treefmt pass, and the recorded-row cross-reference added
  under #13283.
- `eval3.py` — started from the source at
  `~/atlas/turso-atlas/scripts/eval3.py`; this copy updates only the usage
  example to point at the checked-in fixture. Source SHA-256:
  `d121c442566286b6f1ee3327693756bdf0f031f87f00e95ce486582d09d66360`.
  The in-repo copy now diverges from that source by the three #8738
  fixes (2026-11-17): query embeddings are truncated + renormalized
  under `--trunc` exactly as document embeddings are (fresh and
  cached); a `--qcache` is reused only when its sidecar identity
  (framework + model + ordered query texts) matches, instead of on
  row count alone; and per-category recall@10 applies the same
  ten-rank cutoff as the overall counter when `--k` exceeds 10.
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
  "queries": [{ "id": "q1", "class": "code", "split": "dev", "text": "..." }],
  "qrels": [{ "query": "q1", "doc": "chunk-1", "grade": 2 }],
  "runs": [{ "signal": "dense", "results": { "q1": ["chunk-2", "chunk-1"] } }]
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
- Optional (#8627): a query may carry `forbidden` (doc ids that
  must never be returned for it) and `corpora` (the corpora it is
  scoped to); the document may carry `docs` (`{"id", "corpus"}`
  attribution for provenance checks). See the next-but-one section.

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

## Ceiling/scope negatives and provenance (#8627)

The cross-corpus sets for the Atlas memory integration (design
record `../docs/memory-integration-design-8567.md` §6, slice 5)
must be able to fail a run for breaking _policy_, not just for
ranking badly. Three optional `atlas-eval/v1` fields express that,
all backward compatible — a set that omits them scores and
serializes exactly as before:

- `forbidden` (per query): doc ids that must never appear in a
  ranking for that query — above-ceiling or out-of-scope docs a
  correct system cannot return. A forbidden doc may not also hold
  a positive grade for the query; validation rejects the
  contradiction.
- `corpora` (per query): the corpora the query is scoped to. When
  present, provenance is enforced for its rankings.
- `docs` (per document): `{"id": ..., "corpus": ...}` attribution
  naming each doc's corpus; may cover docs nothing else mentions.

A ranking that surfaces a forbidden doc, a doc from outside the
query's corpus scope, or a doc with no attribution at all records
a **violation** in that signal's report (query, doc, rank, kind,
corpus). The metrics are computed unchanged — the doc still scores
grade 0 — but the signal has _failed_: `select_best_signal`
refuses a report containing violations, and the runner exits
non-zero. A policy regression therefore fails the eval itself,
not just a code review.

**The one evidence command.** A corpus or connector change ships
with before/after output from:

```sh
cargo run -p atlas-core --bin atlas-eval -- <set.json> dev
```

Exit 0 = every signal passed; exit 1 = at least one violation
(the report is still printed, as the evidence). Per-family sets
(OpenClaw, OpenCode, OpenChamber, Muse lanes) with graded qrels
and recorded baselines land on #8627 per family as slices
#8622/#8623/#8625/#8626 stage their corpora; qrels are never
invented for a corpus that is not staged yet.
`fixtures/eval-negatives-smoke.json` is a synthetic set that
exercises the negative/provenance paths end to end (one clean
signal, one leaking signal) — like the smoke fixture below, it is
not evaluation evidence and not a family baseline.

## The smoke fixture is not evidence

`fixtures/eval-smoke.json` is synthetic — hand-written queries,
qrels, and rankings that exercise the harness end to end (it backs
the determinism and selection tests). It is **not** a held-out set,
was never measured against a corpus, and must never be quoted as
evaluation evidence or as support for adopting SPLADE or any other
signal. A real held-out set does now exist: `fixtures/heldout-nearexact-54.json`
(54 verbatim near-exact passages, built 2026-11-17), and a full
gate has run on it — `gate5733.py`, verdict record
`gate-5733-results.md` (FAIL for the Jaccard rescore). That set is
a near-exact-passage set built for the #5733 gate; the broader
held-out collection of real query shapes described in #5741's test
plan, step 1, remains the plan for general query shapes, and this
harness is what will score it when it exists.

## The recorded golden-311 rows

Golden-311 carries two recorded rows that were both long labelled
"production hybrid". They are different configurations, not a
measurement error, and neither should be cited without its
configuration:

| Row                           | Configuration                                                                          | Recorded in                                                                                                                       |
| ----------------------------- | -------------------------------------------------------------------------------------- | --------------------------------------------------------------------------------------------------------------------------------- |
| 189 hits@10, MRR 0.3894       | production shape: one-stage hybrid, RRF k=10, pool 100 per branch, full 768 dimensions | `../atlas_search.py` header (2026-10-31 bake-off decision); configuration pinned by `crates/atlas-proto/query-parameters-v1.json` |
| recall@10 183/311, MRR 0.3640 | eval lineage: branch depth 30, RRF k=60, vectors truncated to 384 dimensions           | `gate-5733-results.md` (sanity gate, reproduced exactly)                                                                          |

Three different artefacts in this directory are all called
"golden"; they are not interchangeable:

- `fixtures/golden-311.json` — 311 judged queries; qrels are
  chunk-id judgements against the conversation corpus.
- `fixtures/heldout-nearexact-54.json` — the 54-query held-out
  near-exact set above.
- The overlap experiment's query set (`overlap-report-6039.md`) —
  provisional and self-derived, with span judgements against a
  slice of this repository. Golden-311 cannot serve that
  experiment: its qrels judge chunk ids in a different corpus,
  not spans in the repo slice.

## What this does not do

No signal generation (no SPLADE encoder, no postings table, no
production query-path change), no latency measurement, no adoption
decision. Those are the later #5741 steps, gated on this harness's
numbers.
