// Complexity weights for Project-pool items (#1108 slice 1).
//
// Progress on the dashboard must reflect complexity, not issue counts
// alone. Weights come from the item's `effort/*` labels:
//
//   effort/xs = 1 · effort/s = 4 · effort/m = 12 · effort/l = 40
//
// (`effort/l` is flagged open-ended in #1108 — it is a floor, not a cap.)
// An item with no recognised effort label is UNSIZED: it contributes no
// weight and is counted separately in every summary, never silently
// zero-weighted — a page full of unsized items reads 0% weighted with a
// visible unsized count, not a false 100%.
//
// Pure module: no I/O, no feed access. Callers pass pool items (or any
// `{ state, labels }` shape) in.

import type { ProjectPoolItem } from "./model.ts";

export const EFFORT_WEIGHTS: Readonly<Record<string, number>> = {
  xs: 1,
  s: 4,
  m: 12,
  l: 40,
};

/** The effort weight of one label set, or null when unsized. When more
 * than one `effort/*` label is present the first recognised one wins —
 * labels are single-valued in the taxonomy, and a duplicate is a data
 * smell the summary's unsized/duplicate handling should not hide by
 * summing. */
export function effortWeight(labels: readonly string[] | undefined): number | null {
  if (!labels) return null;
  for (const raw of labels) {
    const label = raw.trim().toLowerCase();
    if (!label.startsWith("effort/")) continue;
    const size = label.slice("effort/".length);
    const w = EFFORT_WEIGHTS[size];
    if (w !== undefined) return w;
  }
  return null;
}

/** The `mode/*` value of a label set ("physical" | "hybrid" | "online"
 * | any other mode/* suffix), or null when the item carries no mode. */
export function modeOf(labels: readonly string[] | undefined): string | null {
  if (!labels) return null;
  for (const raw of labels) {
    const label = raw.trim().toLowerCase();
    if (label.startsWith("mode/") && label.length > "mode/".length) {
      return label.slice("mode/".length);
    }
  }
  return null;
}

export interface WeightSummary {
  /** Number of items in the set. */
  count: number;
  closedCount: number;
  openCount: number;
  /** Sum of effort weights over sized items. */
  totalWeight: number;
  closedWeight: number;
  openWeight: number;
  /** closedWeight / totalWeight × 100, or null when nothing is sized. */
  weightedPct: number | null;
  /** Items with no recognised effort label — counted, never zeroed. */
  unsizedCount: number;
  unsizedOpenCount: number;
  unsizedClosedCount: number;
  /** Hands queue: OPEN sized weight whose mode is physical or hybrid —
   * work waiting on hands. */
  handsWeight: number;
  /** OPEN sized weight whose mode is online. */
  onlineWeight: number;
  /** OPEN sized weight with no mode/* label. */
  unmodedOpenWeight: number;
}

type WeightItem = Pick<ProjectPoolItem, "state" | "labels">;

export function summarizeWeights(items: readonly WeightItem[]): WeightSummary {
  const out: WeightSummary = {
    count: items.length,
    closedCount: 0,
    openCount: 0,
    totalWeight: 0,
    closedWeight: 0,
    openWeight: 0,
    weightedPct: null,
    unsizedCount: 0,
    unsizedOpenCount: 0,
    unsizedClosedCount: 0,
    handsWeight: 0,
    onlineWeight: 0,
    unmodedOpenWeight: 0,
  };
  for (const item of items) {
    const open = item.state === "open";
    if (open) out.openCount += 1;
    else out.closedCount += 1;
    const w = effortWeight(item.labels);
    if (w === null) {
      out.unsizedCount += 1;
      if (open) out.unsizedOpenCount += 1;
      else out.unsizedClosedCount += 1;
      continue;
    }
    out.totalWeight += w;
    if (open) {
      out.openWeight += w;
      const mode = modeOf(item.labels);
      if (mode === "physical" || mode === "hybrid") out.handsWeight += w;
      else if (mode === "online") out.onlineWeight += w;
      else out.unmodedOpenWeight += w;
    } else {
      out.closedWeight += w;
    }
  }
  out.weightedPct =
    out.totalWeight > 0 ? Math.round((out.closedWeight / out.totalWeight) * 1000) / 10 : null;
  return out;
}

/** One-line weighted-progress text: weighted % first, raw counts second,
 * unsized always visible. Shared by the project page, the /projects
 * per-Area rows, and the /program project cards so the three surfaces
 * can never drift apart. */
export function weightedLine(s: WeightSummary): string {
  const pct = s.weightedPct === null ? "— (nothing sized)" : `${s.weightedPct}% weighted`;
  const unsized = s.unsizedCount ? ` · ${s.unsizedCount} unsized` : "";
  return `${pct} (${s.closedWeight}/${s.totalWeight} weight closed) · ${s.closedCount}/${s.count} items closed${unsized}`;
}

/** Hands-queue line: open weight waiting on hands vs online. */
export function handsLine(s: WeightSummary): string {
  const bits = [`hands queue: ${s.handsWeight} weight waiting on hands (physical + hybrid)`, `online: ${s.onlineWeight}`];
  if (s.unmodedOpenWeight) bits.push(`no mode: ${s.unmodedOpenWeight}`);
  if (s.unsizedOpenCount) bits.push(`${s.unsizedOpenCount} open unsized`);
  return bits.join(" · ");
}
