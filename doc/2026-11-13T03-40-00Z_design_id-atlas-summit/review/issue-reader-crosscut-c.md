# #9106 delivery — crosscut / Part C issue-reader review

**Verdict: SOURCE FIXES PASS; regenerate approval-bound drafts before publication.** I rechecked only the two prior handoff findings against the frozen crosscut/Part C manifests and sequencing text; I did not reread the 42 bodies. The prior 42-row coverage remains the scope of this bounded recheck. The crosscut verifier passes (39 issue slots; 17 implementation + 2 research records; DAG, collisions, statement coverage, tiers, and config oracles).

## Recheck

1. **Root ownership resolved.** `9110-02` now explicitly owns phase 1 (private root skeleton, no absent leaf declarations/exports, then relinquishes root writes); `9110-09` owns phase 2 after required V2 leaves merge (sole public export wiring). Consumer leaf modules use direct paths, with no concurrent root writers. Sequencing text now states this serial ownership model. The V2 gate remains in `9110-09`; V3 remains additive/later and no V4 module blocks V2.

2. **Five domain oracles resolved in the frozen manifest.** `C9133-LOCAL`, `C9133-CLOUD`, `C9135-01`, `C9135-03`, and `C9138-01` now each own and invoke a domain-specific validator; `C9135-01`'s validator also has an explicit `--check-test` path for the owned Rust contract test. The stated oracles now name the exact fields/constraints checked, including cloud preflight without provider calls, no placement winner without comparable measurements, and an undecided embedder owner decision. These are future owned deliverables, not claims that their validators already exist or have run.

**Publication handoff note:** the temporary issue drafts I last inspected still contain the previous generic commands/text. Regenerate the approval-bound report and drafts from the frozen manifests before publishing; this is draft freshness, not a remaining design/oracle blocker.

## Previously verified checks (not re-read in this bounded follow-up)

- `9110-02` keeps absent modules unexported; `9110-09` is the V2 integration/release gate and does not wait on V4. Its V3 export dependencies are soft/later.
- The issue set preserves the identity split: `PayloadId`/`ContentId`/`EnvelopeId`, legacy signature bytes, and `ParentSnapshotId` versus per-page `EnvelopeId` are not conflated.
- GC/R2 scope does not assume conditional `DeleteObject`: the fence plus never-reused generation-qualified physical keys are prerequisites; absent proof keeps deletion disabled. The reference model permits ordinary bounded scans and does not invent a producer/backend guarantee.
- Cache contracts fail closed on stale/unknown policy and cover revocation. Part C and crosscut issues consume #9077/#9083/#9084-owned decisions rather than settling those decisions themselves.
- `9110-R1` owns byte-vector research and its validator. `9110-R2` permits an explicit producer-unavailable result; that outcome must not be replaced by a fabricated interoperability vector/claim.
- The original-30 task allows a completed bounded scan with `unrecovered` and no fabricated list. The #9112 orchestration record is a separate meta row with only the design-PR publication dependency, so it can start on the ready frontier and need not wait for every child to close.

## Coverage

- Crosscut (19): `9110-01`–`9110-11`, `9110-13`, `9111-01`–`9111-05`, `9110-R1`, `9110-R2`.
- Part C (22): `C9133-01`, `C9133-02-D1`, `C9133-02-R2`, `C9133-02-redb`, `C9133-02-turso`, `C9133-LOCAL`, `C9133-CLOUD`, `C9134-01`–`02`, `C9135-01`–`03`, `C9136-01`–`02`, `C9137-01`–`02`, `C9138-01`–`02`, `C9139-01`–`02`, `C9140-01`–`02`.
- Delivery special: `meta-9112-R-original-30-provenance`.
