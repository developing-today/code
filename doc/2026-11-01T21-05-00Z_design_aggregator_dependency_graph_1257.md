# Dependency graphs across the PM stack

**Status:** proposed design, based on current `main` (`70a595a7735c208ccbb1ad0de115b3156cd572d4`)  
**Tracks:** [#1257](https://github.com/developing-today/code/issues/1257) · [epic #1184](https://github.com/developing-today/code/issues/1184)

## Decision summary

Build a dependency view **per selected epic**, using GitHub's native issue
`blockedBy`/`blocking` relations and the PM Project's native issue hierarchy
and fields. Do not infer edges from prose. On wide screens show a navigable
graph beside its summary and ranked work list; on phones make the same graph
available as a readable, vertically ordered list with explicit dependency
links. Keep the primary ranking about **how much open work depends on a
blocker**, not about its age or the length of one chain. Treat incomplete
external references as unresolved, and put all writes through the dashboard's
existing action store and fleet consumers.

The existing longest-chain summary remains useful context, but is not the
primary "unblock this next" ranking: chain length measures serial depth, not
the amount of work exposed by a blocker.

## Current foundation and source of truth

The collector already requests native GitHub `parent`, `blockedBy`, and
`blocking` issue edges in the Projects query
([`projects.ts:32-45`](../pm-stack/aggregator/src/projects.ts#L32-L45));
the parser canonicalizes those refs and carries them on each pool item
([`projects.ts:114-165`](../pm-stack/aggregator/src/projects.ts#L114-L165)).
Epic membership is resolved only after the complete pool is parsed, so a
parent on another Project page can still be recognized
([`projects.ts:170-200`](../pm-stack/aggregator/src/projects.ts#L170-L200)).
The model therefore has an authoritative graph source already: native
relations plus Project fields. This is the source contract landed by #2399;
the design does not reopen that decision.

`graph.ts` already computes blocked counts/effort and a cycle-safe longest
blocked-by chain. A ref outside the loaded pool is unresolved, never silently
treated as done ([`graph.ts:28-48`](../pm-stack/aggregator/src/graph.ts#L28-L48),
[`graph.ts:50-106`](../pm-stack/aggregator/src/graph.ts#L50-L106)).
`depthLayout` currently lays out **parent hierarchy** as depth and row, caps
the SVG at 120 nodes, and degrades parent cycles to roots rather than looping
([`graph.ts:125-193`](../pm-stack/aggregator/src/graph.ts#L125-L193)).
The program page renders the blocked summary and that parent-depth SVG
([`render.ts:5463-5520`](../pm-stack/aggregator/src/render.ts#L5463-L5520));
it is not yet a per-epic dependency graph. Epic rollups already use explicit
`epicRef` membership and treat a missing blocker as unresolved
([`epics.ts:14-55`](../pm-stack/aggregator/src/epics.ts#L14-L55)).

Prose `blockedBy` values from other feeds are descriptive source data, not
edges in this graph. Do not fuzzy-match titles or parse prose into live
relations. Where old prose can be mapped confidently, a separate migration
may suggest the native GitHub relation for a human to confirm; until that
relation appears in the authoritative feed, the graph must say that no native
edge was observed.

## Views and layout

- The entry point is a list of epics with item/blocked counts. Selecting an
  epic filters the graph to that epic's open and closed pool items. Do not
  render one global all-project graph by default: it would mix unrelated
  work, overwhelm the existing per-project view, and make per-epic ranking
  hard to interpret.
- Include a one-hop blocker reference even when it lies outside the selected
  epic. Render it as an external node, keep its canonical `owner/repo#number`
  identity, and make it a link. Do not recursively fetch the external
  project's graph in this slice.
- On desktop, show the ranked blocker list and summary adjacent to the graph.
  The dependency edges must be visually distinct from parent/sub-issue edges;
  reuse of the existing parent-depth SVG without that distinction would
  misrepresent the data. Keep titles and issue links on nodes, with a separate
  text/list representation for assistive technology.
- On a 390px phone, prioritize the epic selector, summary, and a vertical
  dependency list: each item names its blockers and dependents as links, with
  cycle/external/open/closed state written as text rather than color alone.
  The graph may be a horizontally scrollable secondary detail, but the
  ranked list and its actions must not require sideways scrolling.
- Keep a visible truncation/count notice when the existing 120-node graph
  cap is reached (or a lower per-epic cap is selected). Never imply that a
  capped graph is complete. Collapsing completed branches is a presentation
  choice; it must not change counts or ranking calculations.

## Criticality and "unblock next" ranking

**Primary ranking: most distinct open epic items with the candidate in their
transitive blocker ancestry.** For each unresolved blocker, traverse the
reverse dependency graph and count each open item in the selected epic once.
This measures the amount of open work exposed downstream, which is the
question #1257 asks the dashboard to answer. It does not claim that every
counted item becomes immediately unblocked: if an item also has another open
or unknown blocker, show that remaining blocker count beside it and label the
ranked number "open downstream items" (not "will be freed").

Order by descending distinct-item count, then descending sum of known effort
weights for those items (continue to report unsized items separately), then
canonical ref for a stable tie. Show the contributing items so a person can
inspect the number. Closed/merged items do not contribute. An unknown external
blocker remains eligible as a referenced node but is visibly marked unknown;
it is not counted as resolved. The ranking is per epic and computed from the
same snapshot as the graph.

Why this choice: the existing longest chain is valuable structural context
and already exists, but two blockers on equally long chains can expose very
different amounts of work. Oldest-blocker ranking is also a poor proxy for
leverage and current collection does not provide issue creation time.
Transitive dependent count directly explains why an item ranks highly. Keep
the existing longest chain as a secondary diagnostic, explicitly named
"longest dependency chain," not as a second competing primary rank.

## Cycles and dangling references

- Detect strongly connected components in the **dependency** graph; the
  existing cycle guard in `blockedSummary` only ensures a finite chain and
  does not identify or explain a cycle to the user. Render a cycle as a
  labelled group listing every member and its links, with a warning that the
  dependency data is contradictory. Preserve edge direction in the detail
  list, but do not select an arbitrary member as the "root" or claim that
  resolving one member frees the group. Exclude the cyclic component from
  positive unblock counts until its edges are corrected; surface it as an
  anomaly requiring review.
- A ref missing from the current Project pool is **outside/unresolved**, not
  done. Show its canonical ref as a linked stub with status "not in this
  pool / status unknown." If the collector has a trustworthy closed state,
  show that state and treat it as resolved; absence alone never resolves it.
- Distinguish a missing or malformed edge endpoint (discarded by the parser)
  from a valid external ref: report the former as unavailable edge data and
  the latter as an external node. Do not fabricate a title or status for
  either.

## Actions from the graph

The dashboard is not a direct GitHub/Project writer. All controls post to the
existing `POST /api/actions` path; the action store persists the record and
the existing fleet consumer applies or rejects it
([`actions.ts:67-114`](../pm-stack/aggregator/src/actions.ts#L67-L114),
[`server.ts:555-585`](../pm-stack/aggregator/src/server.ts#L555-L585)).
For a graph item already present in a published Needs-Drew/Needs-Orchestrator
tier, reuse the same triage form and `objectRef`/validation path as the board
([`render.ts:1080-1137`](../pm-stack/aggregator/src/render.ts#L1080-L1137)).
For a session-backed item, only show the existing session action when its
source supports it. Do not invent a graph-only direct write or bypass the
published-item check. If a blocker is not action-eligible, link to its normal
board/GitHub detail and state why no inline action is available.

The first implementation should reuse supported actions (for example,
triage escalation/delegation for an eligible tier item), preserve pending
and rejected status, and show the consumer's result. A new issue-targeted
dispatch or poke verb requires an explicit action schema, consumer, and
audit contract; it is not implied by drawing a graph button.

## Needs Drew

These are the only product-policy choices left for Drew; implementation can
proceed with the recommendations above unless he overrides them:

1. **Ranking meaning:** confirm "most distinct open downstream items" as the
   primary unblock ranking (with longest chain secondary), or choose a
   different primary interpretation. Recommendation: confirm this choice;
   item count, known effort, and a stable ref tie-break are explainable from
   the current data.
2. **Cross-project expansion:** confirm that this release stays within the
   selected epic/Project pool and represents out-of-pool blockers as one-hop
   unresolved links, rather than recursively loading every linked project.
   Recommendation: keep the bounded pool-only view.
3. **Inline action eligibility:** confirm that graph controls are limited to
   items already eligible in the existing published action pipeline, with
   other blockers linking to their normal detail, rather than adding a new
   direct issue-dispatch API in this release. Recommendation: reuse existing
   actions first; any new verb gets its own audited design and implementation.

Responsive layout, explicit cycle/external states, and no inference from
prose are implementation defaults, not additional Drew gates.

## Implementation breakdown

The linked implementation issues are children of epic #1184 and each links
back here:

- [Per-epic responsive dependency graph](https://github.com/developing-today/code/issues/3973)
- [Rank blockers by downstream open work](https://github.com/developing-today/code/issues/3974)
- [Explain cycles, external refs, and action eligibility](https://github.com/developing-today/code/issues/3975)

## Change log

- 2026-11-01: initial proposed design from the #1257 acceptance questions and
  current-main evidence. No implementation is included in this note.
