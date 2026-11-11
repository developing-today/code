// Flow analytics computation (#4829, step 2 of the #1281 split):
// time-in-status and weekly throughput per project/epic, computed
// from the #3981 item-history store and exposed machine-first as the
// JSON twin the step-3 charts (#4830) will render.
//
// The history seam is `item-history.ts` (#3981) — this module adds no
// store and no collector. It consumes the same weekly cutoffs the
// forecasting derivations consume and follows the same honesty rules:
//
// - Only complete, available cutoffs take part. An interval between
//   two consecutive stored cutoffs accrues time-in-status only when
//   BOTH bounding cutoffs are complete: an incomplete cutoff breaks
//   the chain exactly as it does in `weeklyTransitions`, the spanning
//   interval is attributed to no status and no week, and the cutoff
//   is named in the window's `gaps` — never silently zero-filled.
// - A complete week with no completions is a real observation: it
//   appears in every throughput series with `completed: 0`.
// - Throughput counts issue items only (the #3981 design SS1 unit):
//   a PR closing or merging is not a completed unit. Time-in-status
//   covers every recorded item, issues and PRs alike — their status
//   is a real board observation — with `kind` carried on each item
//   row so a consumer can filter.
// - Grouping is the forecast subject model (forecast.ts): an item's
//   group is its `parentRef` when present (direct membership),
//   otherwise its `epicRef` (ancestor membership), otherwise the
//   explicit unassigned group (`groupRef: null`). Membership is read
//   from the observation at the START of each interval, so an item
//   that changes parent mid-window accrues each interval to the
//   group it was observed in at that interval's start.
// - Time-in-status accrues an interval's whole span to the status
//   observed at the interval's first cutoff (the status the item was
//   seen in while that time passed). Transitions are observed
//   between cutoffs, never exact event times — the same lower-bound
//   honesty as the history module itself.
//
// Every result carries its computation window and the gap list, so
// no number is ever presented without its window and coverage state
// (#4829 acceptance).

import {
  weeklyTransitions,
  type ItemHistoryCutoff,
  type ItemHistoryItem,
} from "./item-history.ts";
import type { RefKey } from "./model.ts";

export const FLOW_ANALYTICS_SCHEMA_VERSION = 1;

/** Status label used for an item whose Project Status is unset. */
export const UNSET_STATUS = "(unset)";

export interface FlowWindowGap {
  cutoffAt: string;
  complete: boolean;
  unavailable: boolean;
}

export interface FlowWindow {
  /** First stored cutoff in the window, or null when history is empty. */
  fromCutoff: string | null;
  /** Last stored cutoff in the window, or null when history is empty. */
  toCutoff: string | null;
  /** All stored cutoffs in the window. */
  cutoffCount: number;
  /** Cutoffs usable as interval bounds (complete and available). */
  completeCutoffs: number;
  /** Incomplete or unavailable cutoffs, in order — the visible gap signal. */
  gaps: FlowWindowGap[];
}

export interface ItemStatusTime {
  ref: RefKey;
  kind: string;
  /** Days accrued per observed status over the window's complete intervals. */
  byStatus: { status: string; days: number }[];
  totalDays: number;
}

export interface FlowGroup {
  /** The project/epic subject ref, or null for the unassigned group. */
  groupRef: RefKey | null;
  /** Distinct items observed in this group at complete cutoffs. */
  itemCount: number;
  /** Aggregate item-days per status across the group's items. */
  timeInStatus: { status: string; itemDays: number }[];
  /** Per-item time-in-status, sorted by ref. */
  items: ItemStatusTime[];
  /** One entry per complete week in the window (the same weeks for
   * every group), in order; `completed` counts this group's issue
   * items observed completed that week. Zero weeks are preserved. */
  throughput: { cutoffAt: string; completed: number; completedRefs: RefKey[] }[];
}

export interface FlowAnalytics {
  schemaVersion: number;
  window: FlowWindow;
  groups: FlowGroup[];
}

function isUsable(cutoff: ItemHistoryCutoff): boolean {
  return cutoff.coverage.complete && !cutoff.coverage.unavailable;
}

function groupRefOf(item: ItemHistoryItem): RefKey | null {
  return item.parentRef ?? item.epicRef ?? null;
}

function statusOf(item: ItemHistoryItem): string {
  return item.status ?? UNSET_STATUS;
}

const DAY_MS = 86_400_000;

/**
 * Compute flow analytics over stored item-history cutoffs. Pure:
 * the caller supplies the cutoffs (the server reads them from the
 * ItemHistoryStore); no I/O, no clock.
 */
