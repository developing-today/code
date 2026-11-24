# atlas-eval

Source: [pkgs/atlas-search/crates/atlas-core/src/bin/atlas-eval.rs](https://github.com/developing-today/code/blob/main/pkgs/atlas-search/crates/atlas-core/src/bin/atlas-eval.rs) and [pkgs/atlas-search/crates/atlas-core/src/eval.rs](https://github.com/developing-today/code/blob/main/pkgs/atlas-search/crates/atlas-core/src/eval.rs) (issue #5741).

`atlas-eval` is a single-verb binary with no subcommands: the offline ranking-evaluation runner for Atlas. It scores saved rankings — lists of document ids produced earlier by any retrieval signal, against a corpus — against judged relevance data, and prints a report. It generates no rankings itself: it loads no embedding model, opens no database, and makes no network calls. The same input therefore produces a byte-identical report on every run.

## Invocation

```
atlas-eval <eval.json> <dev|test>
```

There are no option flags. `--help` / `-h` prints the one-line usage text to stdout and exits 0; `--version` / `-V` prints `atlas-eval 0.1.0`, exit 0. The literal word `help` is not recognised: it falls into the wrong-arity path, prints the usage text to stderr, and exits 2.

| Flag                               | Argument                            | Default  | Meaning                                                                                                                    |
| ---------------------------------- | ----------------------------------- | -------- | -------------------------------------------------------------------------------------------------------------------------- |
| (none)                             | —                                   | —        | This binary takes no `--flag` options beyond `--help` / `--version`.                                                       |
| `&lt;eval.json&gt;` (positional 1) | path to an `atlas-eval/v1` document | required | The evaluation input: queries, qrels, and saved rankings (see below).                                                      |
| `&lt;dev\|test&gt;` (positional 2) | `dev` or `test`                     | required | Which split of the input document to score. Any other value is an error: `atlas-eval: unknown split "…" (want dev\|test)`. |

## Inputs: the golden set, the qrels, and the saved rankings

The input is one JSON document of format `atlas-eval/v1`. A _golden set_ is the judged query set for a corpus: a fixed list of queries, each with human judgements — _qrels_ (query relevance judgements) — naming which corpus documents are relevant to it, and how strongly. The document carries three arrays:

- `queries`: each has an `id`, a `class` (the query class, for example `code`, `documentation`, `error`, or `concept`), a `split` (`dev` or `test`), and the query `text`.
- `qrels`: each names a `query`, a `doc` (a document id, such as a chunk id from the corpus the rankings were produced against), and a `grade`. Grades are unsigned integers up to a maximum of 30 (`MAX_GRADE`); a document counts as relevant when its grade is at least 1, and unjudged documents score grade 0.
- `runs`: each names a `signal` (for example `dense`, `bm25`, or a fusion such as `fused:dense+bm25`) and, per query, the ranked list of document ids that signal returned. Rank order is the whole contract — raw scores are not needed by any metric — which is what lets dense, BM25, SPLADE, and fused rankings share one format. A run with no entry for a query scores that query as an empty ranking: incomplete runs are penalised, never skipped.

Validation rejects, before any scoring: a wrong format version, duplicate query ids, qrels or runs naming unknown queries, duplicate (query, doc) qrels, grades above 30, duplicate documents inside one ranking, duplicate result keys inside one run, and duplicate signal names.

## Behaviour: what is scored and reported

For the requested split, the runner scores every run in the document and prints one `atlas-eval-report/v1` JSON report to stdout. Four metrics are computed per query, then averaged over the queries that have at least one relevant document (`queries_scored`), both overall and per query class:

| Metric      | Meaning                                                                                                                                                                      |
| ----------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| nDCG@10     | Normalised discounted cumulative gain over the top 10 ranks, with graded gains `2^grade − 1` and a log2 discount (the gain at 0-based rank `i` is divided by `log2(i + 2)`). |
| Recall@100  | The fraction of the query's relevant documents that appear in the top 100 ranks.                                                                                             |
| Recall@1000 | The same fraction over the top 1000 ranks.                                                                                                                                   |
| MRR@10      | Mean reciprocal rank: `1 / rank` of the first relevant document, counting only the top 10 ranks (0 when none appears there).                                                 |

Each per-query record in the report preserves the run's top-1000 document ids (field `top1000`) and the query's judged grades, so any number in the report can be recomputed from the report alone. Reports are deterministic: signals, queries, and classes are sorted, and floats are rounded to 6 decimal places.

## Dev/test discipline

The two splits exist to keep measurement honest:

- The **dev** split is for iteration and parameter selection. Compare signals on dev, and pick the winner on dev — the library function `select_best_signal` does exactly this. Selection requires at least one scored development query for each signal. A split with no positive relevance judgments still produces a report for inspection. When judgments exist and every run misses, the zero scores are valid and the usual tie-break applies.
- The **test** split is the held-out set: score it once, at decision time, for the single final scoring pass the protocol allows. Parameter selection on the test split is refused inside the library: `select_best_signal` rejects a test-split report outright. The binary will print a test report — that final pass is legitimate — but nothing in the harness will help you tune against it.

See [Evaluation: golden sets and gates](../concepts/evaluation.md) for how golden sets and gates fit the wider programme.

## Worked example

A toy input, `toy-eval.json` (invented ids and grades; two queries, two signals):

```json
{
  "format": "atlas-eval/v1",
  "queries": [
    { "id": "q-toy-1", "class": "code", "split": "dev", "text": "toy queue lease" },
    { "id": "q-toy-2", "class": "concept", "split": "test", "text": "toy vector index" }
  ],
  "qrels": [
    { "query": "q-toy-1", "doc": "chunk-toy-1", "grade": 2 },
    { "query": "q-toy-1", "doc": "chunk-toy-2", "grade": 1 },
    { "query": "q-toy-2", "doc": "chunk-toy-9", "grade": 1 }
  ],
  "runs": [
    { "signal": "dense", "results": { "q-toy-1": ["chunk-toy-2", "chunk-toy-1"], "q-toy-2": ["chunk-toy-9"] } },
    { "signal": "bm25", "results": { "q-toy-1": ["chunk-toy-1"], "q-toy-2": [] } }
  ]
}
```

Scoring the dev split (only `q-toy-1` is scored; the `bm25` run is not penalised for the test query, and the empty `bm25` ranking for `q-toy-2` would count against it on the test split):

```
$ atlas-eval toy-eval.json dev
{ "format": "atlas-eval-report/v1", "split": "dev", … }
```

The report's per-signal means carry the four metrics above — `ndcg10`, `recall100`, `recall1000`, and `mrr10` — overall and per class.

## Failure shapes

- Wrong number of arguments, or an unrecognised split: the usage line (or the `unknown split` message) on stderr, exit 2.
- The input file cannot be read: `atlas-eval: cannot read &lt;path&gt;: &lt;error&gt;`, exit 1.
- The input parses but is invalid `atlas-eval/v1` data (any validation rule above), or evaluation itself fails: `atlas-eval: &lt;error&gt;`, exit 1. No partial report is printed.

## Exit codes

| Code | Meaning                                                                                    |
| ---- | ------------------------------------------------------------------------------------------ |
| 0    | Success: the report for the requested split was printed to stdout.                         |
| 1    | Runtime failure: unreadable input file, invalid evaluation data, or an evaluation failure. |
| 2    | Usage failure: wrong arity (including `--help`/`-h`/`help`) or an unknown split value.     |
