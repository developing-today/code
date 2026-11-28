## Problem and goal
Execute approved D1/R2 cloud storage workloads and publish raw measurements, dated price, durability and source metadata. Does not evaluate recall or recommend shards.

## Version and design basis
Version: V4. Settled design: https://github.com/developing-today/code/blob/main/doc/2026-11-13T03-40-00Z_design_id-atlas-summit/part-c/versioned-design.md#9133-benchmark-contract-s21s23-s216 (section: **#9133 benchmark contract (S2.1–S2.3, S2.16)**).
Tier: **contingent**. Start trigger: Only after approved credentials, finite spend cap, named region and representative corpus snapshot/digest are available; this task itself runs fixture-only tests and makes no cloud call.
Statement IDs: S2.1, S2.2, S2.3, S2.16
Enables: evidence inputs to #9134/#9135 placement research

## Scope and ownership
Parent: #9133 (native component/meta parent).
- `pkgs/id/benchmarks/run_cloud.py` (new; owner: C9133-CLOUD)
- `pkgs/id/benchmarks/validate_results.py` (new; owner: C9133-CLOUD)
- `pkgs/id/benchmarks/tests/test_cloud_preflight.py` (new; owner: C9133-CLOUD)
- `doc/2026-11-13T03-40-00Z_design_id-atlas-summit/part-c/results/cloud-run.v1.json` (new; owner: C9133-CLOUD)
- `doc/2026-11-13T03-40-00Z_design_id-atlas-summit/part-c/results/cloud-run.v1.md` (new; owner: C9133-CLOUD)
Hard dependencies (native blockers): C9133-01 — result-v1 schema and benchmark crate interface; C9133-02-D1 — D1 adapter and native counter extraction; C9133-02-R2 — R2 adapter and native counter extraction
Soft sequencing (not blockers): none

## Test / evidence first
Create or validate `pkgs/id/benchmarks/validate_results.py` first. Proposed command: `python3 -I pkgs/id/benchmarks/validate_results.py --input doc/2026-11-13T03-40-00Z_design_id-atlas-summit/part-c/results/cloud-run.v1.json`
Oracle: **research-matrix** — expected `Cloud result includes provider/region/corpus digest, raw D1/R2 storage samples, provider-native operation units, dated price-source, durability/RPO, replica semantics and errors; no search/shard recommendation. The separate preflight fixture rejects malformed runner arguments before side effects.`; verifier: `Owned provider-results validator checks D1 row billing fields and R2 Class A/B/storage units, dated prices, durability and raw samples; owned fixture preflight test uses a fake side-effect recorder to verify parser rejection before credentials/network/locks/writes/spend, with zero provider calls.`.

## Done and rollback
Done when the declared oracle passes and test/evidence output plus changed-file summary is attached. Research evidence: https://developers.cloudflare.com/d1/platform/limits/; https://developers.cloudflare.com/d1/platform/pricing/; https://developers.cloudflare.com/r2/pricing/; https://developers.cloudflare.com/r2/api/s3/conditional-operations/; doc/2026-11-13T03-40-00Z_design_id-atlas-summit/part-c/benchmark-plan.json.
Rollback: Stop run and revoke run-scoped credentials if a preflight gate fails; retain any completed billable output with invalid/incomplete status and do not claim a winner. No automatic provider-side rollback is assumed.

Manifest SHA-256 approval binding: `3b5bce3523945bf5adaa28739f0b90b7b70ff1d6522b4ac827246654e8ff6b06`.
<!-- id-atlas-delivery: #9106:C9133-CLOUD -->
