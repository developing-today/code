# Design: the single-user recommender

> **Design — not shipped behaviour.** This chapter describes a proposed design. Nothing in it should be read as how Atlas works today; the final section, "What exists today", states the shipped behaviour it would build on. Tracking: [#9140](https://github.com/developing-today/code/issues/9140) (recommendation objects, search history, retention classes), [#9109](https://github.com/developing-today/code/issues/9109) Part C (ID–Atlas storage economics, ID search and the recommendation surfaces), and [#10158](https://github.com/developing-today/code/issues/10158) (recommendation snapshot paging); programme epic [#13241](https://github.com/developing-today/code/issues/13241), book issue [#13251](https://github.com/developing-today/code/issues/13251).

Source: written from `pkgs/atlas-search/crates/atlas-core/src/search.rs`, `trace.rs`, `feedback.rs` and `shards/` (paths under https://github.com/developing-today/code/blob/main/pkgs/atlas-search/), the #9109 Part C design record (`doc/2026-11-13T03-40-00Z_design_id-atlas-summit/part-c/`), and the programme's search-technology research survey (§5). Judgements labelled as such below are design judgements, not sourced benchmark claims.

## Why a personal recommender is a different problem

Classic recommender systems run on collaborative filtering — "users who liked X also liked Y" — computed across many users. Atlas has **one user**. There is no cross-user signal to copy, no crowd to average over, and no off-the-shelf collaborative design that survives contact with a population of one.

What a single-user corpus has instead is rich behavioural exhaust from that one user, plus the corpus's own structure:

- every search session, and which results were opened;
- saves, re-finds (a document the user keeps returning to is a hub, whatever its text says), and dwell;
- provenance — which documents derive from, cite, or were produced alongside which others;
- co-occurrence in sessions, projects and time windows.

The recommender in this design is therefore **content-based + session-based + recency-aware**, with the collaborative machinery repurposed: "items that go together" means items that appear together in _the user's_ sessions and work, not across users. The signals are also the behavioural layer of [Design: layered search characteristics on Snowflake](layered-search.md) — search and recommender are designed here as one loop, not two systems.

## The signals

Four nominating signals, each cheap, each biased in a different direction — the biases are why the architecture (§ below) unions them rather than picking one.

### 1. The profile centroid (content similarity)

The user profile is a **weighted centroid of embeddings**: take the vectors — from the embedding index Atlas already maintains, see [Embeddings: Snowflake Arctic and the parity contract](../concepts/embeddings.md) — of the documents the user opened, saved or searched for, weight each by the strength of the interaction, **decay the weights with age**, and average. Candidates are the embedding nearest neighbours of that centroid: "more like what I have been working on", with zero new infrastructure beyond the index and the interaction log.

Decay is the load-bearing detail. An undecayed centroid is a portrait of everything the user has ever cared about, dominated by the largest past project; a decaying one tracks the current work, with the half-life a named, tunable parameter. Per-source profiles (one centroid for code work, one for notes, and so on) are a refinement the same shape supports when a single centroid proves too blunt.

### 2. Session co-occurrence (item-to-item, single-user)

Documents opened or cited in the same session or task are related _for this user_, even when their texts are not similar — a specification and the incident log it eventually caused may share almost no vocabulary. Precomputing item→item neighbours offline from the session logs (pairs that co-occur, weighted by how often and how distinctively) is the strongest single-user analogue of Amazon-style item-to-item collaborative filtering, and the signal least replaceable by content similarity: it captures relationships that exist in the user's work rather than in the text.

The raw material is already recorded: every executed search mints a query trace (`atlas-query-trace/v1`) carrying the served results, and feedback records (`atlas-feedback/v1`) attach votes and `opened` flags to `(query_id, chunk_id)` pairs. Session co-occurrence is computed from that history, not from a new tracking system.

### 3. Recency and frequency

Trending-in-my-corpus: recent opens, repeated re-finds, and fresh ingest. A newly ingested document that matches an active interest should be able to surface _before it is ever searched for_ — that is the difference between a recommender and a search box. Frequency matters independently of recency: the document returned to every week for a year is infrastructure, and its neighbours are standing recommendations.

### 4. Graph and provenance neighbours

Atlas's corpus is provenance-first — content-addressed objects with occurrence records, and ID-graph edges where the publication format (#6062) lands: see [The corpus: provenance and content addressing](../concepts/corpus-and-provenance.md) and [Design: search integration for fleet and ID](fleet-and-id-integration.md). Neighbours of a known-relevant document along those edges — the next version, the document that derived from it, documents sharing a rare chunk or entity — are candidates no text signal would nominate.

## The architecture: the same three stages as search

Production recommenders and production search have converged on one shape — **candidate generation → ranking → re-ranking** — which is also the shape of Atlas's search pipeline ([Retrieval: hybrid full-text and vector search](../concepts/retrieval.md)). The recommender reuses the stack instead of duplicating it:

1. **Candidate generation (recall).** Union the nominators: embedding kNN against the profile centroid; item-to-item neighbours of recent items; recency/frequency leaders within active sources; graph/provenance neighbours. Each nominator is biased; the union covers the biases. Sensitivity ceilings and tier enforcement apply here, inside retrieval, exactly as in search — a recommendation is a disclosure, and the ceiling discipline of [Sensitivity tiers and client ceilings](../concepts/sensitivity.md) is inherited, not re-implemented.
2. **Ranking (precision).** Score the union with the richest affordable model. The natural Atlas choice is the cross-encoder of [Design: the reranking stage](reranking.md), with the "query" being a profile/session summary — or, before that stage exists, a transparent feature score (similarity, recency, co-occurrence count, source) whose weights are named constants. Either way the ranker is a component with an evaluation (§ below), not a formula beyond inspection.
3. **Re-ranking (the surface).** Enforce what raw relevance ignores: **diversity** via MMR (maximal marginal relevance — each next pick is penalised for near-duplication of picks already made), a **freshness** quota so the feed is not all old hubs, deduplication against already-seen items, and **exploration** slots — a small ε of deliberately novel items, without which the system collapses into a filter bubble of the user's last topic and can never learn that his interests moved.

## The two-tower question: overkill, deliberately

A trained two-tower model learns a user tower and an item tower into a shared embedding space so that retrieval is kNN of the user vector. It is the right tool at multi-user scale. For one user it is overkill _as a trained model_ — there is exactly one user vector to learn — but its _pattern_ is precisely signal 1 above: the "user tower" collapses to the hand-built, decaying weighted profile centroid. **[Design judgement, not a sourced benchmark claim.]**

So the profile vector is v1, and the upgrade path preserves the serving shape: learned per-source or per-signal weights, or a sequential model over session order (SASRec/BERT4Rec-style "what's next in this session", interesting only once session logs are long and dense), can replace the hand-built centroid _in place_ — candidate generation still consumes one profile vector, and no consumer changes. Choosing the simple form first is not a compromise to apologise for; it is the form whose failures can be read directly off its weights.

## Surfaces

- **More-like-this** on any document or session: pure embedding kNN plus the tier filter, with co-occurrence and graph neighbours blended in when they exist. The day-one surface — it needs no profile at all.
- **Related results beside search:** session co-occurrence neighbours of the top search hits, presented as their own rail rather than interleaved into the ranked results, so search quality measurements stay clean.
- **A resurface feed:** documents matching active interests that were recently ingested, or recently _became_ relevant (a new link, a new occurrence, a new neighbour), ranked by profile similarity + recency and passed through the MMR/freshness/exploration re-rank. This is the surface that justifies the whole system: it finds things before they are searched for.
- **Query suggestions and zero-result recovery:** from the profile and the co-occurrence table — "you needed X last month alongside this" — offered when a search returns nothing or when a query matches a known past session's shape.

## Recommendations as stored objects, and the "next 10" fan-out (#9109 Part C)

The #9109 Part C design settles the serving shape for the ID-facing surfaces: **one fan-out worker, a Durable Object accumulating a "next 10", and recommendations as stored objects with per-class retention**. Unpacked:

- Recommendations are **materialised objects**, not computed per request: a generation pass produces a ranked, re-ranked list for a subject (a user profile state, a document, a search), stores it as an object with a retention class (#9140's subject), and surfaces page through the stored object. That makes recommendation pages stable, cheap to serve, and auditable — what was recommended, when, from which signals, can be answered after the fact.
- The **fan-out worker** is the single producer: it runs the candidate union across the nominators (and, in the ID shape, across the federated stores) and hands partial results to the accumulator rather than blocking a request on the slowest nominator.
- The **Durable Object accumulates the "next 10"**: as fan-out results arrive, the DO merges, ranks and re-ranks them into the next page of the stored recommendation object, so a consumer paging forward is always reading an accumulated, deduplicated, diversity-checked page — never a raw nominator's output, and never a page that changes shape because a slow nominator landed late.

Paging over a materialised object needs its own discipline: an immutable materialised order (score descending, then immutable recommendation ID ascending) at a pinned cutoff, per-page signed envelopes, and a cursor bound to the snapshot it pages. The state machine implementing this serving primitive is merged in PR [#12873](https://github.com/developing-today/code/pull/12873); it does not implement the fan-out, candidate generation, or ranking described above.

## Synthetic acceptance oracles

These fixtures are proposed design checks, not shipped recommender tests. Each uses invented IDs and scores; the oracle is the stated relation, not a real user's data. Keep the corresponding executable tests alongside whichever implementation is eventually approved.

| Invariant | Synthetic input | Pass oracle | Fail oracle / source |
|---|---|---|---|
| One-user signal boundary | Two invented profiles A and B; replay an `opened` event for A on item `doc-a` only. | A's signal state changes; B's state and every shared/cross-user aggregate remain byte-for-byte unchanged. | Any B state change or cross-profile contribution fails. The single-user boundary is the premise in § “Why a personal recommender is a different problem”; event shapes are grounded in [`feedback.rs`](https://github.com/developing-today/code/blob/main/pkgs/atlas-search/crates/atlas-core/src/feedback.rs) and [`trace.rs`](https://github.com/developing-today/code/blob/main/pkgs/atlas-search/crates/atlas-core/src/trace.rs). |
| Deterministic candidate union and ranking | Freeze profile, source snapshot, policy epoch, scoring parameters, and nominator outputs: one emits `r-a, r-b`, another `r-b, r-c`; all scores are 0.7. Permute completion order, with every frozen output included before materialisation. | Candidate union is exactly `r-a, r-b, r-c`, each once; unchanged features give unchanged scores and ties resolve by immutable ID ascending. | Duplicate IDs, changed scores, or arrival-dependent membership/order fail. This is a proposed generation/ranking oracle for [the three-stage architecture](#the-architecture-the-same-three-stages-as-search), not a guarantee supplied by paging [PR #12873](https://github.com/developing-today/code/pull/12873). |
| Deterministic diversity re-ranking | Freeze candidates `r-a, r-b, r-c` with relevance 1.0, 0.9, 0.8; similarities `sim(a,b)=1`, `sim(a,c)=0`; MMR weight 0.5 and two slots. No quotas bind; exploration is disabled for this fixture. Add already-seen `r-s` and duplicate nominations of `r-b`. | Seen/duplicate IDs are removed. Greedy MMR picks `r-a`, then `r-c`: their second-pick objectives are 0.4 for `r-c` versus -0.05 for `r-b`. Permutations preserve that sequence; any objective tie resolves by immutable ID. | Selecting the redundant `r-b` second, retaining `r-s`, duplicate slots, or arrival-dependent picks fails. Oracle fixes `MMR = 0.5 × relevance − 0.5 × max similarity to prior picks`; production parameters remain design choices. Source: [re-ranking stage](#the-architecture-the-same-three-stages-as-search) and [resurface feed](#surfaces). |
| Sensitivity ceiling | Candidate `secret-x` has strongest score but tier `restricted`; request ceiling is `public`. Candidate `public-y` is weaker and public. | `secret-x` is excluded before ranking/surfacing; `public-y` may remain. | Returning or ranking `secret-x` before filtering fails. Source: [Sensitivity tiers and client ceilings](../concepts/sensitivity.md), and the retrieval-stage inheritance in § “The architecture”. |
| Snapshot paging stability | Materialize synthetic order `(0.9, r-2), (0.9, r-8), (0.4, r-1)` at cutoff `t0`; fetch page one, then add a late candidate. | Page two follows the pinned order; equal scores tie by ID; late candidate is absent from this snapshot. Cursor for another parent and expired cursor reject. | Changed page boundary/order, late insertion, wrong-parent acceptance, or expired-cursor acceptance fails. Source: [#10158](https://github.com/developing-today/code/issues/10158) and merged [PR #12873](https://github.com/developing-today/code/pull/12873). |
| Held-out evaluation separation | Synthetic sessions: training half opens `train-a`; held-out half opens distinct `heldout-b`. Freeze the eligible corpus and construct recommendations using training behaviour only. | Evaluation scores held-out opens only; held-out interaction IDs, labels, and feedback are invisible to profile construction, candidate generation and ranking. `heldout-b` may still be nominated from its eligible content, without knowledge of its later open. | Leaking held-out behaviour into features or counting a training open as a held-out hit fails. Evaluation target: [Design: the standing evaluation programme](evaluation-programme.md); feedback event source: [`feedback.rs`](https://github.com/developing-today/code/blob/main/pkgs/atlas-search/crates/atlas-core/src/feedback.rs). |

These checks bound privacy, determinism, ranking inputs, paging, and evaluation leakage. They do not settle the human-reserved policy or sensitivity-tier decisions.

## Evaluation

The recommender is measured like search, on held-out behaviour: split real sessions, and from each session's first half predict its held-out opens — precision@k and nDCG on that prediction task — plus coverage and diversity measures, so the system cannot score well by merely echoing search results back. A recommender that only recommends what search would have found anyway has added a feed, not a capability. The standing programme that hosts these measurements is [Design: the standing evaluation programme](evaluation-programme.md).

## What exists today

- **Design only.** The recommender is described by #9140 (recommendation objects, search history, retention classes) and #9109 Part C (fan-out worker, Durable Object “next 10” accumulator, stored recommendations with per-class retention). **No candidate generation exists**: no profile centroid is computed, no co-occurrence table is built, no recommendation endpoint is served, and no feedback-driven personalisation runs anywhere in the system.
- **Paging implementation merged; recommendation generation remains unbuilt.** Issue #10158 is closed and its PR #12873 is merged. The code (`worker/src/recommendation_state.ts` plus tests) implements state for paging an already-materialised recommendation snapshot: immutable score-descending/ID-ascending order at a pinned cutoff, per-page signed envelopes with their own envelope IDs, and a cursor bound to the parent snapshot ID. It generates no recommendations and does not make the proposed recommender shipped behaviour.
- **The raw material exists.** Query traces (`atlas-query-trace/v1`) are captured for every executed search, feedback capture (`atlas-feedback/v1`) and its normalised derivative are shipped, and the embedding index the profile centroid would query is the production index. What is missing is every consumer that would turn that material into recommendations — today those records feed statistics, weekly-report nominations and retention protection only.
- **Authorization dependency rechecked (2026-11-27).** The native blocked-by relation for #10158 names #9083; both issues are closed. [#9083's completion receipt](https://github.com/developing-today/code/issues/9083#issuecomment-7090474120) closes the **design-and-handoff** acceptance: tenancy design [PR #11325](https://github.com/developing-today/code/pull/11325) merged and implementation issues E1–E7 (#12796–#12802) filed. Closure is not evidence that tenant-scoped subject authorization or the policy-epoch interface is integrated into recommendation paging; [PR #12873](https://github.com/developing-today/code/pull/12873) alone does not establish that end-to-end boundary. See [#10158](https://github.com/developing-today/code/issues/10158).
