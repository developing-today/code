# Personalized search ranking for notes — decision record

Decision record for issue #5836 ("id: design: personalized search ranking
for notes"). Written per `doc/DOCUMENTATION_PROTOCOL.md` (dated record in
`doc/<UTC>_design_<name>/`). Origin issue: #5836, a decomposition child of
#5677 (notes, boards, multiplayer editing v1; decomposition item 7).
Adjacent contracts: #5675 (querying local and remote ID servers — the
search this record ranks), #5732 (recommendations — the discovery system
this record is *not*), #5683 (residency & boundaries — sensitivity rules).
Status: v1 decision, settled except §9. No implementation is authorized
by this record alone; §8 states what implementation must wait for and how
it splits.

Verified at origin/main `d2c2a05b5e` (2026-11-09): no personalized
ranking exists in the repo — `git grep` for "personalized search",
"own-edit-first", and "graffiti" returns only the recommendations
record's out-of-scope note (§7 of
`doc/2026-11-08T00-45-00Z_design_id_recommendations/`), which explicitly
leaves this system to this record.

---

## 1. Problem

#5675 search answers an expressed intent: it returns the objects that
match a query, filtered server-side to what the searcher is cleared to
see. It does not order those matches *for the searcher*. #5677 §6
requires that ordering to be personal: a searcher's own version of an
object leads their own results, ranking beyond that is stated and
ordered, and the whole ranking is explainable. #5677 §7 (sovereignty)
constrains how abuse is handled: anyone may fork or edit anything they
can see, attribution is never erased, and graffiti control operates on
visibility and ranking — never on history. #5836 must settle three
things inside those constraints: own-edit-first ranking, explainability
of ranking, and graffiti demotion without erasure.

## 2. Decisions in one paragraph

