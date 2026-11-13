# Part A graph delivery normalization checkpoint

- **Scope:** only `implementation-slices.json` plus this normalization note. Other graph research/design artifacts were read-only inputs and were not edited.
- **Canonical row contract applied:** stable id, native component parent, title/kind/version/tier/statements/goal/enables, exact owned path + `new|existing` + owner slice ID, typed hard `requires`, typed soft `after`, test file/command/typed oracle, rollback, exact design source/heading and owner; contingent items have explicit triggers. Research rows cite sources and own a report/artifact validator.

## Manifest changes

- Preserved all 16 prior graph slice IDs and complete S5.1–S5.64 coverage.
- Added only `A-GRAPH-9125-03`, the genuinely missing real candidate-engine encoding/storage bake-off. It is a research/benchmark slice (harness + raw artifact + report + acceptance validator), not production codec work; it is the sole row tagged `research_scope: real-encoding-bakeoff`.
- Removed `atlas-content-hash-producer-follow-up.md` from graph slice ownership. Crosscut `9110-R2` owns that producer-contract research (moving to parent #9113); the graph row keeps only a soft consume-later edge. The existing part-a-graph follow-up note remains read-only context.
- Removed any ownership of `pkgs/id/crates/id-core/src/lib.rs`; `A-GRAPH-9125-01` hard-requires `9110-02` for common root-export registration and owns only new graph module/test paths. It also consumes `9110-11` legacy-profile compatibility output.
- Marked existing `pkgs/id/crates/id-core/src/petname.rs` as `existing`, owned by `A-GRAPH-9131-01` for compatibility tests only.
- Updated #9128 freshness acceptance to use a durable projection-ledger sequence for the complete `(scope, tenant, query, generation)` feed, preserving each source's own authority/epoch/sequence and pinned frontier. The test oracle includes two sources with identical source sequence numbers, stable projection sequence/entry IDs and generation invalidation.
- Kept `A-GRAPH-9129-01` V2 for the `succeeds` edge core; moved `A-GRAPH-9129-02` comment carry projection to V3 with the social shape dependency.
- Query-language row is research/decision only: it compares alternatives and may decide “none”; no parser or named API is selected by the manifest.

## Local validation

To run after this edit:

```sh
python3 -I -m json.tool doc/2026-11-13T03-40-00Z_design_id-atlas-summit/part-a-graph/implementation-slices.json >/dev/null
```

The bounded local checker also verifies: original IDs retained plus the one bake-off row; 17 total; every S5 statement covered; required keys and exact test ownership; all intra-graph and crosscut refs resolve; all owned paths are disjoint; existing/new labels match filesystem state; design headings exist; only one real-bakeoff scope; and no crucial hard prerequisite is below crucial. No global delivery gate, reviewer, issue filing, Git operation or external write is run here.

## Source/design continuation completed

- Toy equality checks now use explicit exceptions in all execution modes, before any PASS line; the mutation probe verifies a corrupted decoded row is rejected. SQLite JSONB is accurately included in the measured toy serialization scope, with no production/candidate-engine winner claim.
- The feed contract and `versioned-design.md` now use a durable projection-ledger sequence per complete feed scope, preserve source sequence only as provenance, pin source frontiers, invalidate on policy generation changes, and explicitly cover two authorities both at `(epoch, sequence) = (e,1)`.
- V2 retains the `succeeds` edge/version-DAG core; V3 owns only social comment carry projection.

## Validation performed

- `python3 -I graph-encoding-toy.py` — explicit equality checks and mutation-rejection probe pass.
- `python3 -I -O graph-encoding-toy.py` — same correctness checks and PASS results under optimized Python.
- `python3 -I graph-encoding-toy.py --check-oracles` — checks pass; stdout compared byte-for-byte with `experiment-output.txt`.
- Normal and optimized toy stdout compared directly; unknown flags for both scripts exit with code 2 before any PASS output.
- `python3 -I feed-cursor-oracle.py --check` and `python3 -I -O feed-cursor-oracle.py --check` — two source attributes at the same source sequence, distinct epochs, pinned cutoff, since/before order, restart high-water and policy-generation invalidation cases pass in normal and optimized Python.

The feed fixture is an in-memory model only; it does not claim any runtime or deployed replication behavior. The remaining cross-part gap is the Atlas producer contract owned by crosscut `9110-R2` (moving to #9113); this graph part neither owns that investigation nor infers an algorithm. Until its producer/profile contract exists, Atlas `content_hash` remains an opaque external identifier.
