# Tutorial: run the golden set and read a gate

Source: [pkgs/atlas-search/crates/atlas-core/src/eval.rs](https://github.com/developing-today/code/blob/main/pkgs/atlas-search/crates/atlas-core/src/eval.rs), [pkgs/atlas-search/crates/atlas-core/src/bin/atlas-eval.rs](https://github.com/developing-today/code/blob/main/pkgs/atlas-search/crates/atlas-core/src/bin/atlas-eval.rs), [pkgs/atlas-search/eval/README.md](https://github.com/developing-today/code/blob/main/pkgs/atlas-search/eval/README.md), [pkgs/atlas-search/eval/eval3.py](https://github.com/developing-today/code/blob/main/pkgs/atlas-search/eval/eval3.py), and [pkgs/atlas-search/eval/gate-5733-results.md](https://github.com/developing-today/code/blob/main/pkgs/atlas-search/eval/gate-5733-results.md) (issues #5741, #5733). Provenance only — this chapter is complete without following it.

This tutorial runs Atlas's offline evaluation twice: once with the `atlas-eval` harness over saved rankings, and once with the recorded Jaccard gate script, whose correct verdict is `FAIL`. A _golden set_ is a fixed, judged query set for a corpus — queries whose relevant documents (the _qrels_, query relevance judgements) are recorded in advance, so a ranking can be scored without asking a human each time. The point of the tutorial is not the numbers themselves. It is the discipline the numbers enforce, stated at the end: a change ships on its eval result, not on being finished. The programme behind this is in [Evaluation: golden sets and gates](../concepts/evaluation.md); the command surface is in [atlas-eval](../reference/atlas-eval.md).

## Before you start: two runners, two jobs

Atlas has two evaluation runners, and confusing them produces meaningless comparisons.

- `atlas-eval` (the #5741 harness) scores _saved rankings_ offline. It generates no rankings, loads no model, opens no database, and makes no network calls. Dense, BM25 (the full-text ranking function), SPLADE (a learned sparse signal), and fused rankings all enter it in the same format, so the ablation matrix — each signal alone and in combination, with and without a rescore — is measured, not argued.
- `eval3.py` (the Python reference runner) _produces_ rankings against a real corpus for the golden-311 set and scores them by gold chunk ids. The gate script in Step 4 is built on its functions. Its hybrid configuration is a bake-off-era configuration (see Step 3), not the shipped service configuration.

Golden-311 itself: 311 queries in `pkgs/atlas-search/eval/fixtures/golden-311.json`, frozen 2026-10-26 against the merged corpus of 101,471 chunks and 18,116 sessions. Its corpus fingerprint is the SHA-256 of the concatenation of `content_hash` values for chunks with `id % 997 = 0`. Each query records its text, a category (for example `exact-fact`), all its gold chunk ids, and its session ids. Questions were written from a short summary of the gold chunk, not mined by checking a hint verbatim first. Treat the set and its corpus as frozen: if the corpus guard below aborts, you are not looking at the corpus the set was judged against, and no score from that run is comparable to any recorded row.

## Step 1 — Run `atlas-eval` on an eval document

The invocation has two positionals and no flags:

```
atlas-eval <eval.json> <dev|test>
```

`--help`, `-h`, and `help` are not recognised; each prints the usage line to stderr and exits 2. An unreadable file, invalid data, or an evaluation failure exits 1. Success prints one `atlas-eval-report/v1` JSON report to stdout and exits 0, byte-identical on every run for the same input.

The input is one `atlas-eval/v1` document:

```json
{ "format": "atlas-eval/v1", "queries": [{ "id": "q1", "class": "code", "split": "dev", "text": "..." }], "qrels": [{ "query": "q1", "doc": "chunk-1", "grade": 2 }], "runs": [{ "signal": "dense", "results": { "q1": ["chunk-2", "chunk-1"] } }] }
```

Grades are unsigned, maximum 30; a document is relevant when its grade is at least 1, and an unjudged document scores 0. Rank order is the whole contract — no raw scores are needed. A run with no entry for a query scores that query as an empty ranking: incomplete runs are penalised, never skipped. Validation rejects a wrong format version, duplicate query ids, qrels or runs naming unknown queries, duplicate (query, doc) qrels, grades above 30, duplicate documents inside one ranking, duplicate result keys inside one run, and duplicate signal names, before any scoring happens.

To score golden-311 with this harness, first produce an eval document whose queries and qrels are golden-311's gold chunk ids (binary gains in the gate's use: a gold id is relevant, everything else is not) and whose `runs` are the saved rankings you want compared — for example the current production hybrid's ranking per query, saved from a retrieval run. Then score the split you are iterating on:

```
atlas-eval golden-311-eval.json dev
```

Score `dev` while iterating. The `test` split is the held-out set: score it once, at decision time. The library's `select_best_signal` function — the supported way to pick a winner — refuses a test-split report outright, because picking parameters against the held-out set destroys what it is held out for.

## Step 2 — Read the report: nDCG@10, Recall@100, MRR@10

The report carries, per signal, means over the queries that have at least one relevant document (reported as `queries_scored`), overall and sliced per query class (`code`, `documentation`, `error`, `concept`, and so on). Each per-query record preserves the run's top-1000 document ids and the query's judged grades, so any number in the report can be recomputed from the report alone.

| Field        | Metric      | How to read it                                                                                                                                                                                                                                                                    |
| ------------ | ----------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `ndcg10`     | nDCG@10     | Normalised discounted cumulative gain over the top 10 ranks. Gains are graded, `2^grade − 1`, discounted by `log2(i + 2)` at 0-based rank `i`, and normalised against the ideal ordering. This is the ranking-quality number: it rewards putting highly relevant documents early. |
| `recall100`  | Recall@100  | The fraction of the query's relevant documents found in the top 100 ranks. This is the retrieval number: a document outside the candidate pool can never be ranked, fused, or rescored into the answer.                                                                           |
| `recall1000` | Recall@1000 | The same fraction over the top 1000 ranks. The gap between Recall@100 and Recall@1000 shows how much relevant material exists deep in the ranking that a wider pool or a reranker could in principle recover.                                                                     |
| `mrr10`      | MRR@10      | Mean reciprocal rank over the top 10: `1 / rank` of the first relevant document, 0 when none appears in the top 10. This is the first-answer number: it moves only when the first relevant result moves.                                                                          |

Read them in that order of causation, not in order of size. If Recall@100 fell, the problem is retrieval — pool depth, the branch signals, the filters — and no ranking change downstream will fix it. If recall held and nDCG@10 or MRR@10 fell, retrieval found the material and ranking buried it. Always read the per-class slices before the overall mean: an overall gain bought by a collapse in one class (for example `error` queries) is not a gain the overall number is entitled to claim, and the class slices are where a planner or signal change is supposed to prove itself.

A worked reading, with invented toy numbers: signal A reports `ndcg10: 0.410`, `recall100: 0.82`, `mrr10: 0.390`; signal B reports `ndcg10: 0.425`, `recall100: 0.82`, `mrr10: 0.401`, with no class slice down. Same recall, better ordering — B is a ranking improvement over A on this split. If B instead reported `recall100: 0.79`, the higher nDCG would not rescue it: B retrieves less, and on queries where the relevant document never enters the pool its ranking quality is irrelevant.

## Step 3 — Compare against the recorded production rows

Golden-311 has two recorded production-hybrid rows. They are both real, they differ, and they differ because their configurations differ. A comparison that does not name its configuration is invalid.

| Row                           | Configuration                                                                                                                | Recorded result               |
| ----------------------------- | ---------------------------------------------------------------------------------------------------------------------------- | ----------------------------- |
| Bake-off best row, 2026-10-31 | One-stage hybrid, reciprocal rank fusion (RRF) at `k = 10`, pool 100 per branch — the configuration the shipped service uses | 189 hits@10 / MRR 0.3894      |
| Gate sanity row (see Step 4)  | Python reference hybrid at branch depth 30, RRF `k = 60`, query vectors truncated to 384 dimensions                          | Recall@10 183/311, MRR 0.3640 |

Three naming traps sit inside that table, and the sources themselves fall into them in places:

- **RRF `k` is 10 in the shipped service and 60 in the Python eval lineage.** `eval3.py`'s hybrid fuses top-30 branches at `k = 60`. A score produced by the eval lineage is not a score of the production configuration, even when a document calls it the production hybrid.
- **Hits@10 / Recall@10 over 311 is a count, not one of the harness metrics.** 189 hits@10 means 189 of the 311 queries had a gold chunk in the top 10. It is comparable to the gate row's 183/311, and to nothing in an `atlas-eval` report until you convert deliberately.
- **Truncation differs.** The shipped default is full width (768 dimensions); the gate row is a 384-dimension number. Dimension count is part of the configuration.

So the comparison procedure is: state your run's mode, pool or branch depth, RRF `k`, and truncation; pick the recorded row with the matching configuration; compare like with like. If no recorded row matches your configuration, your run establishes a new row — record it with its configuration, and do not claim it beats or matches production.

## Step 4 — Run the gate-5733 script and read its `FAIL`

The gate script `pkgs/atlas-search/eval/gate5733.py` is the held-out gate for the Jaccard rescore (#5733): a third signal that compares token-shingle overlap (sets of 5-token shingles, hashed and compared by Jaccard similarity, the size of the intersection over the size of the union) between the query and each fused candidate, applied only as a rescore over the fused candidate union — never as a retrieval scorer. The gate asks whether turning that rescore on meets two bars agreed _before_ measuring.

Its shape, in outline:

```
python3 pkgs/atlas-search/eval/gate5733.py
# constants in the script: corpus ~/atlas/v3/corpus.db,
# vectors ~/atlas/v3/vec-snowflake.db, table vec_snowflake_m_v2_768_f32,
# query cache ~/atlas/v3/qcache-snowflake.npy,
# rescore depth 100, near-exact threshold 0.60,
# output /tmp/gate5733-results.json; recorded runtime ~19 min on the laptop
```

Do not skip the sanity gate inside it. Before any gate number was trusted, the script's base pipeline — the Python reference hybrid: Snowflake truncated-384 cosine plus FTS BM25, RRF `k = 60`, using `eval3.py`'s own functions — was run at branch depth 30 and reproduced the recorded production-hybrid numbers for golden-311 _exactly_ (Recall@10 183/311, MRR 0.3640, the second row in Step 3). A gate whose base pipeline cannot reproduce a known row measures nothing. The rescore implementation in the script is likewise a port of the shipped Rust `rescore_fused`, pinned against the Rust test vectors before measuring.

The two test sets are the general golden-311 and a held-out near-exact set, `eval/fixtures/heldout-nearexact-54.json`: 54 queries, each a verbatim 30-token passage sampled deterministically from a corpus chunk, with that chunk as its single gold. The recorded run (2026-11-17) disclosed its corpus caveat in its own results file: the local corpus copy held 101,339 of the frozen 101,471 chunks, with 132 chunk ids absent including one gold chunk — every golden-311 query kept other gold chunks present, so all 311 scored, but a future rerun should use a bit-exact frozen corpus. Copy that habit: a gate run that does not disclose its corpus state has not finished reporting.

The recorded verdict table:

| Set             | Metric  | Rescore off | Rescore on | Bar                     | Result             |
| --------------- | ------- | ----------- | ---------- | ----------------------- | ------------------ |
| Near-exact (54) | MRR@10  | 0.5635      | 0.5997     | Gain at least +0.10     | +0.0361 — **FAIL** |
| General (311)   | nDCG@10 | 0.4097      | 0.4080     | Regression at most 0.01 | 0.0017 — pass      |

Read the verdict correctly:

- The gate's verdict is the conjunction of its bars. One bar passed and one failed, so the gate **failed**. The correct summary is not "the rescore helps a bit" and not "it passed on the general set". It is: `FAIL` — the near-exact bar was not met.
- The consequence was mechanical, not a matter of taste: `jaccard_depth` stays `0` in the versioned query parameters, which is off, and no production ranking change was made. The shipped default still records that outcome today.
- The diagnostics explain the failure and are part of the verdict: the near-exact tier (Jaccard at least 0.60) fired in only 4 of 54 near-exact queries and 0 of 311 general queries; the gold chunk was already in the top-100 candidate union for 50 of 54 near-exact queries and in the top 10 for 43 both ways; per-query MRR improved for 7 queries and worsened for none. The standard blend did quiet good, but a 30-token passage compared against a long chunk's full shingle set is capped far below 0.60 by set-size geometry alone — the signal as designed cannot deliver a +0.10 gain at that threshold on real chunk lengths. A failed gate with a diagnosed reason is a successful experiment; shipping anyway would have been the failure.
- The verdict is about raw set-overlap geometry, not about the lexical signal family. It neither validates nor undermines SPLADE, whose adoption rests on its own ablation through the #5741 harness — and no SPLADE gate has run: there is no SPLADE encoder, postings table, or run in the repo to score.

If you rerun the gate, expect the same discipline: reproduce the sanity row first, keep depth 100 and threshold 0.60 unless the change under test is exactly one of those, and report both bars with their arithmetic (`0.5997 − 0.5635 = 0.0361`, against a bar of `0.10`) rather than a bare pass or fail.

## Step 5 — Apply the shipping rule

The rule this tutorial exists to teach, in the programme's own terms: a model, chunker, fusion, reranker, or planner change makes its **cutover on eval result, not on completion**.

- Finished code is not evidence. "The implementation is done" is a statement about work, not about quality, and it carries no weight in a shipping decision.
- The evidence is a gate run: a named golden set, a named configuration, pre-agreed bars, a sanity reproduction of a recorded row, and a disclosed corpus state. Bars are set before measuring; a bar moved after seeing the numbers is not a bar.
- A `FAIL` verdict has a mechanical consequence — the feature stays off, the default stays unchanged, the change does not ship — as `jaccard_depth: 0` records for #5733. Filing the diagnosis and moving on is the correct outcome of a failed gate.
- A `PASS` is scoped to what was measured: that signal, that configuration, that corpus, those query classes. It is not a general claim about the technique, and it does not transfer to a different model, threshold, or corpus without a new run.
- The held-out discipline is part of the rule: iterate on `dev`, score `test` once at decision time, and never tune against the set you will cite as proof.

When you write up any Atlas ranking change, write the gate table first — set, metric, off, on, bar, result — and let the shipping sentence be whatever that table forces it to be. For the reference details of the runner itself, see [atlas-eval](../reference/atlas-eval.md).
