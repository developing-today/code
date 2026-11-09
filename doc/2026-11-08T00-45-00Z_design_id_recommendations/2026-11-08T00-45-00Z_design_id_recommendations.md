# ID recommendations across all object types — v1 design

Design record for issue #5732 ("Design: recommendations for ID — similar
objects and similar users, across all object types"). Written per
`doc/DOCUMENTATION_PROTOCOL.md` (dated record in `doc/<UTC>_design_<name>/`).
Origin issue: #5732. Extends #5675 (querying / discovery). Adjacent
contracts: #5677 (notes, boards, multiplayer editing — a surface, not this
system), #5669 (atlas staging tier — computation placement), #5683
(residency & boundaries — sensitivity rules). Status: v1 design, settled
except the items in §10. No implementation is authorized by this record
alone; §9 states what implementation must wait for.

---

## 1. Problem

#5675 covers *finding* things in ID when the user knows what they are
looking for (search, local and remote). It does not cover *being shown*
things the user did not ask for: "worlds like this one", "documents people
in your position read next", "users working on similar things". Search
answers intent; recommendations create discovery. #5732 requires this for
**all** first-class ID object types, with similar-user signals in scope
from v1.

Verified at origin/main `bade432d21` (2026-11-08): there is no
recommendation or similarity implementation under `pkgs/id` (the only
`recommend` hits are in vendored p2panda code), and no open PR implements
one. This record is the design the implementation will be built against.

## 2. Proposal in one paragraph

