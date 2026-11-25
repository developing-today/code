# Design: the reranking stage

> **Design — not shipped behaviour.** This chapter describes a proposed design. Nothing in it should be read as how Atlas works today; the final section, "What exists today", states the shipped behaviour it would build on. Tracking: the Atlas programme epic [#13241](https://github.com/developing-today/code/issues/13241) and book issue [#13251](https://github.com/developing-today/code/issues/13251); the feedback seam is [#6046](https://github.com/developing-today/code/issues/6046) / [#8062](https://github.com/developing-today/code/issues/8062); the evaluation harness is [#5741](https://github.com/developing-today/code/issues/5741).

Source: written from `pkgs/atlas-search/crates/atlas-core/src/search.rs`, `fusion.rs`, `eval.rs`, `feedback.rs` and `feedback_normalise.rs` (paths under https://github.com/developing-today/code/blob/main/pkgs/atlas-search/), plus the programme's search-technology research survey. The gain and latency figures quoted below are research figures from that survey — typical values reported for this class of system — not measurements of Atlas.

## What a reranker is, and where it sits

A **cross-encoder reranker** scores a (query, document) pair jointly: the query and the candidate text go through one transformer together, with full attention between them, and the model emits a single relevance score. That is far more precise than comparing two independently computed embeddings — and far too slow to run over a corpus. So the pipeline shape is fixed:

```
retrieve (hybrid, wide pool) → fuse → cross-encoder rerank → top results
```

Atlas's retrieval and fusion stages are described in [Retrieval: hybrid full-text and vector search](../concepts/retrieval.md) and layered further in [Design: layered search characteristics on Snowflake](layered-search.md). The reranker is the last stage: it takes the fused candidate pool — in this design, the **fused top 50–200** — rescores every candidate against the query, and the final order is the reranker's order (or a blend, § "Blend, don't replace" below).

## What it fixes — and what it cannot

A reranker can only reorder what retrieval already found. It therefore:

- **fixes precision** — the order of the first handful of results, which is what nDCG@10 and MRR measure, and what a reader or an agent actually consumes;
- **never fixes recall** — a relevant chunk that no retrieval leg surfaced is invisible to it, and no reranker quality compensates for a retrieval miss.

That asymmetry drives the evaluation discipline (§ "Gating"): retrieval changes are judged on Recall@k, reranker changes on nDCG/MRR over a fixed retrieved pool, so a failure localises to a stage instead of grading the pipeline as one black box — the per-stage pattern set out in [Evaluation: golden sets and gates](../concepts/evaluation.md).

Expected value, stated as the research survey reports it for this class of system: a cross-encoder over the fused ranking typically gains **+5–15 nDCG@10**, at roughly **50–500 ms of CPU latency for a top-100 pool**, depending on model size (about 50–150 ms for 20 candidates with a mid-size model in the cited local deployments). These are programme/research numbers quoted to size the opportunity. Atlas's own numbers do not exist until the stage is built and gated; nothing in this chapter should be cited as an Atlas measurement.

## The default candidate: BGE-Reranker-v2-m3

The design's default open candidate is **BGE-Reranker-v2-m3** (BAAI): 568M parameters, multilingual (100+ languages), 8,192-token query–document pairs, MIT-licensed, and the strongest quality-per-size balance among the open rerankers in the survey's comparison (MTEB reranking ≈ 60.4 in the cited table — a leaderboard figure, quoted for candidate selection only). Multilingual matters here for the same reason it mattered for the embedder: the corpus is not English-only, and a reranker that degrades on the non-English slice would show up exactly where the golden set is thinnest.

Two practical properties shape integration:

- **Scores are unbounded logits**, not probabilities or cosines. Before a reranker score is blended with any other signal it is normalised within the pool (§ below); raw logits are never compared across queries.
- **The architecture is plain** — an `XLMRobertaForSequenceClassification` — which is what makes the candle route below possible.

Alternatives stay on the table as gated challengers, not defaults: Jina Reranker v2 base multilingual (278M, roughly half the size, slightly lower quality in the survey's table — and it ships custom modelling code, which matters for the candle route; its licence history is inconsistent across sources and must be verified on the current model card before any redistribution), and the smaller English fast tier (mxbai-rerank, gte-reranker-modernbert-base) if the latency budget tightens.

## Inference routes: fastembed-rs (ONNX) or candle

Both routes are Rust, in-process, and need no hosted service:

- **fastembed-rs `TextRerank`** — the main route. ONNX Runtime via the `ort` crate, tokenizers via the Hugging Face `tokenizers` crate, a synchronous API. It supports `bge-reranker-v2-m3` out of the box, alongside bge-reranker-base and the Jina rerankers, plus user-defined ONNX models.
- **candle** — Hugging Face's pure-Rust framework, and the route taken when ONNX Runtime is a liability. The liability is concrete: prebuilt `ort` binaries assume AVX-class CPUs and have SIGILL-crashed on older CPUs in a documented deployment, which moved that project to candle with runtime CPU-feature dispatch. BGE-Reranker-v2-m3 ports to candle because its architecture is standard; a reranker with custom modelling code (the Jina candidate) does not, which is a real selection constraint, not a footnote. fastembed itself uses candle as the backend for some of its model families, so the two routes can coexist in one dependency tree.

Serving shape follows the consumers. For the CLI and the local service, an in-process library keeps the deployment Atlas already has. If several consumers (the service, fleet, ID, the MCP server) end up reranking, a small shared inference service holding one warm model beats each consumer loading its own ~0.5–1 GB copy — that decision is deferred until a second consumer actually needs the stage.

One Atlas-specific caution: the existing embedder's ONNX path is a _parity contract_ (bit-identical output is a gate, not a preference — see [Embeddings: Snowflake Arctic and the parity contract](../concepts/embeddings.md)). The reranker carries no such contract: it is a new model with no Python reference to match, judged on ranking quality alone. The ONNX-versus-candle choice for the reranker is therefore free to be made on robustness and latency evidence, without touching the embedder's route.

## Hosted rerankers: quality ceilings, never a serving path

Hosted rerankers (Cohere Rerank, Voyage) are useful to this programme in exactly one role: as an **offline quality ceiling**. Running a hosted reranker over the golden set's fixed pools answers "how much headroom is above the local candidate?" — if the local model is within noise of the ceiling, further reranker work is wasted; if it is far below, the gap is worth chasing.

They are never a serving path, for a reason that is not negotiable: **secret-tier text is never sent to a hosted reranker** — the same absolute rule that keeps secret text away from hosted embedders. A ceiling run is confined to queries and chunks the caller's ceiling permits to leave the machine, which in practice means it is run deliberately, on non-secret material, by a person or a lane that states what it sent. No code path in the serving design contains a hosted-reranker call.

## Gating: per-corpus, or it does not ship

Rerankers can hurt. In one local deployment measurement cited by the survey, two of four rerankers scored _below_ plain hybrid retrieval on that corpus, and the largest was both the slowest and the worse. Leaderboard standing is therefore evidence for candidacy, never for adoption. The adoption rule is:

1. Build a fixed candidate pool per golden query with the shipped retrieval pipeline.
2. Score the reranked order with the #5741 harness (nDCG@10, MRR@10), overall and per query class.
3. Adopt only on a win (or a tie at a cost worth paying) on Atlas's own set — and re-gate on every model, pool-size or blending change, the same as every other ranking change in [Design: the standing evaluation programme](evaluation-programme.md).

### Executable acceptance oracles

Before a reranker is measured on a corpus, its gating contract can be checked with a small, synthetic fixture set. These cases use abstract chunk identities only; they are not relevance examples, carry no corpus text, and must not be added to the golden corpus or to any fixture pool drawn from it.

| Fixture                                 | Synthetic setup                                                                                                                               | Gate                                                                                 | Oracle                                                                                                                                                                                                                                                                                                                                                                                                                                               |
| --------------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------ | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `retrieval_miss`                        | For query `Q`, the judged relevant chunk `C` is absent from the fixed fused pool.                                                             | Retrieval recall gate — judged before, and separately from, any reranking result.    | Pool membership checked against the judged qrels: `C` counts as a recall miss at every pool depth that excludes it, and the case fails the retrieval gate no matter what order the reranker returns. A reranker pass on this fixture is impossible by construction, which is the point: the failure localises to retrieval.                                                                                                                          |
| `ranking_win_on_fixed_pool`             | The fixed pool for `Q` contains the judged relevant chunks, in low fused positions.                                                           | Reranking ranking gate, on that fixed pool only.                                     | The #5741 harness scores the saved reranked rankings against the same qrels and the same pool as the fused baseline: adoption requires a win (or a tie at a cost worth paying, per the adoption rule above) on nDCG@10 and MRR@10, overall and per query class.                                                                                                                                                                                      |
| `ranking_regression`                    | The reranked order places judged non-relevant chunks above the relevant ones the fused order had ranked higher.                               | Reranking ranking gate — a failing case, not a trade-off.                            | The same harness comparison shows nDCG@10 or MRR@10 below the fused baseline on the identical pool, so the candidate is rejected even if its latency is acceptable; leaderboard standing is not evidence against this oracle.                                                                                                                                                                                                                        |
| `recall_invariant_under_rerank`         | Any fixed pool, reranked in any order.                                                                                                        | Separation check between the two gates.                                              | Recall@100 and Recall@1000 computed on the pool are identical before and after reranking, because reranking only reorders the pool. Any recall delta proves the pool itself changed, so the run is scored as a retrieval change, not a reranker change.                                                                                                                                                                                              |
| `blend_uses_normalised_scores`          | A pool whose reranker emits unbounded logits on different scales for two synthetic queries.                                                   | Blending gate (only when the blend option in § "Blend, don't replace" is exercised). | The blended score for every candidate is computed from the min-max-normalised reranker score within its own pool and the fused RRF score; raw logits are never compared across queries and never blended unnormalised.                                                                                                                                                                                                                               |
| `research_figures_are_not_measurements` | The figures quoted in this chapter: +5–15 nDCG@10, roughly 50–500 ms for a top-100 pool, and MTEB reranking ≈ 60.4 for the default candidate. | Provenance gate, checked by citation, not by running the harness.                    | Each figure stays labelled as a research, survey, or leaderboard figure for candidate selection and opportunity sizing, and none of them is entered as an Atlas baseline, an Atlas result, or a gate threshold. An Atlas figure exists only when the #5741 harness has scored saved rankings from Atlas's own golden set — and that harness measures no latency, so an Atlas latency figure additionally requires a separately recorded latency run. |

The two gates are run and reported separately, in causal order: recall on the pool first, ranking quality over that fixed pool second. Passing the synthetic fixtures proves only that the gating contract is wired correctly — that a miss is charged to retrieval, a ranking change is scored on a fixed pool, and research figures cannot be mistaken for measurements. It does not adopt a reranker. Adoption still requires the corpus gate above, and current serving behaviour is unchanged by any of these cases: the shipped pipeline ends at fusion (RRF at `k = 10`, default pool 100 per branch), with no cross-encoder reranker in the serving path, as stated in "What exists today".

## Pool size is the latency knob

Cross-encoder cost is linear in the number of candidates scored, so the pool size _is_ the latency budget: in the survey's local measurements, reranking a top-10 pool was roughly 3× faster than top-30 on that corpus, with no quality loss there; top-50–100 is the common production compromise. This design therefore exposes the rerank pool as a named parameter (alongside the existing retrieval `pool`, from which it draws), defaults it inside the 50–200 band, and sets the shipped default from the golden-set run — the pool size at which nDCG stops improving is the pool size Atlas pays for, and no larger.

## Blend, don't replace (an option the gate decides)

Pure reranker order is the simple design. The alternative is to min-max-normalise the reranker scores within the pool and blend them with the fused RRF score, which protects against reranker pathologies on out-of-distribution queries — a fused candidate the whole retrieval stack agreed on is not discarded because one model disliked it. Neither is asserted here: both orders are scored in the gating run, and the gate picks. What is not optional is the normalisation itself — unbounded logits are never blended raw.

## The feedback loop is the prerequisite for learning-to-rank

Everything above is a fixed model applied to fixed pools. Any stronger claim — learning-to-rank (training or tuning a ranker on Atlas's own usage), personalised ranking, behavioural boosts that adapt — needs labelled evidence of what good ranking looks like _for this corpus and this user_. That evidence is the feedback loop:

- **Capture exists.** `atlas-feedback/v1` records a vote (`up`/`down`, optional comment, `opened` flag) against the `(query_id, chunk_id)` identity of a served result, append-only, with a normalised derivative (`atlas-feedback-normalised/v1`) that classifies comments into categories such as `ranking-too-low`, `missing-result` and `right-file-wrong-chunk`, weighted by requester kind (Drew/user 1.0, agent 0.25, anonymous 0.1; self-feedback by an agent on its own session's retrieval is weighted 0). Query traces (`atlas-query-trace/v1`) record per-result stage scores for every pool entrant, served or not.
- **The seam is the design work.** The #6046/#8062 seam is where those records become ranking experiments: feedback joined back to traces gives, per query, the pool that was available, the order that was served, and a human/agent judgement on it — the minimum dataset from which a learned ranker, or even an honest per-class weight tuning, can be built and gated.
- **Until that seam is wired, no learning-to-rank claim is made.** A reranker adopted on golden-set evidence is a measured component; calling any part of today's ranking "learned" would be false. The feedback loop is sequenced _before_ learning-to-rank work in the programme for exactly this reason.

## What exists today

- **Shipped:** the pipeline ends at fusion. Retrieval (FTS + vector) is fused with reciprocal rank fusion at the shipped constant `k = 10`, equal leg weights, default pool 100 per branch, and the fused order is the served order. **There is no learned or cross-encoder reranker in the serving path.**
- **Shipped, off by default, and gated off on evidence:** the one post-fusion rescore that exists — the Jaccard shingle rescore (#5733) — **failed its held-out gate** (near-exact MRR@10 gain +0.0361 against a +0.10 bar) and stays off (`jaccard_depth = 0`). It is also the standing example of this chapter's gating rule working as intended.
- **Shipped:** the evaluation harness (#5741) that this design gates with — saved rankings from any signal scored offline against judged qrels (nDCG@10, Recall@100, Recall@1000, MRR@10), with a dev/test split discipline in which the test split is scored once, at decision time. It generates no signals and measures no latency; a reranker run would arrive as saved rankings like any other signal.
- **Shipped:** feedback capture and normalisation (`atlas-feedback/v1`, `atlas-feedback-normalised/v1`) and query-trace capture, as described above. **No consumer feeds feedback back into retrieval ranking** — nothing in the search, fusion or rescore code reads feedback records. Today feedback influences only statistics, weekly-report nominations, trace-retention protection, and the normalised derivative store.
- **Not shipped:** any cross-encoder model, any fastembed or candle dependency in the search path, any hosted-reranker call, and any learning-to-rank component.
