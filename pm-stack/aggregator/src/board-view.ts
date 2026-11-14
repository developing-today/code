// Shared saved-view filtering for the board surfaces (#1276, #8521).
//
// /board and its Datastar fragment twin /frag/board MUST agree on
// what a view keeps: PR #4013 applied resolveView to /board (and the
// /api/*.json twins) but never to /frag/board, so a saved view that
// filtered the full page was silently dropped on the fragment path
// (#8521). Both handlers now compose through filterBoardForView —
// the twin agreement is by construction, and the tests in
// test/board-view-8521.test.ts pin the semantics.

import type { BoardItem, GhItem, ProjectPoolItem, RefKey, RoadmapRow } from "./model.ts";
import { itemMatchesView, type ViewFilters, type ViewItemLike } from "./views.ts";

/** The ViewItemLike projection of a canonical GitHub item: the
 *  item's own project fields win, the pool row fills the gaps. */
export function ghItemLike(item: GhItem, pool?: ProjectPoolItem | null): ViewItemLike {
  return {
    ref: item.ref,
    repo: item.ref.replace(/#\d+$/, ""),
    area: item.projectArea ?? pool?.area ?? null,
    horizon: item.projectHorizon ?? pool?.horizon ?? null,
    owner: item.author,
    state: item.state,
    epic: pool?.epicRef ?? null,
    status: pool?.status ?? null,
    kind: pool?.kind ?? null,
  };
}

/** A board card / roadmap row matches when its own fields match, or
 *  when any canonical item it links to matches. */
export function linkedLike(
  own: ViewItemLike,
  links: readonly { ref: RefKey }[],
  items: Map<RefKey, GhItem>,
  poolByRef: Map<RefKey, ProjectPoolItem>,
  filters: ViewFilters,
): boolean {
  if (itemMatchesView(own, filters)) return true;
  for (const l of links) {
    const item = items.get(l.ref);
    if (item && itemMatchesView(ghItemLike(item, poolByRef.get(l.ref)), filters)) return true;
  }
  return false;
}

/** The /board view rule (#4013), shared verbatim with /frag/board
 *  (#8521): keep a card when the linkedLike rule matches it; the
 *  items map keeps every item a kept card links to (its metadata
 *  renders on the card even when the item fails the filter itself)
 *  plus every item that matches the view directly. */
export function filterBoardForView(
  board: readonly BoardItem[],
  items: Map<RefKey, GhItem>,
  poolByRef: Map<RefKey, ProjectPoolItem>,
  filters: ViewFilters,
): { board: BoardItem[]; items: Map<RefKey, GhItem> } {
  const keptBoard = board.filter((b) =>
    linkedLike({ ref: b.id, repo: b.repo, state: b.status }, b.links, items, poolByRef, filters),
  );
  const keptRefs = new Set<string>();
  for (const b of keptBoard) for (const l of b.links) keptRefs.add(l.ref);
  const keptItems = new Map(
    [...items].filter(([ref, it]) => keptRefs.has(ref) || itemMatchesView(ghItemLike(it, poolByRef.get(ref)), filters)),
  );
  return { board: keptBoard, items: keptItems };
}

/** The /roadmap view rule (#4013), shared verbatim with its JSON
 *  twin /api/roadmap.json (#8723): keep a row when the linkedLike rule
 *  matches it; the items map keeps every item a kept row links to
 *  (its metadata renders in roadmapCard's links/badges even when the
 *  item fails the filter itself -- Codex P2, PR #4013) plus every
 *  item that matches the view directly. The HTML handler has applied
 *  this rule since #4013; the JSON handler filtered items by the
 *  view alone, so a kept row could reference items absent from its
 *  own payload. Both handlers now compose through this function --
 *  the twin agreement is by construction. */
export function filterRoadmapForView(
  rows: readonly RoadmapRow[],
  items: Map<RefKey, GhItem>,
  poolByRef: Map<RefKey, ProjectPoolItem>,
  filters: ViewFilters,
): { rows: RoadmapRow[]; items: Map<RefKey, GhItem> } {
  const keptRows = rows.filter((r) =>
    linkedLike(
      { ref: r.subject, area: r.area, horizon: r.horizon, state: r.status },
      r.links,
      items,
      poolByRef,
      filters,
    ),
  );
  const keptRefs = new Set<string>();
  for (const r of keptRows) for (const l of r.links) keptRefs.add(l.ref);
  const keptItems = new Map(
    [...items].filter(([ref, it]) => keptRefs.has(ref) || itemMatchesView(ghItemLike(it, poolByRef.get(ref)), filters)),
  );
  return { rows: keptRows, items: keptItems };
}

/** The view-naming query params of a request (view id, viewer, vf.*
 *  filter params), re-encoded — the params a fragment URL must carry
 *  so a refresh re-resolves the same view instead of dropping it.
 *  `done` rides along too (#7937): ?done=all is board view state the
 *  same way a saved view is, and a fragment refresh that dropped it
 *  silently reverted the show-all Done strip to the 100-card cap. */
export function viewParamsQuery(params: URLSearchParams): string {
  const out = new URLSearchParams();
  for (const [k, v] of params) {
    if (k === "view" || k === "viewer" || k === "done" || k.startsWith("vf.")) out.set(k, v);
  }
  return out.toString();
}

/** The /frag/board URL for a request: bare when no view is named,
 *  view-carrying otherwise (#8521). */
export function fragBoardUrl(params: URLSearchParams): string {
  const q = viewParamsQuery(params);
  return q ? `/frag/board?${q}` : "/frag/board";
}
