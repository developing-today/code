/**
 * Board composition policy for finished work (#1146).
 *
 * The WIP board is a view of work in flight, not a completion log:
 * - Done items stay on the board for DONE_BOARD_WINDOW_MS (7 days)
 *   after their last update, then drop off entirely — GitHub is the
 *   record of finished work.
 * - Kaneo cards in the retired "ROADMAP Done table" bookkeeping class
 *   (the class Drew killed on 2026-10-26) are filtered out at the
 *   aggregator; the board provenance states this filter.
 * - The renderer collapses the surviving done items behind a
 *   "Done recently (N)" expander (see renderBoardBody).
 *
 * Applied in two places so no board source can bypass it: the Kaneo
 * source filters while assembling the board (sources/live.ts), and
 * renderBoardBody applies it again before rendering.
 */
import type { BoardItem } from "./model.ts";

export const DONE_BOARD_WINDOW_MS = 7 * 24 * 60 * 60 * 1000;

/** Future-stamp tolerance, matching the aggregator-wide 60s convention in feed.ts. */
export const DONE_BOARD_FUTURE_SKEW_MS = 60_000;

/**
 * How many done cards the board renders into the collapsed strip
 * (#1179): the strip is a glance at recent finishes, not the record
 * (GitHub is), and 954 rendered cards cost ~1 MB of HTML on /board.
 * The full strip is one click away via /board?done=all.
 */
export const DONE_STRIP_RENDER_LIMIT = 100;

/** Titles of the retired ROADMAP Done-table bookkeeping class. */
export function isRetiredDoneTableTitle(title: string): boolean {
  return /\broadmap\s+done\s+table\b/i.test(title);
}

/**
 * A done item is recent when its last update is inside the window.
 * An item with no parseable updatedAt is kept: its age is unproven,
 * and dropping undated work silently would be worse than showing it.
 */
export function doneItemIsRecent(item: BoardItem, now: number): boolean {
  if (!item.updatedAt) return true;
  const t = Date.parse(item.updatedAt);
  if (Number.isNaN(t)) return true;
  const ageMs = now - t;
  return ageMs >= -DONE_BOARD_FUTURE_SKEW_MS && ageMs <= DONE_BOARD_WINDOW_MS;
}

/** Drop aged-out done items and the retired Done-table class. */
export function applyBoardDonePolicy(items: BoardItem[], now = Date.now()): BoardItem[] {
  return items.filter((item) => {
    if (item.source === "kaneo" && isRetiredDoneTableTitle(item.title)) return false;
    if (item.status === "done" && !doneItemIsRecent(item, now)) return false;
    return true;
  });
}

/**
 * The done cards that actually render in the collapsed strip (#1179):
 * every surviving done card, or the newest `limit` when the caller has
 * not asked for the full strip (`?done=all`). Kept here next to the
 * age-out policy so the HTML board and `/api/board.json` can never
 * disagree about which done cards are "on the board" — `#5082` was
 * exactly that disagreement: the JSON endpoint shipped every card and
 * every linked item (~2.4 MB) while the page rendered 100.
 */
export function selectVisibleDoneCards(
  done: readonly BoardItem[],
  opts: { showAll: boolean; limit?: number },
): BoardItem[] {
  if (opts.showAll) return [...done];
  const limit = opts.limit ?? DONE_STRIP_RENDER_LIMIT;
  if (done.length <= limit) return [...done];
  return [...done]
    .sort((a, b) => (Date.parse(b.updatedAt ?? "") || 0) - (Date.parse(a.updatedAt ?? "") || 0))
    .slice(0, limit);
}