export function computeFlowAnalytics(cutoffs: ItemHistoryCutoff[]): FlowAnalytics {
  const ordered = [...cutoffs].sort((a, b) => a.cutoffAt.localeCompare(b.cutoffAt));

  const gaps: FlowWindowGap[] = ordered
    .filter((c) => !isUsable(c))
    .map((c) => ({
      cutoffAt: c.cutoffAt,
      complete: c.coverage.complete,
      unavailable: c.coverage.unavailable,
    }));

  // Per (group, ref): accrued days per status, plus the item kind.
  interface ItemAcc {
    kind: string;
    byStatus: Map<string, number>;
  }
  const perGroup = new Map<RefKey | null, Map<RefKey, ItemAcc>>();
  const groupItems = new Map<RefKey | null, Set<RefKey>>();

  const accFor = (group: RefKey | null, item: ItemHistoryItem): ItemAcc => {
    let items = perGroup.get(group);
    if (!items) {
      items = new Map();
      perGroup.set(group, items);
    }
    let acc = items.get(item.ref);
    if (!acc) {
      acc = { kind: item.kind ?? "issue", byStatus: new Map() };
      items.set(item.ref, acc);
    }
    return acc;
  };

  // Item membership per group, from complete cutoffs only.
  for (const cutoff of ordered) {
    if (!isUsable(cutoff)) continue;
    for (const item of cutoff.items) {
      const group = groupRefOf(item);
      let set = groupItems.get(group);
      if (!set) {
        set = new Set();
        groupItems.set(group, set);
      }
      set.add(item.ref);
      accFor(group, item); // ensure the item row exists even with no accrued interval yet
    }
  }

  // Time-in-status: an interval accrues only when both bounding
  // cutoffs are usable (the weeklyTransitions chain rule).
  for (let i = 1; i < ordered.length; i++) {
    const prev = ordered[i - 1]!;
    const cur = ordered[i]!;
    if (!isUsable(prev) || !isUsable(cur)) continue;
    const days = (Date.parse(cur.cutoffAt) - Date.parse(prev.cutoffAt)) / DAY_MS;
    if (!(days > 0)) continue;
    for (const item of prev.items) {
      const acc = accFor(groupRefOf(item), item);
      const status = statusOf(item);
      acc.byStatus.set(status, (acc.byStatus.get(status) ?? 0) + days);
    }
  }

  // Throughput per group, reusing weeklyTransitions so the week
  // definition (chain rule, issues only, zero weeks preserved) is
  // identical to the forecasting series.
  const weeks = weeklyTransitions(ordered);
  // Returns undefined when the item is not observed at the cutoff,
  // so a missing observation stays distinguishable from an explicit
  // unassigned membership (groupRefOf -> null) at that cutoff.
  const groupAt = (
    cutoff: ItemHistoryCutoff | undefined,
    ref: RefKey,
  ): { group: RefKey | null } | undefined => {
    const item = cutoff?.items.find((i) => i.ref === ref);
    return item ? { group: groupRefOf(item) } : undefined;
  };
  const byCutoff = new Map(ordered.map((c) => [c.cutoffAt, c]));
  const prevCutoffOf = (cutoffAt: string): ItemHistoryCutoff | undefined => {
    const idx = ordered.findIndex((c) => c.cutoffAt === cutoffAt);
    return idx > 0 ? ordered[idx - 1] : undefined;
  };
  const throughputByGroup = new Map<RefKey | null, Map<string, RefKey[]>>();
  for (const week of weeks) {
    const closing = byCutoff.get(week.cutoffAt);
    const previous = prevCutoffOf(week.cutoffAt);
    for (const ref of week.completedRefs) {
      const group = (groupAt(closing, ref) ?? groupAt(previous, ref))?.group ?? null;
      let perWeek = throughputByGroup.get(group);
      if (!perWeek) {
        perWeek = new Map();
        throughputByGroup.set(group, perWeek);
      }
      const refs = perWeek.get(week.cutoffAt) ?? [];
      refs.push(ref);
      perWeek.set(week.cutoffAt, refs);
    }
  }

  const groupRefs: (RefKey | null)[] = [...new Set([
    ...groupItems.keys(),
    ...perGroup.keys(),
    ...throughputByGroup.keys(),
  ])].sort((a, b) => (a ?? "").localeCompare(b ?? ""));

  const groups: FlowGroup[] = groupRefs.map((groupRef) => {
    const items: ItemStatusTime[] = [...(perGroup.get(groupRef)?.entries() ?? [])]
      .map(([ref, acc]) => {
        const byStatus = [...acc.byStatus.entries()]
          .map(([status, days]) => ({ status, days }))
          .sort((a, b) => a.status.localeCompare(b.status));
        return {
          ref,
          kind: acc.kind,
          byStatus,
          totalDays: byStatus.reduce((sum, s) => sum + s.days, 0),
        };
      })
      .sort((a, b) => a.ref.localeCompare(b.ref));
    const aggregate = new Map<string, number>();
    for (const item of items) {
      for (const s of item.byStatus) {
        aggregate.set(s.status, (aggregate.get(s.status) ?? 0) + s.days);
      }
    }
    const perWeek = throughputByGroup.get(groupRef);
    return {
      groupRef,
      itemCount: groupItems.get(groupRef)?.size ?? items.length,
      timeInStatus: [...aggregate.entries()]
        .map(([status, itemDays]) => ({ status, itemDays }))
        .sort((a, b) => a.status.localeCompare(b.status)),
      items,
      throughput: weeks.map((week) => {
        const completedRefs = [...(perWeek?.get(week.cutoffAt) ?? [])].sort();
        return { cutoffAt: week.cutoffAt, completed: completedRefs.length, completedRefs };
      }),
    };
  });

  return {
    schemaVersion: FLOW_ANALYTICS_SCHEMA_VERSION,
    window: {
      fromCutoff: ordered[0]?.cutoffAt ?? null,
      toCutoff: ordered[ordered.length - 1]?.cutoffAt ?? null,
      cutoffCount: ordered.length,
      completeCutoffs: ordered.filter(isUsable).length,
      gaps,
    },
    groups,
  };
}
