# ID personalized search ranking for notes — v1 design

Design record for issue #5836 ("id: design: personalized search
ranking for notes (from #5677)"). Written per
`doc/DOCUMENTATION_PROTOCOL.md` (dated record in `doc/<UTC>_design_<name>/`).
Origin issue: #5836. Parent: #5677 (ID notes, message board, and
multiplayer editing v1 — this record is decomposition item 7 of that
parent). Adjacent contracts: #5675 (querying local and remote ID
servers — the query model this ranking orders), #5732 (recommendations
across all object types — the discovery system this ranking is not),
#5683 (residency & boundaries — sensitivity rules, via #5732 §6).
Status: v1 design, settled except the items in §10. No implementation
is authorized by this record alone; §9 states the implementation
slices and what they wait for.

---

## 1. Problem

#5675 covers *finding* things in ID: a query with expressed intent,
executed locally or remotely under the requester's clearance, with
server-side sensitivity filtering, stable cursor pagination, and
truncation honesty. It deliberately does not settle the *order* a
searcher's results arrive in beyond the query's own ordering rule.
#5677 §6 requires that order to be personalized: a searcher's own
version of an object surfaces first *for that searcher*, ranking is
per-searcher and explainable, and — per the sovereignty rule in
#5677 §7 — graffiti and spam are handled by ranking demotion, never
by erasing attribution or history.

