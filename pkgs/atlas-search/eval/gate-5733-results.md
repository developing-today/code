# Section-4 held-out gate for the Jaccard rescore — verdict record (#5733)

Run 2026-11-17 by queue task 19897 (lane any-n), per the design note's
section 4 (task 10487) and the pre-agreed bars. **Verdict: FAIL — the
near-exact bar is not met; `jaccard_depth` stays 0 (defaults OFF). No
production ranking change was made.**

## Measurements

| Set | Metric | Rescore OFF | Rescore ON | Bar | Result |
| --- | --- | --- | --- | --- | --- |
| Near-exact (54 queries) | MRR@10 | 0.5635 | 0.5997 | gain ≥ +0.10 | **+0.0361 — FAIL** |
| General (311 queries) | nDCG@10 | 0.4097 | 0.4080 | regression ≤ 0.01 | 0.0017 — pass |

Diagnostics: the near-exact tier (J ≥ 0.60) fired in only 4 of 54
near-exact queries and 0 of 311 general queries. The gold chunk was in
the top-100 candidate union for 50 of 54 near-exact queries and in the
top 10 for 43 both ways; per-query MRR improved for 7 queries, worsened
for 0 — the standard-tier blend (0.85·rrf + 0.15·J) does quiet good,
but the decisive tier almost never fires: against a long chunk's full
shingle set, a 30-token passage's Jaccard is capped far below 0.60 by
set-size geometry alone. The signal as designed cannot deliver the
+0.10 near-exact gain on real chunk lengths at threshold 0.60.

## Method

- **General set:** `fixtures/golden-311.json` (frozen 2026-10-26; gold
  chunk_ids as qrels, binary gains). All 311 queries scored — every
  query retains at least one gold chunk present in the corpus copy.
- **Near-exact set:** `fixtures/heldout-nearexact-54.json` (this
  change): verbatim 30-token passages sampled deterministically from
  corpus chunks (every 997th chunk id, ≥60 normalized tokens, raw
  tokens [10:40]), gold = source chunk. 54 queries (≥50 required).
- **Base retrieval (identical for both arms):** the corpus's Python
  reference hybrid — Snowflake `snowflake-arctic-embed-m-v2.0`
  trunc-384 cosine + FTS BM25, RRF k=60 (eval3.py's own functions),
  branches deepened to top-100 so the rescore union is the top 100.
- **Rescore:** the shipped Rust `rescore_fused` (PR #10335) ported to
  Python in `gate5733.py`, **pinned before measuring** against the
  Rust test vectors (xxHash64 reference values, the normalizer test
  case, the short-text shingle rule, Jaccard fractions) — all exact.
  Configuration measured: depth 100, threshold 0.60 (design default).
- **Sanity gate:** at branch depth 30 the base pipeline reproduced
  the bake-off's recorded production-hybrid numbers for golden-311
  **exactly**: recall@10 183/311, MRR 0.3640. No number below was
  trusted until this reproduced.

## Corpus disclosure

`~/atlas/v3/corpus.db` holds 101,339 of the frozen 101,471 chunks
(sample fingerprint `07b3e17b…` matches the frozen set's header); 132
chunk ids are absent (a 99486–99613 block plus four scattered ids),
including one gold chunk (99555). Every golden-311 query keeps other
gold chunks present, so all 311 scored; the vector matrix covers all
101,471 ids. The drift is disclosed rather than silently absorbed:
it cannot manufacture a near-exact gain, and the general regression
is two orders of magnitude inside its bar, but a future rerun should
use a bit-exact frozen corpus.

## SPLADE-trigger implication (for #5741's record)

This result is about **raw set-overlap geometry**, not about the
lexical signal family: Jaccard's failure mode is that full-chunk
shingle sets dilute short-passage overlap below any usable threshold.
SPLADE's learned sparse scores weight terms rather than counting set
overlap, so this verdict neither validates nor undermines SPLADE.
What it does say: a lexical third signal, in overlap form, is at best
a mild tie-breaker (general regression ≈ 0, small real near-exact
gain) — not the decisive near-exact mechanism the design hoped for.
The SPLADE adoption decision should rest on its own ablation through
the #5741 harness, with expectations set accordingly.

## Reproduction

```sh
LD_LIBRARY_PATH=<gcc-lib>/lib:<zlib>/lib \
  ~/atlas/v3-venv/bin/python pkgs/atlas-search/eval/gate5733.py
# writes /tmp/gate5733-results.json; runtime ~19 min on the loaded laptop
```
