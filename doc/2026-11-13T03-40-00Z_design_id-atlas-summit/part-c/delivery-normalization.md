# Part C delivery normalization

`implementation-slices.json` is now schema-version 3 and uses one canonical handoff row shape: `id`, numeric `parent`, `title`, `kind`, `version`, `tier`, statement IDs, `goal`, `enables`, explicit file/status/owner tuples, hard `requires`, soft `after`, typed `test` oracle, `rollback`, source heading, and owner. Research rows additionally carry `research_scope` and `research_evidence`; contingent rows carry a `trigger`; benchmark rows carry future operator commands and CLI contracts.

## Boundary and dependency repairs

- Kept all 22 IDs and exact coverage of 19 S2 + 17 S6 statements.
- Each test file is a newly owned file from that row; commands name its future executable test/validator and harness manifest. The benchmark crate manifest, shared schema and workload types have one owner (`C9133-01`); the four engine adapters have separate files/tests and hard dependencies on that owner.
- Hard edges use exact slice IDs or named issue interfaces with the minimum output. `after` is soft ordering only. No crucial row hard-depends on a recommended/contingent row.
- File collision check compares exact paths across Part C and crosscut slices; final check found 63 Part C-owned paths, disjoint from crosscut-owned paths. The pre-existing crosscut `artifact_profile_compat.rs` collision remains explicitly registered by crosscut itself; C9135-02 uses its own adapter-consumer test path.
- C9135-02 consumes exact slice `9110-11` compatibility vectors and adds only an id-sync consumer test; it does not re-own the legacy/canonical codec decision. C9138-01 consumes #9113 identity contracts and #9092 model/eval artifacts; it does not claim the external producer source or rerun recall.

## Parent-supplied R2 and recommendation corrections

- R2 primary conditional-operations reference: https://developers.cloudflare.com/r2/api/s3/conditional-operations/ . Conditional Put/Multipart/Copy are writes, not conditional DeleteObject. Research, benchmark mapping, fixture and reference oracle now say so. GC requires #9084 atomic generation fence plus immutable generation-qualified physical keys that are never reused; after fencing, only the captured old key may be plainly deleted. If either fence or non-reuse is unproven, deletion is unsupported/disabled. Resurrection g+1 gets a new key; oracle injects a stale g1 delete after g2 creation and checks g2 bytes/ref survive.
- C9133-01 owns and has aligned the checked-in benchmark plan, synthetic fixture and reference oracle, including identical-put row preservation, tombstoned ordinary-put rejection and explicit-resurrection-only g+1. No provider call/timing was made.
- Main versioned design now states the physical-key fence and parent/page identity separation. The tiny page vector uses deterministic-CBOR identity encoding for a non-circular parent envelope; fixture signatures are markers and no cryptographic/production-state claim is made. Parent SnapshotId excludes page EnvelopeIds; each page EnvelopeId is distinct; cursor binds parent+position. The oracle tests cutoff, equal-score order, restart and wrong-parent rejection.
- All five Part C research rows carry `research_evidence`, new result-report artifacts and `research-matrix` expectations enforced by explicitly owned domain validators: `validate_local_results.py`, `pkgs/id/benchmarks/validate_results.py`, `validate_store_authority.py`, `validate_sqlite_sync_research.py`, and `validate_embedder_research.py`. These validator files are future outputs of their owning issue rows; no validator code or benchmark was executed here.
- `C9135-01`'s validator CLI accepts only `--input` plus optional `--check-test`; the latter invokes the owned narrow `store_authority_contract.rs` Cargo test through `~/tools/fleet-build-lock`. The cloud runner validates the whole bounded argument set before side effects; offline result validation is a separate command.

## Evidence/operation boundaries

- `C9133-LOCAL` has research scope `local-laptop-benchmark`; `C9133-CLOUD` is the sole `cloud-benchmark` row. Neither was run. Cloud runner requires a finite spend cap and strict whole-CLI validation before credentials, network, lock, write or spend; `--fixture-only` tests use no provider access. Search recall/shard recommendation stays wholly with #9092; #9133 can only link/validate an owner artifact.
- No adapter implementation, local timing run, cloud operation, new source research, external write or Git action was performed in this source-fix task.
