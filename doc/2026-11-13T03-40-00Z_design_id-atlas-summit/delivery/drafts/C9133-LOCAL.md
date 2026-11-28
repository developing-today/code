## Problem and goal
Run redb/Turso on identical corpus bytes and digest; capture named laptop/runtime/network/cache/storage-shard conditions in validated result-v1 record. Keep local results separate from cloud results.

## Version and design basis
Version: V2. Settled design: https://github.com/developing-today/code/blob/main/doc/2026-11-13T03-40-00Z_design_id-atlas-summit/part-c/versioned-design.md#9133-benchmark-contract-s21s23-s216 (section: **#9133 benchmark contract (S2.1–S2.3, S2.16)**).
Tier: **recommended**.
Statement IDs: S2.1, S2.2, S2.3, S2.16
Enables: S2.1 local comparison evidence; later placement research

## Scope and ownership
Parent: #9133 (native component/meta parent).
- `pkgs/id/benchmarks/run_local.py` (new; owner: C9133-LOCAL)
- `pkgs/id/benchmarks/tests/test_local_runner.py` (new; owner: C9133-LOCAL)
- `doc/2026-11-13T03-40-00Z_design_id-atlas-summit/part-c/results/local-run.v1.json` (new; owner: C9133-LOCAL)
- `pkgs/id/benchmarks/validate_local_results.py` (new; owner: C9133-LOCAL)
Hard dependencies (native blockers): C9133-01 — result-v1 schema and workload runner interface; C9133-02-redb — redb adapter implementing shared operations; C9133-02-turso — Turso adapter implementing shared operations
Soft sequencing (not blockers): none

## Test / evidence first
Create or validate `pkgs/id/benchmarks/validate_local_results.py` first. Proposed command: `python3 -I pkgs/id/benchmarks/validate_local_results.py --input doc/2026-11-13T03-40-00Z_design_id-atlas-summit/part-c/results/local-run.v1.json`
Oracle: **research-matrix** — expected `Local result contains redb and Turso runs over the same independently recomputed corpus digest, raw per-operation samples and runner/git/hardware/OS/runtime/cache/storage-shard metadata for both engines; never compares local numbers as cloud performance.`; verifier: `Owned domain validator recomputes corpus bytes digest, validates both engine sample sets and checks required environment fields per engine; fixture tests use no benchmark run.`.

## Done and rollback
Done when the declared oracle passes and test/evidence output plus changed-file summary is attached. Research evidence: https://docs.rs/redb/4.3.0/redb/; https://docs.turso.tech/sync/usage; doc/2026-11-13T03-40-00Z_design_id-atlas-summit/part-c/benchmark-plan.json; doc/2026-11-13T03-40-00Z_design_id-atlas-summit/part-c/synthetic-workload.json.
Rollback: Do not use or publish a mismatched result; mark output invalid and retain current per-surface store authority. No data migration occurs.

Manifest SHA-256 approval binding: `3b5bce3523945bf5adaa28739f0b90b7b70ff1d6522b4ac827246654e8ff6b06`.
<!-- id-atlas-delivery: #9106:C9133-LOCAL -->
