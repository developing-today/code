# Atlas embed-storage economics: raw-text storage bound

This is slice 2 of [#9098](https://github.com/developing-today/code/issues/9098). It derives a conservative storage and storage-cost bound from the composition census in [the merged slice-1 report](../2026-11-25T11-33-10Z_report_atlas_embed_storage_composition_9098/2026-11-25T11-33-10Z_report_atlas_embed_storage_composition_9098.md). It does not measure hydration latency or choose the reference-versus-copy design in [#9084](https://github.com/developing-today/code/issues/9084).

## Result

The measured raw-text columns contain **1,277,693,028 bytes** across the Turso stores in the census. Moving every measured copy out of Turso would free at most **1.278 decimal GB**. A content-addressed R2 object set would hold no more than that amount because equal payloads can share one object; the census did not retain hashes, so the unique object bytes are not known yet.

| Measured raw-text source | Bytes |
| --- | ---: |
| Atlas monolith, chunks and messages | 639,187,867 |
| 29 topic stores, chunk text | 173,545,247 |
| 2 content stores, message text | 464,959,914 |
| **All measured copies** | **1,277,693,028** |

Cloudflare R2 Standard storage is **$0.015 per GB-month**. Usage rounds to whole billable GB-months. This upper bound is therefore **2 GB-months**, or **$0.03 per month** if the account has used its included storage allowance. If the full 10 GB-month Standard allowance remains, these bytes fit within it and the incremental storage charge is **$0**. Egress is free. R2 request charges are separate, and this slice has no request-count measurement, so it does not estimate them. See [R2 pricing](https://developers.cloudflare.com/r2/pricing/).

The issue's 2026-11-12 Turso snapshot says 11 GB of the 24 GB Scaler allowance was used and storage overages were off. Under that snapshot, offloading up to 1.278 GB would reduce Turso usage to about **9.722 GB** and free capacity. It would not reduce the fixed monthly invoice while usage stays under the included limit. The Scaler plan and overage schedule are on [Turso's pricing page](https://turso.tech/pricing); live account usage was not re-read for this bound.

## What this establishes

Storage cost alone is small in the no-free-tier scenario, and may be zero while R2's free allowance remains. The Turso invoice does not fall under the issue's fixed-plan, overages-off state. The measured benefit here is capacity headroom. A choice still needs the content-hash dedup ratio and the Class A / Class B operation load. The latency comparison remains open.

The next slice should run the same representative content through the existing Turso path and the R2 reference path, then measure `get-chunk`, `expand`, and `with-message` hydration latency with the hot-cache variant. Keep its results separate from the general design decision on #9084.

## Reproduction

From the repository root:

```sh
python3 -I doc/2026-11-26T23-53-00Z_report_atlas_storage_cost_bound_9098/storage_bound.py
python3 -I doc/2026-11-26T23-53-00Z_report_atlas_storage_cost_bound_9098/test_storage_bound.py
```

The script reads the checked-in aggregate census and `cost_inputs.json`; it never reads or prints raw Atlas content or credentials. The test oracle is the measured byte totals plus the explicit rate and billable-unit assumptions in that file. `storage-bound-results.json` contains the script output.

## Sources and limits

- Composition data: [PR #15064](https://github.com/developing-today/code/pull/15064), merged 2026-11-25. It records aggregate byte counts only and warns that the monolith and tier stores are separate copies.
- R2 Standard pricing, its 10 GB-month free tier, free egress, and per-million request rates: [Cloudflare R2 pricing](https://developers.cloudflare.com/r2/pricing/), checked 2026-11-26.
- Turso Scaler storage allotment and overage pricing: [Turso pricing](https://turso.tech/pricing), checked 2026-11-26.
- The R2 free-tier remaining balance, current Turso account usage, unique content hashes, and request counts were not read in this slice.
