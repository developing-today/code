# Proposal urgency migration record

**Date:** 2026-11-12. **Scope:** only the local, unpublished #9166/#9167 proposal packets; no GitHub/queue/Project state was changed. This preserves original intrinsic scores and why these optional research items were normalized before publication. The standalone packets were moved from the temporary `designs/proposals/` staging location into the package-standard root `proposals/` directory so the coordinator can ingest them without deep-path discovery; their IDs, parent scope, content and dependencies were retained.

## Rule applied

The companion PM charter §3–4 defines intrinsic urgency as lower=sooner: 0–99 Emergency, 100–199 Supercritical, 200–399 Critical, 400–799 High, 800–1199 Standard (default 1000), 1200–1599 Low, 1600–1999 Someday, 2000+ Cold. Scores 0–399 are reserved for actual incident/critical-path work. These speculative, nonblocking infrastructure/MTC investigations must not outrank epic #9155 urgency 450 High merely because an older draft used small numbers.

Intrinsic urgency remains distinct from importance, dependency/blocker status, implementation stage and dispatch directive. The publisher's existing queue priority projection is separate: High→20, Standard→40, Low/Someday/Cold→70. The proposal retains `urgency`, `urgency_band`, and `queue_priority_projection`; do not substitute queue priority for intrinsic score or change PM schema.

## Original → normalized scores

| Proposal | Original draft score | Normalized intrinsic score/band | Queue projection | Rationale |
|---|---:|---|---:|---|
| FUT-01 | 180 | 1400 / Low | 70 | Optional synthetic OpenBao lab; useful groundwork but not a production prerequisite. |
| FUT-02 | 170 | 1400 / Low | 70 | Capability probes are bounded research; no V1 dependency. |
| FUT-03 | 160 | 1800 / Someday | 70 | Workload authentication/federation is speculative and topology dependent. |
| FUT-04 | 155 | 1400 / Low | 70 | Recovery proof is important before a pilot, but pilot itself remains optional. |
| FUT-05 | 100 | 1400 / Low | 70 | OCI feasibility is informational; no deployment authorized. |
| FUT-06 | 105 | 1400 / Low | 70 | K8s integration is topology-dependent and links to existing #9160 delivery work. |
| FUT-07 | 75 | 1800 / Someday | 70 | No current cluster/source-of-truth decision is needed for V1. |
| FUT-08 | 85 | 1800 / Someday | 70 | Mesh/workload identity is only justified by concrete workload need. |
| FUT-09 | 130 | 1800 / Someday | 70 | V4 directed-site federation; core reconcile model already belongs to #9159. |
| FUT-10 | 60 | 1800 / Someday | 70 | R2 archive remains optional and must not duplicate #9160 PERF-06. |
| FUT-11 | 55 | 1800 / Someday | 70 | Access-plane comparison is not an active requirement; SSO is #9077. |
| FUT-12 | New | 1800 / Someday | 70 | New parked OCapN/iroh research proposal; already uses correct scale. |
| FUT-13 | New | 1800 / Someday | 70 | New cross-source offline-failure composition experiment; consumes #9160 PERF-04/06 evidence without redoing adapters/cache/provisioning. |
| MTC-01 | 40 | 1800 / Someday | 70 | Optional standards/implementation maturity research. |
| MTC-02 | 35 | 1800 / Someday | 70 | Optional synthetic lab; no MTC baseline dependency. |
| MTC-03 | 30 | 1800 / Someday | 70 | Optional byte-accounting experiment; no V1 requirement. |
| MTC-04 | 28 | 1800 / Someday | 70 | Optional freshness/fallback experiment. |
| MTC-05 | 25 | 1800 / Someday | 70 | Optional TLS interoperability experiment; ML-DSA-44 remains baseline. |
| MTC-06 | 20 | 1800 / Someday | 70 | Optional operations/cost model. |

No proposal is High because none is a V1 foundation or hard prerequisite. The numeric score changes do not alter issue scope, importance, stage, native parent, dependencies, or safety gates. If the publisher adopts a different approved mapping, record a new migration event rather than silently rewriting these scores.