Verified at origin/main `d2c2a05b5e` (2026-11-09): `git grep` for
"personalized search", "own-edit-first", and "graffiti" returns only
the recommendations record's out-of-scope note
(`doc/2026-11-08T00-45-00Z_design_id_recommendations` §7: personalized
search ranking "is a different system and is out of scope here"). No
decision record, branch, or open PR settles #5836. Its only prior
triage (task 10239) was queue-completed with no landed evidence
(done-task-only audit, #7016). This record is that missing decision.

## 2. Proposal in one paragraph

Personalized search ranking is a **request-time reordering of a
query result set, and nothing else**. The query model of #5675
decides membership: visibility and sensitivity filtering happen
first, server-side, before any ranking (the #5732 §6 hard constraint,
adopted verbatim). Ranking then orders that filtered set by a
**closed, arithmetic, lexicographic tier list** (§4) computed at the
point the result set is assembled, from signals that are visible in
the object graph to the searcher: base relevance, the searcher's own
authorship, community most-correct marks, graph proximity, and
recency. Own-edit-first operates primarily as *version selection* —
for one object with several lineages, the searcher's own version is
the version presented to them — and secondarily as a tier boost for
objects they authored. Every result carries a machine-readable
explanation of its position (§6). Graffiti is handled by *demotion*:
attributed, community-scoped demotion marks sink a version within
its relevance band and collapse its presentation, without removing
it from the result set, its attribution, or its history (§7).
Ranking never changes what a query returns, only the order — and
one searcher's order never affects another's.

## 3. Where ranking sits: membership, then order

The pipeline for every search, on every surface, is fixed:

1. **Query execution (#5675 owns this).** The query runs under the
   searcher's clearance; sensitivity and visibility filtering are
   applied server-side before anything leaves the executing server.
2. **Ranking (this record).** The filtered set is ordered by §4.
   Ranking is the ordering function #5675 pushes to the executing
   server; it introduces no new query surface, no new filter, and
   no membership change.
3. **Result shaping (#5675 owns this).** Projection, cursor
   pagination, and truncation honesty apply over the ranked order.
   A cursor binds to the ranked order of its query execution;
   ranking neither promises nor breaks snapshot semantics beyond
   what #5675's pagination design settles.

Two consequences are load-bearing:

- **Filter before ranking, always.** Post-rank filtering is rejected
  for the reason #5732 §6 gives: it leaks by slot absence and wastes
  the list. A result the searcher may not see cannot influence —
  visibly or by its absence — the order of results they may see.
- **Personalization is view-local.** Own-edit-first leads the
  searcher's *own* view of the world (#5677 §6: "without changing
  anyone else's ranking"). There is no global personalized order,
  no cached cross-user ranked list, and no signal in §4 that is
  computed from one user's behaviour for another user's ranking.

## 4. The ranking function: closed tiers, arithmetic only

Within the filtered result set, ordering is lexicographic over
these tiers, highest first. The list is **closed for v1**: adding
a signal requires a dated amendment to this record.

0. **Relevance band.** The base relevance of the result to the
   query, in the coarse bands the underlying search produces
   (e.g. exact / strong / weak match). Personalization never lifts
   a weak match above a strong one: own-edit-first orders
   *comparable* results, it does not override expressed intent.
1. **Own version / own authorship.** Within a band: results whose
   presented version (§5) is the searcher's own edit or fork, then
   objects the searcher authored.
2. **Most-correct.** Results carrying a most-correct mark effective
   in a community the searcher belongs to (mark mechanics are owned
   by the edit/fork graph design, sibling decomposition item 4,
   #5833; this record consumes marks, it does not define them).
3. **Graph proximity.** Shorter graph distance from the searcher's
   context object (the note/board/conversation the search was
   launched from), per the graph model (#5831, closed). No context
   object → this tier is uniformly neutral.
4. **Recency.** More recent last-edit first.
5. **Deterministic tie-break.** Object id, ascending. The full
   order is reproducible from the explanation data alone (§6).

Demotion (§7) applies *inside* this list: a demoted version sinks
below every non-demoted result **in the same relevance band**, and
never crosses a band boundary in either direction.

v1 is arithmetic only — tiers and comparisons, no trained model
and no numeric weights to tune. This mirrors #5732 §4's stance for
recommendations and is what makes §6's explainability promise
affordable: a lexicographic order can be explained exactly; a
learned score cannot.

## 5. Own-edit-first, settled

#5677's open question — "how strongly does own-edit-first outweigh
most-correct marks from the community?" — is answered by splitting
the two places "first" can mean something:

- **Version selection (own-edit-first is absolute).** When a query
  result is an object with multiple lineages/versions, the version
  *presented as the result* to a searcher is, in order: the
  searcher's own version if one exists and is visible in the query
  scope; else the most-correct-marked version effective for the
  searcher; else the object's default/effective version. Your view
  of the world leads — for you. This never changes which version
  anyone else is presented, and never changes the object's
  community-effective version.
- **Object ordering (own-edit-first is a tier, not a pin).** Across
  distinct objects, own authorship is tier 1 of §4: below relevance
  band, above most-correct. An irrelevant own fork does not outrank
  a relevant result; a relevant own edit outranks an equally
  relevant community favourite *in the searcher's own results only*.

Own-edit-first applies only to versions the assembling server can
see under the searcher's clearance. A searcher's fork held on a
peer the query does not reach cannot be pinned, and the explanation
(§6) states the scope the ranking ran over rather than implying a
completeness it does not have (truncation/scope honesty, #5675 §6).

## 6. Explainability

Every result, on every surface (CLI, scripting/API, agent — parity
per #5675 §4), carries a ranking explanation:

- **Reason codes**, in tier order, naming exactly the tiers that
  determined the result's position relative to its neighbours:
  `own_edit`, `own_authored`, `most_correct:<community>`,
  `graph_proximity:<distance>`, `recency`, `demoted:<reason-code>`
  (§7), plus the relevance band. A human surface renders these as
  text ("shown first because it's your edit" — #5677 §6); the codes
  are the contract, the sentence is a rendering.
- **No invisible reasons.** An explanation may only cite signals
  the searcher is allowed to see. A signal derived from content
  invisible to the searcher may not move a result *and* may not be
  hinted at; per §3 such content is filtered before ranking, so
  this situation cannot arise for membership-derived signals, and
  mark/proximity signals are graph-visible by construction.
- **The unpersonalized view is one flag away.** A searcher may run
  any query with personalization off (`ranking=off`): order is
  relevance band, then most-correct default presentation, then the
  deterministic tie-break. This is both the opt-out (§8) and the
  audit tool: the difference between the two orders is exactly the
  personalization, and both are explainable in the same codes.

## 7. Graffiti demotion without erasure

The sovereignty rule (#5677 §7) fixes the shape: anyone may fork or
edit anything they can see; provenance is attributed and append-only;
the community's remedies against graffiti are adoption decisions and
*visibility/ranking* controls — "owners cannot prevent forks of
visible objects, only decline to adopt them". Demotion is therefore
designed as a ranking act with the same ethics as a mark:

- **What demotion is.** A demotion is an attributed graph event in
  the same family as a most-correct mark: a named actor, in a named
  community scope, at a timestamp, with a reason code — recorded on
  the object's changelog (sibling item 5, #5834) like every other
  graph act. Its effect: within the demoting community's scope, the
  version sinks per §4 and its presentation collapses (grouped under
  an expandable "demoted versions" affordance, count disclosed).
- **What counts as a demotion input (closed list, v1).**
  (a) An explicit demotion mark by an actor with mark standing in
  that community (standing mechanics: #5833, as for most-correct);
  (b) a draft of that version rejected under the approval flow
  (sibling item 6, #5835) — a recorded community act, effective
  only in the approving community's scope;
  (c) the searcher's own per-user filter/mute list — effective only
  in that searcher's view, applied locally, never transmitted to a
  remote executing server (a preference list is the searcher's
  private data; sending it would leak it).
  Absence of adoption is **not** a demotion input: an ignored fork
  is neutral (tier order decides), not graffiti. Unpopularity is
  not a signal.
- **What demotion never does.** It never removes a version from a
  result set (membership is the query's, §3), never crosses a
  relevance band, never applies outside its community/searcher
  scope, never edits attribution or history, and never hides the
  fact it happened — the `demoted:<reason-code>` explanation and
  the disclosed demoted count are part of the result.
- **Reversal.** Adoption reverses demotion in the same scope: a
  most-correct mark or an approval adoption of a demoted version
  cancels the demotion effect (the events both stay in history;
  the *effect* is what ends). There is no appeal queue in v1
  because there is nothing irreversible to appeal.
- **One community's graffiti is another's authority.** Because
  authority is per-community by design (#5677 non-goals), demotion
  has no global form in v1. A version demoted in every community
  the searcher belongs to still appears — demoted, explained,
  expandable — in that searcher's results.

## 8. Privacy and opt-out (hard constraints)

Adopted from #5732 §6, restated for ranking; where this record and
#5683 disagree, #5683 wins and this record gets a dated amendment.

1. **Filter before ranking** (§3). No exceptions, no post-rank
   filtering.
2. **No existence leaks.** Order, explanations, demoted counts, and
   pagination behaviour are computed only over the searcher's
   filtered set. Nothing in a ranked result reveals an object,
   version, mark, or user the searcher cannot already see.
3. **No cross-user behaviour signals.** Ranking inputs are the
   searcher's own authorship and graph-visible marks/proximity —
   never another user's reading or interaction history. Similar-user
   signals belong to recommendations (#5732) and are out of scope
   here even as an input.
4. **`reco_feedback` is not a ranking input.** Recommendation
   feedback is evaluation data (#5732 §8); using it to rank search
   results would close a feedback loop neither record can explain.
   The same applies to search click/order feedback in v1: clicks
   are not a §4 tier.
5. **Opt-out is total and cheap.** `ranking=off` (§6) disables all
   personalization for a query; a per-user setting disables it by
   default. Own-edit *version presentation* (§5) under
   `ranking=off` falls back to the most-correct/default version.

## 9. Boundaries, dependencies, and implementation slices

**Boundary with recommendations (#5732).** Recommendations are
precomputed discovery edges rendered on surfaces; search ranking is
request-time ordering of an expressed-intent query result.
Recommendation edges are **not** a ranking input, surfaces that
render both must label which system produced an ordering, and the
two systems share only the §8 hard constraints. Where #5732 §7 said
it "neither defines nor constrains" this system beyond its §6, this
record accepts exactly that constraint set (§8) and no more.

**Boundary with query/search (#5675).** #5675 owns query shape,
filtering, surfaces, shaping, and pagination; this record owns only
the ordering function applied between filtering and shaping (§3).
Nothing here adds a query capability, weakens the owner-override
boundary, or gives any surface a broader order for one identity
than another with the same clearance and personal signals.

**Parent decisions respected (#5677).** The note↔board reference
model, the merge model, the sovereignty rule, and the approval flow
are inputs, not re-decisions: version presentation (§5) presumes
the reference model; demotion-by-rejection (§7) presumes the
approval flow's recorded outcomes; the whole record presumes
sovereignty. Sibling states at writing: #5830 CLOSED, #5831 CLOSED,
#5832 OPEN, #5833 OPEN, #5834 OPEN, #5835 OPEN. This
record consumes #5833's marks and #5834's changelog as interfaces;
if those designs land with different mark mechanics, this record
gets a dated amendment, not a silent reinterpretation.

**Implementation slices (follow-on work, not design splits).**
(1) Ranking function + explanation codes in the query serve path,
local first. (2) `ranking=off` and the per-user default setting.
(3) Demoted presentation (collapse + count) on human surfaces.
(4) Remote-execution composition: §4 tiers computed by the
executing server from graph-visible signals; per-user filter lists
(c) applied at the searcher's own assembly point only.
No slice starts before #5675's ordering/pagination shape lands;
slices are filed as implementation tickets when that dependency is
scheduled — the design does not split further.

## 10. Unresolved decisions (open, owned elsewhere)

1. **Mark standing and cross-community mark conflicts** — who may
   mark most-correct or demote, and how conflicting marks display,
   is #5677's existing open question and #5833's design surface.
   This record depends on the answer (it consumes marks) but does
   not fork it: until it lands, §4 tier 2 and §7 input (a) operate
   with whatever mark set the graph exposes as effective for the
   searcher, and this dependency is recorded here and routed to
   parent #5677, not silently assumed.
2. **Federated own-edit pinning** — when a searcher's own fork lives
   on a peer outside the query's reach, v1 ranks only what the
   assembling server can see (§5) and says so. Whether a future
   federated assembly merges ranked sets across peers is a #5675 /
   #5677 question; v1 explicitly does not merge ranked sets, because
   merging two lexicographic orders without a shared relevance band
   would break §6's reproducibility.

## 11. Invariants implementation must hold

1. Ranking never changes result membership — only order and
   presented version.
2. Filtering (visibility + sensitivity) completes before ranking
   begins, on every surface and execution placement.
3. A searcher's presented version of an object is their own visible
   version when one exists (personalization on); otherwise the
   most-correct/default version.
4. One searcher's ranking, marks-in-view, and filters never alter
   another searcher's order.
5. Every position in a ranked result is reproducible from its
   explanation codes plus §4; the order is deterministic.
6. Demotion never erases: membership, attribution, and history are
   preserved; demotion is scoped, explained, counted, and reversed
   by adoption in the same scope.
7. No ranking output (order, explanation, count, timing of pages)
   leaks the existence of anything filtered out.
8. The §4 signal list is closed: `reco_feedback`, click feedback,
   and cross-user behaviour are not ranking inputs in v1.
9. `ranking=off` always exists and yields the §6 unpersonalized
   order; personalization is never a condition of getting results.

---

*Recorded 2026-11-09 by repo queue worker lane code-h, task 13997,
for issue #5836. Parent #5677 and siblings #5830–#5835 as cited;
recommendations boundary per the landed #5732 record (PR #6999,
merged 2026-11-08).*
