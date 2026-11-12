# Synthesis directives and architecture judgments

Owner: summit design session. Date: 2026-11-11 UTC. These are proposed
design constraints, not claims of implementation or external research results.

## Scope and horizon boundaries

- **V1:** the existing system and its landed contract set, with runtime gaps
  explicitly distinguished. Do not call design-only lineage, manifest or traces
  deployed. Historical performance numbers are dated evidence, not current SLOs.
- **V2, evidence before adaptation:** diagnose misses; provenance-resolve visible
  occurrences; capture feedback with exposure context; classification-safe
  retrieval; observable, bounded tiering; read-only citation-bearing agent recall.
  Start from a deterministic baseline, not an automatically trained ranker.
- **V3, controlled adaptation and connected evidence:** debiased opt-in ranking
  experiments, a typed cross-source graph, reversible curation proposals and
  measured placement policies. Promotion needs held-out evidence, isolation and
  rollback. No autonomous irreversible decisions.
- **V4, policy-bounded corpus stewardship:** personalized, workload-adaptive
  retrieval and research assistance with explicit budget/quality contracts;
  narrowly delegated reversible operations only after audited V3 evidence.
  Human approval remains required for usefulness-driven irreversible deletion.

All seven threads need a V2/V3/V4 row with a stable claim ID, exact v1 dependency
IDs linked to the landed documents, predecessor gate, acceptance oracle and
fallback. Later prose inherits its row's dependency tag; no untagged horizon
promises. Gates are capability gates, not calendar schedules or one big release.

## Cross-cutting constraints

1. **Visibility before computation.** Select authorized physical stores before
   branch ranking; enforce again on graph traversal, occurrence enumeration,
   hydration, cache reads, replay, exports and telemetry. Reclassification and
   rights revocation invalidate serving eligibility even for an older snapshot.
   Historical replay cannot borrow yesterday's permissions.
2. **Three distinct axes:** classification/security (`public/internal/secret`
   in the scaler design), temperature (`hot/warm/cold`), and source/retention
   class (`code/docs/session logs/...`). Never turn a manifest label or hotness
   score into authority. Do not silently alias the old four-rung sensitivity
   ladder into the newer three-store taxonomy; name the compatibility gap.
3. **Identity is not authority.** Bytes/content hash, occurrence, logical file,
   immutable version, source-native coordinates, writer set, and interpretation
   revision are different. Deduplication does not merge permissions or rights.
   Edges carry evidence, kind and confidence; embedding similarity cannot make
   an authoritative identity merge.
4. **Scoped completeness.** A resolver can be complete only for its committed
   ingestion coverage, retained state, permitted scope and graph revision.
   Forbidden occurrences are not returned, counted, hinted at or joined through.
   A tombstone is metadata, not universally public information.
5. **Replay honesty.** Persist identity/version/hash/ranking/configuration
   references separately from text. Exact byte replay is possible only while
   retained bytes are lawful and authorized; otherwise return an authorized
   tombstone/unavailable response. Never regenerate and pretend it is the old
   answer. A checksum is integrity evidence, not author truth or signature trust.
6. **Separate data plane and control plane.** Preserve the read-only serving
   role. Feedback and traces use a separate authorized append path; promotion
   stays with the aggregator. The learned ranker cannot mutate source history,
   classification, writer authority, licensing or retention rules.
7. **Bound all expensive work.** Query fan-out, graph depth/edge count, candidate
   depth, hydration bytes, cold opens/restores, training jobs and explanation
   payloads each need budgets, cancellation, timeouts and explicit partial-state
   reporting. Do not infer linear speedup from the scaler's historical math.

## Evaluation before learning

A synthetic, exhaustively known eligible-corpus fixture is the decisive oracle
for the missing-vs-buried distinction. Production logs alone cannot prove global
absence. Diagnostics separate:

- scope/coverage unknown or ingest incomplete;
- target known absent from this fully enumerated eligible snapshot;
- eligible target stored but not indexed or not generated as a candidate;
- eligible target in candidates but below the served cutoff;
- target served but unsupported/wrong for the intended claim;
- intentionally excluded by version, temperature or policy (only disclose
  policy details if allowed; otherwise ordinary not-found shape).

Keep diagnosis multidimensional (stage + coverage confidence), not a misleading
single boolean. Positive-control fixtures prove one target's existence;
negative controls have full fixture enumeration. Retained-source discovery and
source-outage handling are separate ingestion questions, not ranker failures.

Log exposures, ranked candidates/scores where permitted, served cutoff and exact
configuration. Non-clicks are not negatives; thumbs are about a particular
exposure, not permanent truth about a document. Agent self-citations and repeated
copies are not independent human judgments. Randomized exploration requires
explicit opt-in and propensities; use offline, frozen, time-split comparisons
before any online adaptation. Separate retrieval metrics from answer support.

Proposed numeric guardrails must be labeled unmeasured defaults, with the
measurement that will choose them. No invented win rates or corpus-wide recall
claims from judged top-k alone. Include rare queries, language/garbling,
historical pins, rights changes and restricted partitions in evaluation slices.

## Placement and curation

The November 4 ruling is speed-first within a plan envelope, not cheapest-first.
Turso/D1/rustfs are existing placement roles, not an instruction to duplicate
every data object across all three. D1 is not a cross-tier corpus authority.
Cold shards are outside default fan-out under the November 10 cold format;
explicit historical/cold addressing is a separate budgeted operation.

No retrieval does not mean no value: an unindexed, under-ranked or unexposed
version cannot supply unbiased usefulness evidence. Prefer confidence-qualified
proposals, hysteresis, pinned heads, retained provenance and rollback. Storage
headroom includes temporary export/restore/compaction and duplicate-byte costs.
Cost models must distinguish meter units (storage, sync, read/write, embedding,
egress, local hardware), measured age, uncertainty and policy version.

Usefulness curation proposes archive/placement or retention review. It neither
authorizes source-rights purge nor delays required purge. V1 cold-before-retire
applies to routine retention; deletion-propagation purge deliberately has no
archive-first step. Rebuilds/restores replay revocations before serving.

## Extensibility and new research seeds

Consider evidence disagreement/contradiction views, missing-coverage repair
requests, signed portable evidence packages, user-controlled feedback export,
policy-conditioned benchmark packs, demand forecasting with confidence, and
agent evidence receipts. Keep them options with dependency tags and falsifiable
next experiments, not mandatory framework selections. Prefer typed relational
edges/read projections before a mandatory graph database or GraphRAG rewrite.

## Review and delivery

Main design must include one navigation entry, full statement coverage, sourced
option comparisons and dead ends, versioned capability matrix, detailed seams
and lifecycle failure model, per-stage entry/exit gates, model/property-test
oracles, granular deduped follow-ons, open questions with chosen defaults, and
the evidence limitations. Supporting research remains committed. Independently
buildable follow-ons are filed and queued in the same pass; horizon-dependent
work is conditional, not falsely marked ready. No runtime code in this PR.
