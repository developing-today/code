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

The recommender in this design is therefore **content-based + session-based + recency-aware**, with the collaborative machinery repurposed: "items that go together" means items that appear together in *the user's* sessions and work, not across users. The signals are also the behavioural layer of [Design: layered search characteristics on Snowflake](layered-search.md) — search and recommender are designed here as one loop, not two systems.

## The signals

Four nominating signals, each cheap, each biased in a different direction — the biases are why the architecture (§ below) unions them rather than picking one.

### 1. The profile centroid (content similarity)

The user profile is a **weighted centroid of embeddings**: take the vectors — from the embedding index Atlas already maintains, see [Embeddings: Snowflake Arctic and the parity contract](../concepts/embeddings.md) — of the documents the user opened, saved or searched for, weight each by the strength of the interaction, **decay the weights with age**, and average. Candidates are the embedding nearest neighbours of that centroid: "more like what I have been working on", with zero new infrastructure beyond the index and the interaction log.

Decay is the load-bearing detail. An undecayed centroid is a portrait of everything the user has ever cared about, dominated by the largest past project; a decaying one tracks the current work, with the half-life a named, tunable parameter. Per-source profiles (one centroid for code work, one for notes, and so on) are a refinement the same shape supports when a single centroid proves too blunt.

### 2. Session co-occurrence (item-to-item, single-user)

Documents opened or cited in the same session or task are related *for this user*, even when their texts are not similar — a specification and the incident log it eventually caused may share almost no vocabulary. Precomputing item→item neighbours offline from the session logs (pairs that co-occur, weighted by how often and how distinctively) is the strongest single-user analogue of Amazon-style item-to-item collaborative filtering, and the signal least replaceable by content similarity: it captures relationships that exist in the user's work rather than in the text.

The raw material is already recorded: every executed search mints a query trace (`atlas-query-trace/v1`) carrying the served results, and feedback records (`atlas-feedback/v1`) attach votes and `opened` flags to `(query_id, chunk_id)` pairs. Session co-occurrence is computed from that history, not from a new tracking system.

### 3. Recency and frequency

Trending-in-my-corpus: recent opens, repeated re-finds, and fresh ingest. A newly ingested document that matches an active interest should be able to surface *before it is ever searched for* — that is the difference between a recommender and a search box. Frequency matters independently of recency: the document returned to every week for a year is infrastructure, and its neighbours are standing recommendations.

### 4. Graph and provenance neighbours

Atlas's corpus is provenance-first — content-addressed objects with occurrence records, and ID-graph edges where the publication format (#6062) lands: see [The corpus: provenance and content addressing](../concepts/corpus-and-provenance.md) and [Design: search integration for fleet and ID](fleet-and-id-integration.md). Neighbours of a known-relevant document along those edges — the next version, the document that derived from it, documents sharing a rare chunk or entity — are candidates no text signal would nominate.

## The architecture: the same three stages as search

Production recommenders and production search have converged on one shape — **candidate generation → ranking → re-ranking** — which is also the shape of Atlas's search pipeline ([Retrieval: hybrid full-text and vector search](../concepts/retrieval.md)). The recommender reuses the stack instead of duplicating it:

1. **Candidate generation (recall).** Union the nominators: embedding kNN against the profile centroid; item-to-item neighbours of recent items; recency/frequency leaders within active sources; graph/provenance neighbours. Each nominator is biased; the union covers the biases. Sensitivity ceilings and tier enforcement apply here, inside retrieval, exactly as in search — a recommendation is a disclosure, and the ceiling discipline of [Sensitivity tiers and client ceilings](../concepts/sensitivity.md) is inherited, not re-implemented.
2. **Ranking (precision).** Score the union with the richest affordable model. The natural Atlas choice is the cross-encoder of [Design: the reranking stage](reranking.md), with the "query" being a profile/session summary — or, before that stage exists, a transparent feature score (similarity, recency, co-occurrence count, source) whose weights are named constants. Either way the ranker is a component with an evaluation (§ below), not a formula beyond inspection.
3. **Re-ranking (the surface).** Enforce what raw relevance ignores: **diversity** via MMR (maximal marginal relevance — each next pick is penalised for near-duplication of picks already made), a **freshness** quota so the feed is not all old hubs, deduplication against already-seen items, and **exploration** slots — a small ε of deliberately novel items, without which the system collapses into a filter bubble of the user's last topic and can never learn that his interests moved.

## The two-tower question: overkill, deliberately

A trained two-tower model learns a user tower and an item tower into a shared embedding space so that retrieval is kNN of the user vector. It is the right tool at multi-user scale. For one user it is overkill *as a trained model* — there is exactly one user vector to learn — but its *pattern* is precisely signal 1 above: the "user tower" collapses to the hand-built, decaying weighted profile centroid. **[Design judgement, not a sourced benchmark claim.]**

So the profile vector is v1, and the upgrade path preserves the serving shape: learned per-source or per-signal weights, or a sequential model over session order (SASRec/BERT4Rec-style "what's next in this session", interesting only once session logs are long and dense), can replace the hand-built centroid *in place* — candidate generation still consumes one profile vector, and no consumer changes. Choosing the simple form first is not a compromise to apologise for; it is the form whose failures can be read directly off its weights.

## Surfaces

