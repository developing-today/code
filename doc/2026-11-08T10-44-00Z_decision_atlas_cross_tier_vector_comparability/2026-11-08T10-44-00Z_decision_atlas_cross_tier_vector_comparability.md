# Decision: one embedding model for every Atlas tier — cross-tier comparability by construction

- **Date:** 2026-11-08
- **Status:** Decision record; no implementation is included here.
- **Issue:** [#6060](https://github.com/developing-today/code/issues/6060)
- **Parent:** [#6019](https://github.com/developing-today/code/issues/6019)
  (semi-automated ingestion operations), follow-up 42, Cluster 7.
- **Related:** fan-out consumer
  [#5755](https://github.com/developing-today/code/issues/5755) (closed),
  classification revalidation
  [#6021](https://github.com/developing-today/code/issues/6021), query traces
  [#6027](https://github.com/developing-today/code/issues/6027), embedder design
  [#5350](https://github.com/developing-today/code/issues/5350).

## Decision

**One embedding model everywhere.** Every chunk vector in every Atlas tier —
public, internal, and secret — is produced by the single mandated embedding
model, and every query vector is produced by that same model and pipeline.
Cross-tier comparability is an invariant of the corpus, not a correction
applied at merge time. The per-tier-models-plus-normalization route is
rejected; the merge/rescore contract below states what the mandate requires
of fan-out, rescore, ingestion, and the query trace.

The mandated model today is the one the corpus already uses:
`snowflake-arctic-embed-m-v2.0` via the `atlas-embed` crate (local ONNX,
768 dimensions, CLS pooling, L2-normalized output), held to bit-parity with
the Python corpus pipeline by Gate A (#5351). "Model" in this record means
model **and** pipeline: weights, tokenizer, pooling, and normalization —
the `atlas-embed` crate docs treat each of those points as load-bearing,
and changing any of them changes the vector space.

## Why this is decidable now: the current state

- There is exactly one embedder. `pkgs/atlas-search/crates/atlas-embed`
  implements the single model above; the default vector table,
  `vec_snowflake_m_v2_768_f32` (`atlas-core/src/types.rs`), encodes that
  model and width in its name. No tier uses a second model today.
- Fan-out merge assumes comparability. `merge_top_k`
  (`atlas-core/src/shards/scan.rs`) sorts all hits from all scanned shards
  by raw cosine distance, tie-broken by tier merge rank, shard, and chunk
  id. That sort is only meaningful if every distance was measured in one
  vector space against a query vector from the same space.
- The invariant is currently **unenforced and unobservable**. A shard's
  `atlas_manifest` (`atlas-core/src/shards/mod.rs`) records tier, topic,
  part, expected chunk count, and `metric` — and the consumer refuses a
  shard whose metric is not cosine — but no manifest field records which
  embedding model produced the shard's vectors. A second model could
  enter a tier today and nothing would notice; merge would silently
  compare distances across spaces.
- The local/API split does not force a second model. The premise behind
  per-tier models was that the secret tier might be embedded locally
  while the internal tier uses an API route. The mandated model already
  runs locally as an ONNX embedder, so the split is about **where the
  same weights execute**, not which weights: internal-tier embedding may
  run through the serve path, secret-tier embedding runs the identical
  model locally, and no secret text needs to leave for an external API
  in either case.

## Rationale, and the rejected alternative

The rejected route — per-tier models with a normalization contract at
merge/rescore (per-model score calibration, rank-based merge, or per-tier
result quotas) — was considered seriously; rank-based fusion is already
the house pattern for combining FTS and vector branches (`atlas-core`
`fusion.rs`, reciprocal rank fusion with `RRF_K = 10`). It is rejected
for cross-tier vector merge for four reasons:

1. **Calibration corrects distributions, not queries.** Per-model score
   calibration aligns score scales on average. The residual per-query
   error lands exactly on cross-tier boundary cases — a secret-tier
   candidate competing with internal-tier candidates for the last top-K
   slots — which are the cases where a wrong order is hardest to notice
   and, for the secret tier, the worst to get wrong in either direction.
2. **The small tier cannot calibrate.** The secret tier is 1 shard /
   638 chunks against the internal tier's 28 shards / 107,001 chunks
   (#5755). There is no per-tier eval corpus of meaningful size for the
   secret tier, and the golden eval corpus is single-model; a
   calibration fitted on that little signal is a guess wearing a
   contract's clothes.
3. **Rank merge and quotas change what merge promises.** Rank-based
   cross-tier merge discards the exact-distance semantics `merge_top_k`
   and the rescore work (#5733/#5741) are built on, and per-tier quotas
   cap a tier's recall by fiat rather than by relevance. Both make
   #6027's top-K survival statistics conditional on the merge method
   instead of on retrieval quality.
4. **One space keeps every downstream artifact interpretable.** Gate A
   parity, the golden eval corpus, per-shard survival statistics, and
   vector-neighborhood classification checks (#6021) all assume vectors
   are mutually comparable. A second space forks each of them per model.

The mandate may be revisited only by amending this record with contrary
evidence — in particular a per-tier eval corpus large enough to validate
a named calibration method against the one-model baseline. Absent that
evidence, a tier that cannot run the mandated model does not silently
adopt another one (see cost 3).

## Cost consequences of the mandate (accepted)

1. **A model change is a corpus event, not a per-tier upgrade.** Moving
   to a new embedding model means re-embedding every tier
   (107,639 chunks at the #5755 counts, plus growth since) into a new
   vector-table generation, re-baselining the golden corpus, and
   cutting over atomically per fan-out set. Mixed generations never
   merge. This is a batch job with a plan-spend check under #6019's
   guardrails, and it is deliberately not cheap or casual.
2. **Tiers cannot adopt models independently.** A tier that would
   benefit from a different model (size, language coverage, hardware
   fit) waits for a corpus-wide decision. The flexibility given up is
   real; it is the price of comparability by construction.
3. **A tier that cannot run the mandated model cannot join cross-tier
   fan-out.** If a future tier's constraints (air-gap, licensing,
   hardware) make the mandated model unrunnable there, that tier stays
   out of merged search — or this record is amended with the evidence
   in the rationale section — but it never enters a fan-out under a
   different space. Silent divergence is a defect, on the same footing
   as a metric mismatch, which the consumer already refuses.
4. **Secret-tier embedding stays local execution.** Because the same
   weights must produce secret-tier vectors, and secret text does not
   go to an external embedding API, the secret tier's embedding path is
   the local ONNX embedder. That constrains secret-tier ingestion
   throughput to local compute; at 638 chunks today this is not a
   binding constraint, and growth is handled under #6019's disk/IO and
   plan guardrails.

## Merge/rescore contract

- **M1 — One space per fan-out.** Every shard admitted to a fan-out,
  and the query vector, share the mandated model identity (model id +
  pipeline version + dimension). A fan-out over mixed model identities
  is invalid and must fail loud, never merge partially — the same
  posture `fanout_scan` already takes toward a failing shard and
  `scan_one` takes toward a tier above the caller's ceiling.
- **M2 — Manifest provenance.** `atlas_manifest` gains an
  `embedding_model` key naming the model that produced the shard's
  vectors. The consumer validates it for every scanned shard, next to
  the existing `metric` check. A missing or mismatched key rejects the
  shard (strict mode) for the same reason a non-cosine metric does.
- **M3 — Merge stays exact.** `merge_top_k`'s raw-distance sort is
  unchanged. No per-tier or per-model score transform may be introduced
  at merge or rescore; any future rescore stage operates inside the one
  vector space and applies globally, not per tier.
- **M4 — Tier boundary is untouched.** Comparability work never admits
  a shard. Tier-ceiling filtering happens before merge, at discovery
  and again in `scan_one`; the secret tier enters a fan-out only under
  a secret-capable token, explicitly (#5755). Nothing in this record
  relaxes pre-merge tier filtering, and the #6021 classification
  revalidation is unaffected: vector-neighborhood checks across tiers
  remain meaningful precisely because there is one space.
- **M5 — Query-trace provenance (#6027).** The query trace records
  `embedding_model` for every candidate, copied from its shard's
  manifest, and `query_embedding_model` once at the trace head. Under
  the mandate these are expected to be identical for every candidate of
  every query; survival statistics stay interpretable without per-model
  conditioning, and a trace showing two model ids in one result set is
  positive evidence of an M1 breach and is filed as a defect, not
  normalized away.
- **M6 — Ingestion asserts the mandate.** The embed pipeline and the
  staging-promotion path (#5669) record the producing model for every
  new shard and refuse promotion of a shard whose model is not the
  mandated one. New corpora join under the mandated model or not at
  all.

## Follow-on implementation (separate issues, filed with this decision)

1. Add the `embedding_model` manifest key and consumer-side validation
   at fan-out (M2/M1), with tests covering mixed-model refusal.
2. Record per-candidate `embedding_model` provenance in the query trace
   (M5), as the #6027 trace slice.
3. Assert the mandated model in the embed pipeline and staging
   promotion (M6), so a non-mandated shard cannot be promoted into a
   tier.

## Consequences and invariants

1. Cross-tier score comparison is meaningful by construction; no
   calibration, rank-merge, or quota machinery is built or maintained.
2. The failure mode for model divergence is a loud refusal at fan-out
   or promotion time, never a silently degraded merge.
3. #6027 survival statistics, Gate A parity, and the golden eval corpus
   keep a single-model interpretation.
4. The tier boundary enforced at fan-out (#6021, #5755) is exactly as
   strong after this decision as before it; comparability and
   classification stay separate concerns.
