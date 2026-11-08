/**
 * The `/api/board.json` body (#5082).
 *
 * The endpoint is the machine twin of the `/board` page, so it must
 * describe exactly the board the page shows and nothing more:
 *  - the same done policy (aged-out done cards dropped, the retired
 *    ROADMAP Done-table class filtered) and the same collapsed-strip
 *    limit as the renderer, so a ~2 900-card completion log never
 *    ships as "the board";
 *  - only the items the visible cards actually link to. Every card
 *    already carries its own `links`, so the `items` list is a
 *    resolution lookup for those links, not a dump of every ref the
 *    board has ever touched.
 *
 * Before this the endpoint returned every card plus its full item
 * closure (~2.4 MB), past the aggregator's own 2 MB stale-response
 * cache limit, and the assembly was slow enough to trip the 10 s
 * request deadline. `showAll` (`?done=all`) deliberately restores the
 * full done strip, matching `/board?done=all`.
 */
import {
  applyBoardDonePolicy,
  selectVisibleDoneCards,
} from "./board-policy.ts";
import type { BoardItem, GhItem, RefKey } from "./model.ts";

export interface BoardJsonBody {
  board: BoardItem[];
  items: GhItem[];
}

/** The bounded board + the items its cards resolve. */
export function boardJsonBody(
  board: readonly BoardItem[],
  items: ReadonlyMap<RefKey, GhItem>,
  opts: { showAllDone?: boolean; limit?: number } = {},
): BoardJsonBody {
  const kept = applyBoardDonePolicy([...board]);
  const visibleDone = selectVisibleDoneCards(
    kept.filter((item) => item.status === "done"),
    { showAll: opts.showAllDone ?? false, ...(opts.limit !== undefined ? { limit: opts.limit } : {}) },
  );
  const visible = [...kept.filter((item) => item.status !== "done"), ...visibleDone];
  const needed = new Set<RefKey>();
  for (const card of visible) for (const link of card.links) needed.add(link.ref);
  // Preserve board first-seen order for a stable machine diff.
  const itemsOut: GhItem[] = [];
  for (const ref of needed) {
    const item = items.get(ref);
    if (item) itemsOut.push(item);
  }
  return { board: visible, items: itemsOut };
}

/** The items map restricted to the cards the bounded body carries. */
export function boardJsonItemsMap(body: BoardJsonBody): Map<RefKey, GhItem> {
  return new Map(body.items.map((item) => [item.ref, item] as const));
}
