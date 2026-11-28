# Atlas overlap budget experiment — report for #6039

Question (from #6039, parent #6028): overlap past ~20% is mostly waste
per general RAG practice, but Atlas had never measured its own
tradeoff. What overlap does each source type actually earn?

**Answer: 10% for both prose docs and logs.** At a 200-word fixed
window, 10% overlap captures essentially all of the measurable
boundary-coverage benefit; 20% adds ~12 points more index and embed
cost over 10% and buys no measurable recall or coverage. 0% is
materially worse exactly where theory says it should be: passages
that straddle a chunk boundary are never retrievable as a whole.

## Method

- Script: `pkgs/atlas-search/eval/overlap-experiment.py` (stdlib
  Python, no network, no model download). Results:
  `pkgs/atlas-search/eval/overlap-results.json`, byte-identical
  across re-runs at the same HEAD (verified).
- Corpus slice, from this repository at the experiment HEAD: prose =
  12 Markdown documents under `doc/` and `pkgs/atlas-search/`
  (34,504 words); logs = the newest 1,500 `git log` entries grouped
  into 25 documents (63,027 words).
- Chunking: fixed 200-word windows per document; stride
  200 / 180 / 160 words for 0% / 10% / 20% overlap.
- Signal: BM25 (lexical). The #5741 eval harness scores saved
  rankings offline, so these same qrels can be re-scored under a
  dense signal later without re-running the corpus work.
- Golden set: **provisional**, constructed by the script from the
  corpus itself — the #6027 golden set has not landed, so per the
  task packet this experiment defines its own and names the re-run
  owed (below). 64 queries: per source type, 16 *interior* queries
  (whole sampled passages: prose paragraphs, log entries) and 16
  *boundary* queries (50-word spans centred on 0%-overlap window
  boundaries — the case overlap exists for). A query is the source
  passage's top-5 highest-IDF content words; a chunk is relevant to
  a query iff its word span intersects the gold span in the same
  document. Because queries are derived from the gold passages,
  absolute recall is an upper bound for lexical signals; the
  *comparison across overlap settings* is the result, and it is
  neutral by construction (identical queries/qrels per config).

## Results

Index size (per source type):

| overlap | prose chunks | prose stored | prose embed tokens (est.) | logs chunks | logs stored | logs embed tokens (est.) |
|---|---|---|---|---|---|---|
| 0%  | 178 | 221,053 B | 45,890  | 325 | 490,618 B | 83,765  |
| 10% | 196 (+10.1%) | 244,558 B | 50,785 (+10.7%) | 358 (+10.2%) | 542,838 B | 92,684 (+10.6%) |
| 20% | 219 (+23.0%) | 273,538 B | 56,903 (+24.0%) | 402 (+23.7%) | 608,048 B | 103,882 (+24.0%) |

Embed cost is estimated as total chunk words × 1.33 tokens/word —
every chunk is embedded once, so embed cost tracks chunk words
exactly; the percentages are the substance.

Recall@10, any-overlap relevance: **1.000 in every configuration**
for both types and both query classes. Under BM25 with these
queries, overlap never changes whether *some* fragment of the gold
passage is found — it changes whether a *whole* passage is
retrievable in one chunk, which is what the coverage metrics show:

Recall@10 with ≥80% span coverage (and mean best coverage in top 10):

| class | type | 0% | 10% | 20% |
|---|---|---|---|---|
| boundary | prose | 0.000 (cov 0.500) | 1.000 (cov 0.975) | 1.000 (cov 1.000) |
| boundary | logs  | 0.000 (cov 0.500) | 1.000 (cov 0.981) | 1.000 (cov 1.000) |
| interior | prose | 0.813 (cov 0.890) | 0.938 (cov 0.961) | 0.938 (cov 0.983) |
| interior | logs  | 0.813 (cov 0.911) | 1.000 (cov 0.981) | 0.938 (cov 0.975) |

At 0% overlap a boundary-centred span is split exactly in half
(mean best coverage 0.500, as constructed) and no chunk ever
reaches 80% coverage. At 10% every boundary query in both types
has a ≥80%-covering chunk in the top 10. Interior differences
between 10% and 20% are one query in sixteen — noise, not signal.

## Recommendation to the chunk-boundary policy under #6028

- **Prose docs: 10% overlap** at a ~200-word window. Boundary
  coverage is fully recovered; 20% costs +12% more chunks/embed
  than 10% for no measured gain.
- **Logs: 10% overlap**, same reasoning. The expectation in #6039
  that prose and logs would differ did not hold at this window
  size and signal: both types saturate at 10%. (Log *entries* are
  short; an entry-aligned chunker — a separate #6028 design
  question — would change this comparison more than overlap does.)
- Do not default to 20%: on this corpus it is +23–24% index size
  and embed cost over 0% (≈ +12% over 10%) with zero measured
  coverage benefit over 10%.

## Limits and the re-run owed

- BM25 only. Dense embeddings blur chunk boundaries differently
  (a split passage can still embed near its other half), so the
  0% penalty is likely *smaller* under dense retrieval and the
  saturation point could move. Re-run owed: score these qrels
  under the dense signal via the #5741 harness when an embedding
  route is wired for eval.
- Provisional golden set (self-derived queries, 16 per class per
  type — small n; one query = 6.25 points). Re-run owed when the
  **#6027 golden set lands**: swap the qrels, keep the corpus and
  chunking code unchanged, and compare against this report's
  tables before the #6028 policy is finalised.
- One window size (200 words), one corpus slice (this repo's own
  docs and git log). Other corpora (chat transcripts, code) are
  unmeasured; code in particular was out of scope for #6039.

<!-- agent-note:v1 agent=repo-worker-code-y task=12883 issue=6039 -->
