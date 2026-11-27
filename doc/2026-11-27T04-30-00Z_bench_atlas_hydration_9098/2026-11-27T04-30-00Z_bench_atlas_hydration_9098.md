# Atlas hydration benchmark: Turso path versus R2-reference path (synthetic harness)

This is slice 3 of [#9098](https://github.com/developing-today/code/issues/9098). It builds on the composition census in [the slice-1 report](../2026-11-25T11-33-10Z_report_atlas_embed_storage_composition_9098/2026-11-25T11-33-10Z_report_atlas_embed_storage_composition_9098.md) and the raw-text storage bound in [the slice-2 report](../2026-11-26T23-53-00Z_report_atlas_storage_cost_bound_9098/2026-11-26T23-53-00Z_report_atlas_storage_cost_bound_9098.md). It does not settle the general reference-versus-copy design in [#9084](https://github.com/developing-today/code/issues/9084).

## Result

The content-safe harness (`pkgs/atlas-search/tools/embed_storage_bench.py`) runs the full report shape on a fixed 128-item synthetic sample, 30 runs per operation variant, cold and hot cache. The 5-test oracle (`pkgs/atlas-search/tools/tests/test_embed_storage_bench.py`) passes: Turso and R2-reference reads return the same content hash for every sample including the cache-hit path, missing/corrupt references fail closed without leaking bytes, the hot variant demonstrably reuses cached objects, and the serialized report contains no raw body bytes.

Synthetic outcome (`hydration-bench-results.json` in this directory):

| Measure                                                         | Value          |
| --------------------------------------------------------------- | -------------- |
| Source bytes                                                    | 95,232         |
| Reference (unique-object) bytes                                 | 71,616         |
| Unique objects                                                  | 96             |
| Content-address dedup ratio                                     | 1.329759       |
| R2 Class A puts (one per unique object)                         | 96             |
| R2 request cost, arithmetic estimate                            | $0.00055476    |
| R2 Class B gets, cold (`get-chunk` / `expand` / `with-message`) | 30 / 150 / 150 |
| R2 Class B gets, hot (`get-chunk` / `expand` / `with-message`)  | 1 / 5 / 5      |

Per-operation p50/p95 latencies for `get-chunk`, `expand`, and `with-message` are in the JSON for both paths and both cache variants. They are process-local timings only (see limits).

## What this establishes

The report contract is fixed and verified: one command emits source and reference storage bytes, the content-address dedup ratio, R2 Class A/B request counts and cost, plus p50/p95 latency for `get-chunk`, `expand`, and `with-message` on both paths with the hot-cache variant, and the oracle confirms hash equivalence on every sample. The hot variant cuts reference-path object fetches by an order of magnitude on the repeated-query shape, which is the mechanism the D1 hot-cache counters from PR #13381 would need to exploit on a live path.

## What is still open (needs live venues)

No empirical Turso or R2 numbers are claimed here. The live run still needs a designated read-only Turso credential and a disposable, non-production R2 test bucket/prefix supplied purely through the environment, with the test prefix cleaned afterwards. The harness docstring records this contract; the harness must never be pointed at Atlas production or backup buckets. Until those venues are designated, the p50/p95 comparison against live storage remains the outstanding acceptance item.

## Reproduction

From the repository root:

```sh
python3 -I pkgs/atlas-search/tools/embed_storage_bench.py --sample-size 128 --runs 30 --cache both
python3 -I pkgs/atlas-search/tools/tests/test_embed_storage_bench.py
```

The script uses a fixed in-memory synthetic corpus; it never reads or prints raw Atlas content or credentials. `hydration-bench-results.json` contains the script output.

## Sources and limits

- Composition census: [PR #15064](https://github.com/developing-today/code/pull/15064), merged 2026-11-25. Storage bound: [PR #15832](https://github.com/developing-today/code/pull/15832), merged 2026-11-26.
- R2 adapter and publication reference path: [PR #14555](https://github.com/developing-today/code/pull/14555) and PR #15688, both merged. D1 hot-cache counters: [PR #13381](https://github.com/developing-today/code/pull/13381), merged, with no live event wiring to this path.
- R2 Standard request rates used for the arithmetic cost estimate: [Cloudflare R2 pricing](https://developers.cloudflare.com/r2/pricing/), checked 2026-11-27.
- Synthetic bytes are not a representative Atlas corpus sample; in-memory adapters do not measure Turso or Cloudflare R2 latency; request costs are arithmetic estimates from the checked-in R2 rates, not billed usage.