Search ranking is a **per-request, per-searcher re-ranking of the
#5675 result set**, never a precomputed list and never a change to what
the query matched. Candidates are visibility- and sensitivity-filtered
*before* ranking (the #5732 §6 rule, adopted unchanged). Ranking is a
**tier order, not a weighted blend**: tier 0 is the searcher's own
versions (own-edit-first, absolute within the candidate set); below it,
results order by base text relevance, then most-correct marks in the
searcher's community scope, then graph proximity, then recency. Every
result carries machine-readable reason codes, and the ordered signal
list in §3 is the complete contract — there are no hidden signals.
Graffiti is handled by **demotion inside the ranking only**: attributed
graffiti marks from the searcher's community scope push an object below
unmarked results for that scope, with the demotion itself explained and
reversible, and with content, attribution, and history untouched.

## 3. The ranking model

### 3.1 Two stages, one owner each

1. **Candidate stage (#5675, unchanged).** The query executes under the
   searcher's clearance; sensitivity filtering happens server-side
   before anything is returned. Ranking receives exactly the filtered
   candidate set. Ranking never adds an object the query did not match,
   never re-adds a filtered object, and learns nothing about withheld
   objects (filtered-result honesty per #5675; post-rank filtering is
   rejected for the same reason #5732 §6 rejects it — it leaks by slot
   absence).
2. **Personal re-rank (this record).** A deterministic re-ordering of
   the candidate set for the requesting searcher, computed at request
   time from the signals in §3.2. For remote search, the re-rank runs
   where the filtered results are assembled for the searcher, using
   only data visible to the searcher.

### 3.2 Tier order (the contract)

Within the candidate set, results sort by the first tier that
distinguishes them:

| Tier | Signal | Rule |
|---|---|---|
| 0 | **Own version** | A version/fork the searcher authored, edited, or forked ranks before every other version of any object. Among the searcher's own versions, most recent edit first. |
| 1 | **Base relevance** | The #5675 query's own relevance score, unchanged. Personalization never promotes a weaker match above a stronger one across tiers 1–4. |
| 2 | **Most-correct marks** | Versions marked most-correct within a community scope the searcher belongs to rank above unmarked versions. Marks from scopes the searcher is not in have no effect on their ranking. |
| 3 | **Graph proximity** | Shorter path in the note↔post↔reply↔edit graph (#5677 §2) to the object or context the searcher navigated from ranks first. No context → signal absent, not zero-distance. |
| 4 | **Recency** | Most recent edit first. Final tie-break: object id, so ordering is total and reproducible. |

This answers #5677's open question — *how strongly does own-edit-first
outweigh most-correct marks?* — **absolutely, by tier separation, for
the searcher's own view only.** A weighted blend is rejected (§6): any
finite weight lets a popular community mark bury the searcher's own
version in their own results, which is exactly what #5677 §6 forbids
("your view of the world leads"). Tier 0 changes nothing for anyone
else: another searcher's ranking of the same query is computed from
their own tiers.

**Bounded personalization.** Re-ranking applies to the whole candidate
set the query returned (search result sets in ID are paginated, scoped
result lists, not web-scale corpora); if a future query surface returns
unbounded sets, the re-rank window is the first 200 candidates by base
relevance and the window is stated in the response — silently ranking
only a prefix is a contract violation.

**Inputs, exhaustively.** Tier signals consume only: the searcher's own
edit/fork provenance, most-correct and graffiti marks (§5) in the
searcher's community scopes, the reference graph, edit timestamps, and
the base relevance score. In particular, **no similar-user or
co-occurrence signal is a ranking input in v1** — that is the
recommendations system's territory (§7), and importing it would make
search ordering depend on other users' behavior in ways §4 cannot
explain.

## 4. Explainability

- **Reason codes on every result.** Each returned object carries
  `rank_reasons`: an ordered list drawn only from `own_version`,
  `base_relevance`, `most_correct_marked` (with the marking scope),
  `graph_proximity` (with the distance), `recent_edit`, and
  `demoted_graffiti` (§5). The human rendering is a direct translation
  ("shown first because it's your edit" — #5677 §6's example).
- **A "why this order" view** is a first-class surface (CLI, API, and
  UI parity per #5677 §2): for any result it shows its tier, the
  signals that fired, and the first tier at which it lost to the result
  above it.
- **Determinism.** Given the same query, searcher, and corpus state,
  ranking is byte-for-byte reproducible. Any future signal that cannot
  be rendered as a reason code cannot be added; adding a signal means
  amending this record with a dated update, not a code change alone.
- **No personalization without provenance.** If provenance for a
  version is unavailable (e.g. a remote server withholds edit
  attribution under its sensitivity rules), that version simply cannot
  fire tier 0 for anyone — absence of a signal is never guessed.

## 5. Graffiti demotion without erasure

Sovereignty (#5677 §7) settles the shape: the community may ignore a
fork as graffiti, and controls operate on ranking, never by erasing
attribution or history. This record settles the mechanics:

1. **A graffiti mark is an attributed action**, recorded in the object's
   changelog like any other mark (#5677 §4): who marked, when, in what
   scope. Anonymous or unattributed marks do not exist.
2. **Demotion threshold (v1).** An object is demoted in a community
   scope when (a) the object's owner or a board/collection admin of a
   board it is posted to marks it graffiti in that scope, or (b) at
   least **3 distinct members** of the scope mark it. A single peer
   mark never demotes for anyone but the marker themselves (rule 4).
3. **Effect: one demotion tier, not removal.** Demoted objects sort
   below every non-demoted candidate for searchers in that scope —
   after tier 4 — with reason code `demoted_graffiti` naming the scope.
   They remain fully reachable: direct links, graph navigation,
   backlinks, and an explicit "show demoted results" toggle all return
   them un-demoted, and their content, author attribution, and
   changelog render unchanged everywhere.
4. **Per-searcher filters compose on top.** A searcher may mute an
   author or a fork lineage *for themselves*; this is a personal tier-0
   inverse (their own view only), recorded as a personal preference,
   never published as a community signal.
5. **Scope is never global.** Demotion exists only relative to a
   searcher or a community scope. Two communities may judge the same
   fork oppositely; both judgments coexist, each applying only to its
   own scope's searchers. There is no server-wide or network-wide
   demotion state.
6. **Reversibility and audit.** Removing marks below the threshold
   un-demotes from the next query — demotion is computed from current
   marks, not stored as a verdict. Because marks are changelog entries,
   coordinated flag abuse is itself visible and attributable; a scope
   that disagrees with a demotion contests the marks on the object's
   talk page (#5677 §4), not by editing ranking state.

## 6. Alternatives rejected

- **One global ranking for all searchers.** Rejected: contradicts
  #5677 §6 directly (ranking is per-searcher) and §7 (authority is
  per-community; there is no global most-correct).
- **Weighted blend of own-edit and community signals.** Rejected in
  §3.2: any blend admits a weighting where the searcher's own version
  loses in their own results.
- **Precomputed per-user rankings.** Rejected: rankings depend on the
  query, the searcher's provenance, and current marks; precomputation
  is either stale or a cache of the deterministic function in §3, which
  needs no separate system. Precomputed *discovery* already has an
  owner — #5732 — and is a different product (§7).
- **Deletion, hiding, or history edits as spam control.** Rejected by
  #5677 §7 outright; demotion (§5) is the strongest ranking action
  available, and visibility restriction remains a *sharing* decision
  made before publication, not a ranking decision after it.
- **A trained/opaque ranking model in v1.** Rejected: it cannot satisfy
  §4 (every signal explainable, deterministic, reproducible). Revisit
  only on measured evidence the tier model fails — the same plateau
  discipline #5732 §8 applies to its own models — and only as a dated
  amendment to this record.
- **Post-rank visibility filtering.** Rejected in §3.1 (leaks by slot
  absence; #5732 §6 rule).

## 7. Boundaries with the adjacent systems

- **#5675 (search/query).** Search owns matching, sensitivity
  classification, server-side filtering, and base relevance. This
  record owns only the per-searcher ordering of what search returns.
  Where this record and #5675's filtering disagree, #5675 wins —
  ranking is downstream of filtering, always.
- **#5732 (recommendations).** Recommendations are **precomputed
  discovery edges** served to a subject; search ranking is a
  **per-request reordering of query results**. They share no inputs in
  v1: `reco_edges` are not a ranking input, and **`reco_feedback` is
  excluded from ranking inputs** — the anti-feedback-loop rule of
  #5732 §8 applies here unchanged, and this record adds its mirror:
  search click/edit outcomes are not fed back into ranking either;
  ranking is a pure function of provenance, marks, graph, and time
  (§3.2), so it cannot train on its own output. Both systems adopt
  filter-before-ranking from #5732 §6. If a future design wants
  discovery signals inside search, it amends both records explicitly.
- **#5677 (parent).** This record re-decides nothing the parent fixed:
  the note↔board reference model, the merge model, the sovereignty
  rule, and the approval flow are inputs, cited not restated. The
  most-correct *marking mechanism* (who may mark, how marks conflict
  across communities) belongs to the parent's decomposition item 4
  (edit/fork graph); this record consumes marks as an input signal and
  states only what ranking does with them. If that design changes mark
  semantics, this record gets a dated amendment.
- **#5683 (residency & boundaries).** Sensitivity and visibility
  classes are #5683's; ranking treats them as opaque and inherits
  #5732 §6.5: where this record and #5683 disagree, #5683 wins.

## 8. Dependencies and implementation slices

**Hard prerequisites (implementation must wait):**

- **#5675** — a working filtered search with a base relevance score;
  ranking has nothing to order before it exists.
- **The edit/fork graph (parent decomposition item 4)** — tier 0 and
  tier 3 consume provenance and graph distance; most-correct marks
  (tier 2) and graffiti marks (§5) are actions that design defines.

**Slices (each its own implementation issue, filed when its
prerequisite lands — not before):**

1. Re-rank function (§3.2 tiers 0–1, 4) over #5675 results, with reason
   codes — no marks, no graph.
2. Explainability surface: "why this order" in CLI/API (§4).
3. Most-correct tier + graph proximity (tiers 2–3), consuming the
   edit/fork graph.
4. Graffiti demotion (§5): marks consumption, demotion tier, personal
   mutes, show-demoted toggle.
5. Evaluation: replay harness comparing tier ranking against
   relevance-only ordering on held-out queries (precision of tier-0
   placement, demotion precision against changelog-adjudicated marks).

## 9. Open questions

- **Cross-community mark display** — when two of the searcher's scopes
  judge the same object oppositely (marked most-correct in one,
  demoted in another), tier 2 fires and §5 demotion also fires; the
  order between them (this record's default: demotion wins, because a
  graffiti judgment is a safety signal and the object remains one
  toggle away) needs a parent-level ruling if #5677 item 4 settles mark
  conflicts differently. Routed to #5677, not forked here.
- **Pagination interaction** — if a query surface paginates *before*
  ranking, tier 0 objects on later pages can never surface first. This
  record's contract (§3.1) requires ranking over the full filtered
  candidate set; a surface that cannot do that must say so in its
  response rather than silently ranking per page. Whether such a
  surface is acceptable at all is a #5675 question, recorded here as
  the dependency.

## Invariants (summary — implementation must hold all of these)

1. Ranking never changes the candidate set: no additions, no removals,
   no leakage of filtered objects or their existence.
2. A searcher's own version outranks every other version *for that
   searcher*, and affects no one else's ordering.
3. Every ordering decision is reproducible from the reason codes; there
   are no signals outside §3.2.
4. Demotion never deletes, hides from direct access, or alters content,
   attribution, or history — and is always scoped, explained, and
   reversible.
5. Neither recommendation edges nor any system's feedback data
   (`reco_feedback`, search interaction outcomes) is a ranking input.
6. Where this record conflicts with #5683 or #5675 on sensitivity and
   filtering, those records win and this one is amended.
