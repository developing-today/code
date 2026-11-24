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

Paging over a materialised object needs its own discipline, which is the one implementation slice already in flight (§ "What exists today"): an immutable materialised order (score descending, then immutable recommendation ID ascending) at a pinned cutoff, per-page signed envelopes, and a cursor bound to the snapshot it pages.

## Evaluation

The recommender is measured like search, on held-out behaviour: split real sessions, and from each session's first half predict its held-out opens — precision@k and nDCG on that prediction task — plus coverage and diversity measures, so the system cannot score well by merely echoing search results back. A recommender that only recommends what search would have found anyway has added a feed, not a capability. The standing programme that hosts these measurements is [Design: the standing evaluation programme](evaluation-programme.md).

## What exists today

- **Design only.** The recommender is #9140 (recommendation objects, search history, retention classes — open) and #9109 Part C (the fan-out worker, Durable Object "next 10" accumulator, recommendations as stored objects with per-class retention — open). **No candidate generation exists**: no profile centroid is computed, no co-occurrence table is built, no recommendation endpoint is served, and no feedback-driven personalisation runs anywhere in the system.
- **One implementation slice in flight:** snapshot paging, #10158 — the state machine for paging a materialised recommendation snapshot (immutable order: score descending, then recommendation ID ascending, at a pinned cutoff; per-page signed envelopes with their own envelope IDs; cursor bound to the parent snapshot ID). Its PR #12873 (`worker/src/recommendation_state.ts` plus tests) is open and unmerged, held in the programme's shepherd queue — it is not shipped behaviour, and this chapter does not describe it as such. Note what it is: paging and envelope machinery for serving an _already-materialised_ snapshot. It generates no recommendations.
- **The raw material exists.** Query traces (`atlas-query-trace/v1`) are captured for every executed search, feedback capture (`atlas-feedback/v1`) and its normalised derivative are shipped, and the embedding index the profile centroid would query is the production index. What is missing is every consumer that would turn that material into recommendations — today those records feed statistics, weekly-report nominations and retention protection only.
- **A recorded dependency:** #10158 carries a hard dependency on #9083 (tenant-scoped subject authorisation and policy epoch) in the ID-facing design line — the paging slice cannot be read as unblocked until that dependency's status is checked.
