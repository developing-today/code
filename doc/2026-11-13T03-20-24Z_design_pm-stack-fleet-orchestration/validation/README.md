# Offline design experiment (not production tests)

This standalone standard-library Python model experiments with the proposed selection/exploration contracts in `components/9204-queue-selection-bounded-exploration.md` and the lease/outbox/completion contracts in `components/9209-leases-fencing-outbox-direct-completion.md`. Shared ordering, authority, and no-production boundaries come from `architecture.md`. It imports no Fleet code, does no network or subprocess calls, and writes no files itself; JSON is written only to stdout. Use shell redirection to the owned `report.json` path after argument parsing succeeds.

## Run

```bash
python3 -I doc/2026-11-13T03-20-24Z_design_pm-stack-fleet-orchestration/validation/experiment.py \
  --seed 9196 --cases 1000 --draws 20000 > \
  doc/2026-11-13T03-20-24Z_design_pm-stack-fleet-orchestration/validation/report.json
```

The defaults are `--seed 9196 --cases 1000 --draws 20000`; all three options are declared, abbreviations/extra args are rejected, and case/draw counts must be positive. Parsing and range checks happen before fixture reads. The explicitly requested larger validation run is recorded in the committed report:

```bash
python3 -I doc/2026-11-13T03-20-24Z_design_pm-stack-fleet-orchestration/validation/experiment.py \
  --seed 9196 --cases 5000 --draws 50000 > \
  doc/2026-11-13T03-20-24Z_design_pm-stack-fleet-orchestration/validation/report.json
```

## Named independent oracles

- **Priority and eligibility:** golden decimal-string IDs/ranks parsed exactly; rank 9 sorts before 10 and equal-rank/equal-time ties use canonical ID UTF-8 bytes. Requirements and explicit filters are independent conjunctive gates, including negative-path cases.
- **Dependency graph:** typed fixture edges construct the oracle graph from `hard_blocker` edges only; assertions include blocker→dependent reachability and exclusion of parent/advisory relations. Random directed graphs compare DFS reachability with separately implemented Floyd–Warshall closure; proposed insertion `u→v` cycles iff `u == v` or the prior graph has `v→u`. A serialized opposite-edge case proves the second insertion is rejected.
- **Exploration:** historical profile `r=1.25,d=200,alpha=0.5,width=200`; fixed snapshot densities and initial epsilon mixture masses for epsilon 0, .05, and 1. Float log-weight first-draw probabilities are compared to a 60-digit `Decimal` analytic oracle. Tiny three-item/two-draw inclusion probabilities enumerate all weighted ordered sequences and are empirically checked against the exact inclusion probabilities using the preregistered per-item bound `6*sqrt(p*(1-p)/draws) + 1/draws`. Seeded first-draw frequencies use the same preregistered per-category bound; no post-hoc fitted tolerance. Sampling is without replacement and does not recompute density or mixture after removal.
- **Completion/outbox:** executable negative preconditions reject stale owner, unauthorized actor and blocked dependency without mutation; stale local mutation is also rejected. Lifecycle snapshots are JSON serialized/reloaded at every crash cut; recovery either confirms through same-operation readback or remains pending/unknown, and refuses blind resend in unknown state. The fake remote ledger is keyed by immutable operation ID; this does not establish universal exactly-once semantics or remote fencing.

The JSON report includes seed, SHA-256 fixture digest, runtime, PRNG algorithm, positive oracle counts, random graph cases, number of draws, exact/statistical errors and tolerances, exact and observed small-population inclusion probabilities, plus explicit proof/production limits. PRNG is a SHA-256 counter stream with a domain separator and 53-bit uniform grid; it does not depend on Python's `random` version.

## Limits and next step

This is a small synthetic reference experiment, not a proof and not a production test suite. It makes no production-benefit, calibration, liveness, deployment, exactly-once, or remote-fencing claim. The fake remote ledger cannot establish universal exactly-once semantics; Fleet fences cannot stop a request already in flight to GitHub. No actual Fleet broker, Rust/TypeScript adapter, GitHub API, service, queue, or deployment was exercised.

Future conversion belongs in a separate validation implementation issue under #9198: port the pure model into the existing `fleet-queue` test seam, red-first named tests with `-- --list` proving nonzero selection, property-based graph/order invariants, and generated model-based lifecycle/crash-cut sequences; then test adapters and live/deployed behavior separately. This report is not formal production test coverage and does not authorize implementation or rollout.
