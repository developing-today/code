# Decision: benchmark regression gates, waivers, and result retention

**For:** issue [#5897](https://github.com/developing-today/code/issues/5897), child of [#5685](https://github.com/developing-today/code/issues/5685)
**Decision date:** 2026-11-08
**Status:** decision record; implementation and CI tiering remain follow-on work.

## Decision

The benchmark programme gates only comparable measurements, uses a per-metric budget plus a run-level noise interval, and never treats missing or failed data as a passing result. An accepted regression is either fixed or explicitly waived for one reviewed change; an accepted baseline is never silently overwritten. The versioned JSONL bundle from [#5896](https://github.com/developing-today/code/issues/5896) remains the machine-readable contract, with the baseline identity from [#5894](https://github.com/developing-today/code/issues/5894).

### Comparisons and regression budgets

Compare a candidate with the standard baseline using paired runs of the same workload, interface, configuration, and scale on the same canonical machine class. Follow #5895's one-variable-at-a-time comparison and warm-up exclusion. The candidate's code SHA is expected to differ; the baseline version, machine class, database engine, configuration, workload, interface, metric, statistic, and unit must identify a valid comparison. Never compare raw values across machine classes or silently bridge a baseline-version, workload, scale, configuration, statistic, or unit change.

The v1 budgets are:

| Metric | Regression direction | Budget |
|---|---|---:|
| Throughput (`operations_per_second`) | lower | 10% decrease |
| Latency p50 | higher | 10% increase |
| Latency p95 | higher | 10% increase |
| Latency p99 | higher | 15% increase |
| CPU per operation | higher | 10% increase |
| Memory per operation / high-water memory, when defined by a workload | higher | 15% increase |

A workload must declare a metric's statistic, unit, and worse-is direction before that metric can gate. The v1 blocking set is throughput and p95 latency, matching #5685's explicit examples and #5895's parent-level gate statement. The other budgets are recorded and reported as advisory until a workload's implementation declares them gate-enabled; adding or removing a blocking metric is a reviewed design change, not an implicit comparator default. No universal budget is inferred for counters or resource metrics whose direction is workload-dependent.

### Noise model and verdict rule

Use at least **10 independent paired process runs** per candidate/baseline comparison. Exclude each run's warm-up from samples; alternate which side runs first across pairs, and do not treat individual operations within one process as independent repetitions. Retain the raw samples required by #5896 and derive the declared statistic for each side of each pair. For each metric, calculate the paired candidate-to-baseline ratio (or its inverse for lower-is-worse metrics) and a two-sided 95% bootstrap confidence interval over the independent pairs.

- **Regression:** the whole confidence interval is beyond that metric's budget in the worse direction. Any blocking metric with this verdict fails the gate.
- **Pass:** the whole interval is within the budget. A candidate may improve one metric and regress another; verdicts are per metric, and a blocking regression is not cancelled by a gain elsewhere.
- **Inconclusive:** the interval overlaps the budget. Run one additional batch of 10 paired processes and recompute over all pairs. If it still overlaps, report `inconclusive`, do not call it a regression or a pass, and request a reference-machine investigation. Inconclusive is non-blocking in v1; the result remains visible and cannot be silently converted to pass.

A comparison that does not satisfy the pairing, sample-count, provenance, or comparability requirements has no gate verdict. The interval is calculated from independent run-level observations, not from a large count of correlated operations; this avoids false precision. CI stages, triggers, and time budgets are owned by #5893, which may select where this protocol runs but must not weaken its verdict rules.

### Result states and completeness

Preserve #5896's versioned JSONL manifest, matrix-cell identity, explicit units/statistics, samples, and environment/build provenance. Each expected matrix cell has one measurement state:

- `measured`: has metrics and a separate per-metric gate verdict (`pass`, `regression`, `inconclusive`, or `waived`);
- `failed`: the workload or harness failed; include the failure reason and any samples actually collected;
- `unsupported` or `not_applicable`: no measurement is claimed; include a reason;
- `incomparable`: preserve the measurement and provenance, but identify why it cannot be compared (for example, machine class, baseline version, workload/configuration identity, statistic, or unit differs);
- `missing`: the expected cell was not emitted. A completeness validator reports it explicitly; absence is never a zero, pass, or regression.

`failed`, `unsupported`, `not_applicable`, `incomparable`, and `missing` are not regression verdicts and cannot satisfy a gate. Keep measurement state separate from gate verdict so a failed run is not misreported as a performance regression and a waiver does not erase the measurement. Add any fields needed for these states compatibly to the #5896 schema; if its published schema freezes these values, increment `schema_version` rather than reinterpret existing records.

### Reviewed waivers

A waiver permits one specifically reviewed change to merge despite a measured blocking regression; it does not change the baseline, hide the result, or waive missing/failed/incomparable data. The PR must carry a structured waiver record with:

- issue and PR, exact tested head SHA, baseline version, and affected workload/interface/configuration/metric cells;
- measured estimate and confidence interval, budget exceeded, and links to the archived result bundle;
- rationale, expected user impact, mitigation or follow-up, author, and a non-author maintainer approver;
- expiry: the PR's merge/close or 30 calendar days from approval, whichever comes first.

The approver must explicitly accept the stated trade-off; the PR author cannot approve their own waiver. A waiver applies only to the named cells and tested change. It does not suppress later comparisons or carry to a new head SHA. Store the record alongside the PR in a machine-readable waiver file under `pkgs/id/benches/waivers/`, and link it in the PR description and result bundle. If the regression is to become the new expected performance, use the re-baselining procedure instead of renewing the waiver.

### Re-baselining and identity

Follow #5894's `id-server-baseline vMAJOR.MINOR` identity and existing `BASELINES.md` rules. Never replace an accepted baseline in place. A baseline update is a reviewed PR that includes the new version, prior and proposed identities, the reason and expected trade-off, approver, reference-machine and toolchain provenance, workload/configuration changes, raw paired results, and the archived JSONL bundle. Measure the old and proposed baseline under the same protocol where they are comparable. A machine-class, workload, scale, or unit change is a MAJOR version event and creates a new incomparable series, per #5894; additive compatible metadata or metric changes may use a MINOR version only when the old comparison identity remains valid. The comparator must preserve old versions and mark incompatible series `incomparable`, never manufacture a percentage across them.

A waiver is not a re-baseline. A performance regression accepted as the new normal requires a new version and an explanation of the change; intentional slower behavior must state its user-facing benefit and the evidence that supports accepting the trade-off. Keep `baseline.txt` compatibility output where applicable, but do not let that legacy format replace the matrix identity or JSONL source of truth.

### Durable result store and retention

Use a **monthly GitHub Release archive** as the durable result store, with one uniquely named compressed bundle per run (`run_id`), containing the #5896 JSONL manifest/cells, raw samples, and raw benchmark output. Use release tags `id-bench-results-YYYY-MM`; add assets without replacing an existing `run_id` asset. Archive every completed run—including failed, inconclusive, incomparable, and waived runs—not only passing runs. Retain release assets and accepted baseline versions indefinitely; an archive correction is a new linked record, never a destructive replacement. GitHub Actions run artifacts remain the convenient 90-day download surface, not the historical source of truth. The job summary links the run ID, verdicts, baseline identity, and durable release asset.

An archive-write failure makes that run incomplete and visible as such; it cannot be reported as a successful gate. The archive publisher must reject duplicate run IDs and limit write permission to the archival job. #5893 owns which CI tiers trigger that job and their schedule; this record only defines what every executed comparison must preserve. Periodically inspect archive size and access, but do not apply automatic expiry or silently prune historical result bundles.

## Alternatives considered

- **One fixed percentage on a single median:** rejected because it ignores metric direction, tail latency, pairing, and measurement uncertainty; it would recreate the noisy raw-throughput gate #5685 warns against.
- **Treating all runner classes as interchangeable:** rejected by #5894's machine-class identity and the existing `BASELINES.md` comparability warning.
- **Using only short-lived workflow artifacts:** rejected because their retention is not a durable, multi-month source of truth. Summaries and dashboards are indexes, never the archive.
- **Keeping only passing or “interesting” runs:** rejected because missing failures and waived regressions would make longitudinal comparisons biased and unauditable.
- **Replacing a baseline after a regression:** rejected because it erases the regression and breaks the versioned identity contract.

## Invariants and boundaries

1. #5685's performance programme remains a baseline-comparison programme with gates; this record does not design its DoS scenarios.
2. Preserve #5896's workloads × interfaces × configurations cell identity and JSONL contract; every expected cell is measured or explicitly accounted for.
3. Reuse #5891 workload/generator definitions, #5894 baseline identity, and #5895 paired one-variable comparison protocol. Their records were available in open PRs when this record was written; review changes must be reconciled before implementation, not silently overwritten here.
4. #5893 owns CI tiering, triggers, and placement. #5892 owns the DoS scenario catalogue. Existing F11.8 reporting remains non-blocking, with its documented >20% reporting behavior unchanged; these new budgets do not alter it.
5. Preserve raw samples, units, machine class, baseline identity, run/build/environment provenance, and all result states. A dashboard, job summary, waiver, or baseline update cannot discard or rewrite the underlying result.
6. Gate verdicts are per metric. Only explicitly gate-enabled metrics block; a metric's budget and worse direction must be declared before it is used as a gate.

## Evidence and bounded assumptions

Checked on 2026-11-08 against current `main` (`df780d3c3b`) and fresh GitHub state. Parent #5685 is open and fixes benchmark gates, published raw results, and noise discipline as programme requirements. #5896 is open and has a posted decision covering the JSONL manifest/cell contract. PR #7662 contains the #5891 workload/generator record and is open; PR #7620 contains the #5894 standard-baseline record and is open although issue #5894 is closed; PR #7615 contains the #5895 comparison protocol and is open. These three PR records are review-pending inputs, not merged main facts. Their current choices—stable workload IDs, canonical machine class and versioned baseline identity, and paired same-environment comparisons—are used as bounded assumptions above. If review changes those contracts, amend this record before implementation; do not silently fork the sibling decision.

The current `pkgs/id/benches/BASELINES.md` describes F11.8 as non-blocking, reports >20% median differences, and warns that CI and laptop machine classes are not comparable. The existing `id-bench-compare.sh` is a report-only comparator; its behavior is outside this decision's implementation scope.