- **More-like-this** on any document or session: pure embedding kNN plus the tier filter, with co-occurrence and graph neighbours blended in when they exist. The day-one surface — it needs no profile at all.
- **Related results beside search:** session co-occurrence neighbours of the top search hits, presented as their own rail rather than interleaved into the ranked results, so search quality measurements stay clean.
- **A resurface feed:** documents matching active interests that were recently ingested, or recently *became* relevant (a new link, a new occurrence, a new neighbour), ranked by profile similarity + recency and passed through the MMR/freshness/exploration re-rank. This is the surface that justifies the whole system: it finds things before they are searched for.
- **Query suggestions and zero-result recovery:** from the profile and the co-occurrence table — "you needed X last month alongside this" — offered when a search returns nothing or when a query matches a known past session's shape.

## Recommendations as stored objects, and the "next 10" fan-out (#9109 Part C)

The #9109 Part C design settles the serving shape for the ID-facing surfaces: **one fan-out worker, a Durable Object accumulating a "next 10", and recommendations as stored objects with per-class retention**. Unpacked:

- Recommendations are **materialised objects**, not computed per request: a generation pass produces a ranked, re-ranked list for a subject (a user profile state, a document, a search), stores it as an object with a retention class (#9140's subject), and surfaces page through the stored object. That makes recommendation pages stable, cheap to serve, and auditable — what was recommended, when, from which signals, can be answered after the fact.
- The **fan-out worker** is the single producer: it runs the candidate union across the nominators (and, in the ID shape, across the federated stores) and hands partial results to the accumulator rather than blocking a request on the slowest nominator.
- The **Durable Object accumulates the "next 10"**: as fan-out results arrive, the DO merges, ranks and re-ranks them into the next page of the stored recommendation object, so a consumer paging forward is always reading an accumulated, deduplicated, diversity-checked page — never a raw nominator's output, and never a page that changes shape because a slow nominator landed late.

Paging over a materialised object needs its own discipline, which is implemented by the snapshot paging state machine (§ "What exists today"): an immutable materialised order (score descending, then immutable recommendation ID ascending) at a pinned cutoff, per-page signed envelopes, and a cursor bound to the snapshot it pages.

## Evaluation

The recommender is measured like search, on held-out behaviour: split real sessions, and from each session's first half predict its held-out opens — precision@k and nDCG on that prediction task — plus coverage and diversity measures, so the system cannot score well by merely echoing search results back. A recommender that only recommends what search would have found anyway has added a feed, not a capability. The standing programme that hosts these measurements is [Design: the standing evaluation programme](evaluation-programme.md).

## Evaluating candidate sources and frozen pages

Build each profile from consented, eligible past events in one model space. Keep impressions separate from positive interactions. Cap the influence of long sessions and shrink low-support co-occurrences. On a rolling temporal holdout, expose only items, grants, graph edges and events available before each historical request. Compare recency-only, eligible content-centroid, co-occurrence and graph arms under a fixed candidate budget. Then evaluate the union, ranker and diversity pass separately. Record candidate evidence recall, ranking quality, repeated exposures, distinct-source coverage, within-page similarity, freshness and elapsed time. Include a cold-start cohort, plus a seed-only more-like-this baseline. [Recommendation design](https://github.com/developing-today/code/issues/9140)

| Proposed scenario | Observable oracle |
|---|---|
| Bounded fan-out | Freeze the union at a deadline, apply deterministic total order and sign the snapshot once. A late branch contributes to a future snapshot. |
| Retry and restart | Two requests for the same page produce identical order, and a restart between committing and responding returns the committed page once. Preserve the existing canonical preimage. |
| Scope changes between pages | Revalidate seed, policy, visible generation and deny watermark before replay. Fail a stale lease and let the caller restart under the current scope. |
| Hidden-only graph or feedback changes | A lower-scope caller sees the same order, counts, cursor shape and page sequence in paired synthetic fixtures. Recompute or isolate the eligible profile and edge aggregates. |

The merged snapshot-paging and durable accumulator changes supply paging primitives and replay state. They do not establish a candidate generator, production signer, Durable Object binding or a serving-quality result. Record each of those deployment and evaluation receipts independently before enabling a surface. [Snapshot paging](https://github.com/developing-today/code/pull/12873) · [Durable accumulator](https://github.com/developing-today/code/pull/13353)

## What exists today

- **Candidate generation and recommendation serving remain proposed.** #9140 describes recommendation objects, history and retention. The paging implementations below accept an already-materialized ordering. A producer, published surface and quality receipt are separate deliverables. [Recommendation objects](https://github.com/developing-today/code/issues/9140) · [ID integration](https://github.com/developing-today/code/issues/9109)
- **Snapshot paging and durable replay are merged.** The state machine fixes immutable order at a cutoff and signs page envelopes. The accumulator persists the frozen order and replay state. Production wiring and eligible candidate generation require separate evidence. [Snapshot state](https://github.com/developing-today/code/pull/12873) · [Durable accumulator](https://github.com/developing-today/code/pull/13353)
- **The raw material exists.** Query traces (`atlas-query-trace/v1`) are captured for every executed search, feedback capture (`atlas-feedback/v1`) and its normalised derivative are shipped, and the embedding index the profile centroid would query is the production index. What is missing is every consumer that would turn that material into recommendations — today those records feed statistics, weekly-report nominations and retention protection only.
- **Authorizing a production page** also needs subject scoping and a current policy epoch. Validate live control and revocation before serving an existing signed snapshot. [Subject policy](https://github.com/developing-today/code/issues/9083)