Recommendations are **precomputed typed edges**, never per-request model
work. A batch pass on the staging tier (#5669 pattern) turns interaction
aggregates into edges of one shape — `(subject, recommended object,
signal, score, computed_at)` — promotes them into core-tier tables keyed
by subject, and every surface (a world's sidebar, a document's "read
next", a home/discovery feed, a board header) renders the same edges.
v1 signals are arithmetic only: item–item co-occurrence, user–user
similarity, and a content-based cold start. Privacy is structural:
visibility is filtered **before** ranking, aggregate similarity only,
with a k-anonymity floor and a cheap opt-out (§6).

## 3. Object types and the edge shape

The recommender treats every first-class ID object type uniformly:
**worlds, documents, notes, boards, chats / message threads, programs,
and users**. A user is both a subject (recommendations *for* them) and an
object type (recommendations *of* people: "users working on similar
things" — rendered only as aggregate similarity, never as a history; §6).

Edge record (logical shape; storage in §7):

| Field | Meaning |
|---|---|
| `subject_type`, `subject_id` | The object (or user) the recommendation is attached to. For item–item edges the subject is an object; for similar-user edges the subject is a user and the edge is served only to that user. |
| `object_type`, `object_id` | The recommended object (any first-class type, including a user). |
| `signal` | `cooccur` \| `similar-user` \| `content` — always carried to the surface, so cold-start and social proof can be presented differently. |
| `score` | 0..1, normalized per subject (§4); comparable within one subject's list, not across subjects. |
| `computed_at`, `expires_at` | Batch provenance and staleness bound (§7). Surfaces must not render expired edges. |
| `batch_id` | The staging batch that produced the edge; the join key for evaluation and feedback-loop accounting (§8). |

Invariants: at most one edge per (subject, object, signal); an object is
never recommended to itself; a user's own objects are never recommended
back to that user via `similar-user`.

## 4. Signals (v1 is arithmetic, not a trained model)

Interaction weights (settled for v1): **view = 1, join = 3, edit = 5** —
stronger intent counts more, everywhere below.

1. **Item–item co-occurrence.** For objects A and B, let U(A) be the set
   of users who interacted with A (weighted as above). Score is the
   weighted Jaccard similarity |U(A) ∩ U(B)| / |U(A) ∪ U(B)| over
   decayed interaction weights (§5). This is the primary signal for
   object-subject edges ("worlds like this one").
2. **User–user similarity ("similar users viewed").** For users u and v,
   similarity is the weighted Jaccard over their object-interaction sets.
   u's `similar-user` edges are the objects that u's top similar users
   (similarity ≥ 0.1, top 50) interacted with and u has not seen,
   scored by Σ similarity(u,v) × weight(v's interaction), normalized to
   0..1 per user. In scope from v1 per #5732, weighted alongside
   item–item, not replacing it.
3. **Content-based cold start.** An object with fewer than **5 distinct
   interacting users** has no meaningful co-occurrence: fall back to
   dense cosine similarity over description/content embeddings (the
   atlas machinery), labeled `signal = content`. A new user starts from
   the content similarity of the first objects they touch. Once an
   object crosses the 5-user threshold, co-occurrence edges replace
   content edges for it in the next batch; the two are never blended
   for the same (subject, object) pair.

Non-goals confirmed from #5732: no trained ranking model or matrix
factorization in v1 (revisit only if these signals measurably plateau);
no cross-server federation until #5675's remote querying settles its
sensitivity story — v1 is per-server (per owner-data) only.

## 5. Recency, diversity, and the popularity cap

- **Recency decay.** Every interaction is decayed before aggregation:
  exponential decay with a **30-day half-life for views** and a
  **90-day half-life for joins and edits** (stronger intent ages
  slower). Decay is applied at aggregation time in the staging tier,
  so stored aggregates stay raw counts by day.
- **Diversity (MMR-lite re-rank).** A subject's candidate edges are
  re-ranked with maximal-marginal-relevance at promotion time:
  `final = 0.7 × score − 0.3 × max similarity to an already-selected
  object in this list`. λ = 0.7 is the v1 constant; tuning it is an
  evaluation-driven change (§8), not a taste change.
- **Popularity cap.** An object in the **top 1% by decayed interaction
  count** may occupy at most **20% of the slots** in any one rendered
  list. Pure similarity otherwise collapses into an echo of what is
  already popular — this cap is the minimum anti-echo term #5732 asks
  for, and it is a hard cap, not a soft penalty.

## 6. Privacy and opt-out (hard constraints, from #5683 / #5675)

These are contract terms, not guidelines; an implementation that
violates one is wrong even if its rankings are good.

1. **Filter before ranking.** An object is only ever recommended to
   users who could already access it. The batch computes edges per
   sensitivity/visibility class from #5683, and the serve path
   re-checks the requester's visibility at read time (defense in
   depth). Post-rank filtering is explicitly rejected: it leaks by
   slot absence and wastes the list.
2. **No existence leaks.** Similar-user metrics are computed only over
   interactions with objects visible to the requester. "People like
   you read X" must never reveal a private X, nor that any specific
   user read anything.
3. **Aggregates only, with a k-floor.** No per-user reading history is
   ever surfaced to another user — only aggregate outcomes ("similar
   users viewed"). A `similar-user` edge is emitted only when at
   least **k = 5 distinct users** contribute to its supporting
   aggregate.
4. **Opt-out.** A user may opt out of contributing interaction data to
   similarity computation. Honoring it is cheap by construction:
   opt-out is a flag checked at aggregation time in the staging tier
   (their interactions are excluded from every aggregate) and at
   serve time (no `similar-user` edges are computed *for* them;
   `cooccur` edges attached to public objects still render, since
   those are object properties, not personalization). Opt-out takes
   effect from the next batch; it is not retroactive edge surgery.
5. **No boundary relaxation.** Nothing in this design relaxes a
   sensitivity or encryption boundary from #5683 or #5675; where this
   record and #5683 disagree, #5683 wins and this record gets a dated
   amendment.

## 7. Storage, computation, and refresh (#5669 coordination)

Computation placement follows the staging-tier pattern of #5669:
aggregates accumulate in the staging tier; a batch pass computes
edges; edges are promoted into the core tier keyed by subject, so
reads are simple lookups.

- **Staging (per #5669):** daily interaction aggregates (subject-set
  counts by object, by day, decayed at read), the opt-out registry
  mirror, and per-batch scratch similarity output. Raw per-event logs
  are *not* promoted.
- **Core tables (logical):** `reco_edges` (the §3 shape, keyed by
  (subject_type, subject_id, signal)), `reco_optout` (user, opted_out,
  updated_at), `reco_feedback` (§8; evaluation data, never a ranking
  input).
- **Cadence.** Full recompute **nightly**; an **hourly incremental**
  pass recomputes only subjects whose objects gained interactions in
  the last hour above a noise floor (≥3 decayed weight). `expires_at`
  is `computed_at + 48h`: a surface that finds only expired edges
  renders no recommendations rather than stale ones.
- **Cost bound.** Recompute cost is bounded by the interaction graph's
  active slice (objects/users with decayed weight above the noise
  floor in the last 90 days), never by whole history.

**Boundary with search.** Recommendations are precomputed discovery
edges. #5675 query/search answers an expressed intent at request time;
#5677's surfaces may *render* recommendation edges but own no ranking.
Personalized search ranking (reordering search results per user) is a
different system and is out of scope here — this record neither
defines nor constrains it beyond §6.

## 8. Feedback-loop accounting and evaluation

- **Feedback loop.** When a user follows a recommendation, that
  interaction is recorded in `reco_feedback` with its `batch_id` and
  is **excluded from similarity inputs**. A system that trains on its
  own suggestions manufactures fake consensus; v1 accounting is
  therefore subtractive by construction: recommendation-sourced
  interactions count for evaluation, never for ranking.
- **Evaluation metrics (v1, offline-first).** Primary: **follow-through
  rate** — fraction of rendered edges followed by a join/read/edit of
  the recommended object within 7 days — measured against two
  baselines on the same surfaces: recency-ordered and
  popularity-ordered lists. Secondary guardrails: list coverage
  (fraction of eligible subjects with ≥1 non-expired edge), intra-list
  diversity (mean pairwise dissimilarity after §5 re-rank), opt-out
  rate, and the popularity-cap hit rate. Method: replay on a held-out
  week of interaction aggregates before any surface ships; online
  interleaving is a later decision, not v1.
- **Plateau rule.** The non-goal on trained models (§4) is revisited
  only when follow-through plateaus against both baselines for two
  consecutive evaluation runs — measured, not vibes.

## 9. Dependencies and implementation slices

**Hard prerequisites (implementation must wait):**
- **#5683** — the residency & boundaries framework supplies the
  sensitivity/visibility classes §6 filters by. Until its classes are
  contract-stable, only the aggregation plumbing (slice 1) may be
  built, and it must treat visibility as an opaque class key.
- **#5675** — supplies the visibility semantics ("could already access
  it") and the discovery surfaces this system feeds.

**Coordination points (not blockers):** #5669 (staging placement and
promotion mechanics — this record assumes its pattern, and edge
promotion lands wherever #5669's aggregator lands); #5677 (rendering
surfaces for notes/boards).

**Slices (each its own implementation issue, filed when its
prerequisite lands — not before):**
1. Interaction aggregation + opt-out registry (staging), no ranking.
2. Item–item co-occurrence batch + `reco_edges` promotion + expiry.
3. Similar-user edges with the §6 k-floor.
4. Content cold start via the atlas embedding scan.
5. Serve-path lookup + first surface (world sidebar), with the §6
   serve-time visibility re-check.
6. Evaluation harness (offline replay, baselines, guardrail report).

## 10. Unresolved decisions (open, owned by follow-up design work)

- **Embedding pin for the `content` signal:** which atlas embedding
  model/version, and how model upgrades invalidate old content edges
  (full recompute vs lazy). Blocks slice 4 only.
- **MMR λ and the popularity-cap thresholds** (0.7 / top-1% / 20%) are
  v1 constants chosen conservatively; §8's first evaluation run owns
  revisiting them with data.
- **User-as-object rendering:** how "users working on similar things"
  is presented without implying a history (aggregate counts only?
  shared public objects only?) needs a #5683-consistent ruling before
  slice 3 renders user objects to other users.
- **Cross-server federation** is a stated non-goal for v1 (§4); if
  #5675's remote querying settles its sensitivity story, a v2 record
  supersedes this one per the documentation protocol — this record is
  not amended into a federation design.
