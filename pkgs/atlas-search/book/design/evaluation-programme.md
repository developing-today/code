# Design: the standing evaluation programme

> **Design — not shipped behaviour.** This chapter describes the standing evaluation programme as designed and tracked, not a gate that runs today in the form described. The golden set, the offline harness, and the recorded gate results named in [What exists today](#what-exists-today) are real; the grown golden set, the adversarial slices, the CI regression gate on every change class, and the online failure-mining loop are the programme this chapter designs. Tracking: [#8627](https://github.com/developing-today/code/issues/8627) (cross-agent eval sets for the unified index — the standing oracle) and [#5741](https://github.com/developing-today/code/issues/5741) (the ablation harness), under the programme epic [#8983](https://github.com/developing-today/code/issues/8983). Master prompt §2H in the Atlas programme investigation.

Source: `pkgs/atlas-search/eval/README.md`, `pkgs/atlas-search/eval/fixtures/golden-311.json`, `pkgs/atlas-search/crates/atlas-core/src/eval.rs`, `doc/2026-11-08T10-45-00Z_design_atlas_golden_set_eval_gate/` under https://github.com/developing-today/code/blob/main/pkgs/atlas-search/eval/README.md — provenance only; this chapter is complete without following it.

Every other design in this book is gated on measurement. Query planning, layered search characteristics, reranking, the recommender, and the fleet and ID integrations all claim to make search better; the evaluation programme is how any of those claims is checked. Its shape is a standing programme, not a one-off benchmark: a golden set (a fixed set of queries with judged relevant results — the qrels, relevance judgements per query) that grows from real use, a set of metrics assigned per pipeline stage, a regression gate in CI (continuous integration) that every change class must pass, and an online loop that mines real failures back into the set. The current harness is described in [Evaluation: golden sets and gates](../concepts/evaluation.md); this chapter designs the programme around it.

## Grow the golden set from real traces

The golden set's authority comes from where its queries and judgements come from. The design grows it from real traces: gold contexts — the passages and chunks that actually answered a real query — harvested from real retrieval runs, principally the fleet and ID query traces (`atlas-query-trace/v1`) and the feedback records (`atlas-feedback/v1`) that real lanes and real sessions produce.

The failure mode this defends against has a name and two numbers. A system tuned and measured on an offline set alone can score **0.91 offline and 0.40 in production**: the offline set drifts toward the queries its builders imagined, the ranking learns the set rather than the corpus, and production — different queries, different phrasing, different intent — exposes the gap all at once. Harvesting gold contexts from real retrieval runs is the direct countermeasure. A query enters the golden set because someone actually asked it and a run actually answered it; its judgements record what in the real corpus answered it. Offline-only additions are still possible — the adversarial slices below are built, not harvested — but they extend a real-trace core; they do not replace it.

Growth is continuous and versioned. New harvested queries land in a development split first, are judged, and are promoted into the gated set on a stated cadence; a held-out split is preserved so that tuning cannot quietly consume the test evidence. The existing held-out set (54 near-exact queries, held out from the 311-query v3 golden set) is the model for that discipline.

## No-answer probes

A search system that always returns something is not necessarily helping. The golden set therefore includes no-answer probes: queries for which the correct behaviour is to return nothing, or to return results only with an explicit low-confidence signal — queries about material the corpus does not hold, about sources not yet ingested, and about tiers above the caller's ceiling, where "no answer" is also the security-correct answer.

No-answer probes are judged differently from ordinary queries. Precision on them is the metric that matters (see below): every confident-looking hit on a no-answer probe is a fabrication risk in an agent's hands, because a lane will cite what search returns. A change that improves ranking metrics while degrading no-answer behaviour fails the gate.

## Adversarial slices

Beyond ordinary and no-answer queries, the set carries adversarial slices — query groups constructed to attack a specific weakness. Each slice is scored separately, so a weakness cannot hide inside an averaged headline number:

- **Distractors** — queries whose corpus neighbourhood contains plausible, topically adjacent chunks that do not answer the query. They test whether ranking reads intent or merely topic.
- **Semantic twins** — pairs of chunks with near-identical meaning and different provenance, wording, or date, where only one is the correct answer (the current version, the applicable decision). They test provenance and recency handling, which pure semantic similarity gets wrong by construction.
- **Alias-only** — queries that name their target only by an alias, abbreviation, codename, or former name, with no shared vocabulary with the target chunk. They test expansion, rewriting, and entity handling — the planner workstream's home ground (see [Design: query planning](query-planning.md)).
- **Long-doc** — queries whose answer is a small span inside a very long document or session. They test chunking and span handling: a system that retrieves the document but buries the span has not answered the query.

A new attack class becomes a new slice when a real failure exhibits it; the online mining loop below is the usual source.

## Per-stage metrics

Atlas retrieval is a pipeline — retrieval (full-text and vector branches gathering candidates), fusion, any reranking stage, and final presentation — and a single end-to-end number cannot say which stage regressed. The programme assigns each stage the metric that gates it:

- **Recall@k gates retrieval.** Recall at depth `k` — of the chunks judged relevant, what fraction appears in the candidate pool at depth `k` — is the retrieval stage's gate. Nothing downstream can recover a chunk retrieval missed, so retrieval changes (a new signal, a new embedder, an ANN index replacing the exact scan) are judged first on whether recall holds. The harness reports Recall@100 and Recall@1000.
- **nDCG and MRR judge ranking.** Normalised discounted cumulative gain at 10 (nDCG@10) — graded relevance, discounted by rank, normalised against the ideal ordering — and mean reciprocal rank at 10 (MRR@10) — how high the first relevant result sits — judge the ranking stages: fusion, the planner's routing, and the reranker. A reranker that improves nDCG without touching recall is doing exactly its job (see [Design: the reranking stage](reranking.md)); a fusion change that trades recall for nDCG has moved the loss somewhere the headline number hides, and the per-stage split exposes it.
- **Precision@k controls noise.** Precision at depth `k` — of the results returned, what fraction is relevant — controls how much noise a caller must filter. It is the primary metric for the no-answer probes and a guard metric everywhere else: a change may not buy recall or nDCG with a precision collapse that floods lanes with plausible junk.

Metrics are reported per query class as well as in aggregate — the harness already reports per class — so that a strategy that helps one class (say, alias-only queries) and hurts another cannot pass on the average.

## The CI regression gate

The programme's enforcement point is a CI regression gate on **every** change class that can move retrieval quality:

- an embedding **model** change, including a second embedder's introduction and any model-revision bump;
- a **chunker** change — chunk size, boundary policy, or span handling;
- a **fusion** change — weights, the RRF constant, or a new fused signal;
- a **reranker** change — model, pool size, or adoption for a corpus;
- a **planner** change — routing, rewriting, expansion, or any transform's gating.

The gate runs the golden set through the ablation harness (#5741) — saved rankings from each signal combination scored offline against the judged qrels — and compares per-stage metrics and per-slice scores against the recorded baseline. A regression beyond the stated tolerance in any gated metric or adversarial slice blocks the change, whatever its other merits; an improvement is recorded as the new baseline only when the change merges. This is the same discipline that already governed two landed changes: the Rust binary's swap for the Python CLI was gated on the golden 311 tying the Python reference, and the shard programme's oracle is p50/p95 latency at fixed Recall@10 against the standing eval sets — a configuration wins only at equal-or-better recall. The design generalises those one-off gates into the standing rule. Model cutovers in particular follow it absolutely: a new model or embedder cuts over on its eval result, not on its completion.

## Online failure mining

The offline gate cannot see queries nobody has written yet. The online half of the programme mines real failures from real use and folds them back into the set:

1. **Collect.** Fleet lane queries and ID queries (as the ID query path lands — see [Design: search integration for fleet and ID](fleet-and-id-integration.md)) produce traces and feedback continuously.
2. **Mine.** A periodic pass selects failures by signal: queries with down-votes, queries whose results were all ignored or immediately re-queried in different words, queries with no clicks on a full result page, and queries a lane escalated to a human or another source.
3. **Judge and fold back.** Mined queries are judged — what in the corpus should have answered this? — and enter the golden set's development split, or become a new adversarial slice when they exhibit a new attack class. The 0.91/0.40 trap is, operationally, what happens when this loop does not run: the set stays frozen while production moves. Failure mining is what keeps the gated set the same distribution as the real one.

Feedback capture is the seam this loop consumes. `atlas-feedback/v1` records thumbs votes keyed to `(query_id, chunk_id)` result identity — never rank position, so a vote survives re-ranking — with a derived supersede view and requester-kind weighting in the aggregation. Today nothing consumes the aggregated feedback for ranking; the mining loop is its first standing consumer, and any later learning-to-rank claim is built on the same seam, not on a new one.

## What exists today

- The golden set is `pkgs/atlas-search/eval/fixtures/golden-311.json`: the 311-query v3 golden set, copied byte-for-byte from the laptop's golden queries file, whose header records the source corpus size and fingerprint the corpus-specific runner checks before scoring. Beside it: `fixtures/heldout-nearexact-54.json` (54 held-out near-exact queries) and `fixtures/eval-smoke.json` (synthetic; explicitly not held-out evaluation evidence).
- The harness (#5741) exists: `atlas-eval` in `atlas-core` (`eval.rs`, format `atlas-eval/v1`) scores saved rankings from any signal combination offline against judged qrels, reporting nDCG@10, Recall@100, Recall@1000, and MRR@10 per query class. The designed ablation matrix — dense / BM25 / SPLADE / their fusions, each with and without Jaccard rescore — is measured, not argued. Recorded gate results exist for the Jaccard third-signal gate (#5733) and the overlap and boundary-drift studies (#6039, #6038).
- The standing programme issue [#8627](https://github.com/developing-today/code/issues/8627) is open. [#9092](https://github.com/developing-today/code/issues/9092) names its oracle as p50/p95 latency at fixed Recall@10 against the standing eval sets.
- Feedback capture exists with no ranking consumer: `atlas-core` implements `atlas-feedback/v1` (append-only thumbs votes keyed to query and chunk identity) and query traces (`atlas-query-trace/v1`); no code path in the tree consumes aggregated feedback for ranking or recommendation today, and the evaluator scores supplied rankings — it does not itself run retrieval (#8062 records that seam).
- What does not exist today: a golden set grown by harvesting real traces, the no-answer and adversarial slices as standing gated slices, a CI regression gate spanning all five change classes, and the online failure-mining loop. Those are this chapter's design.
