// Pure derivations from the read model: the execution strip, the roadmap
// timeline lanes, and the unified comment stream. No I/O here.
import type { Comment, GhItem, Link, RefKey, RoadmapRow } from "./model.ts";

export interface ExecutionStrip {
  /** Draft PRs, and open issues already dispatched or in progress. */
  implementing: GhItem[];
  /** Open, non-draft PRs. */
  review: GhItem[];
  /** Merged PRs. */
  merged: GhItem[];
  /** Open issues not yet being implemented. */
  queued: GhItem[];
  /** Closed issues and closed-unmerged PRs. */
  closed: GhItem[];
  /** Linked refs the source could not resolve. */
  unknown: RefKey[];
}

const IMPLEMENTING_LABELS = new Set(["status:in-progress", "in-progress", "ready-for-agent"]);

export function executionStrip(links: Link[], items: Map<RefKey, GhItem>): ExecutionStrip {
  const strip: ExecutionStrip = {
    implementing: [],
    review: [],
    merged: [],
    queued: [],
    closed: [],
    unknown: [],
  };
  const seen = new Set<RefKey>();
  for (const { ref } of links) {
    if (seen.has(ref)) continue;
    seen.add(ref);
    const item = items.get(ref);
    if (!item) {
      strip.unknown.push(ref);
      continue;
    }
    if (item.kind === "pr") {
      if (item.state === "merged") strip.merged.push(item);
      else if (item.state === "closed") strip.closed.push(item);
      else if (item.draft) strip.implementing.push(item);
      else strip.review.push(item);
    } else if (item.state !== "open") {
      strip.closed.push(item);
    } else if (item.labels.some((l) => IMPLEMENTING_LABELS.has(l.toLowerCase()))) {
      strip.implementing.push(item);
    } else {
      strip.queued.push(item);
    }
  }
  return strip;
}

/** A timeline column: a calendar quarter, or a non-dated horizon. */
export interface Lane {
  key: string;
  label: string;
  order: number;
}

const QUARTER = /^Q([1-4])\s+(\d{4})$/i;
const DATE = /^(\d{4})-(\d{2})-\d{2}/;

/**
 * Maps a Horizon cell to its timeline lane. Dates fold into their quarter;
 * "Ongoing", "Unscheduled" and anything else sort after every quarter.
 */
export function laneFor(horizon: string): Lane {
  const h = horizon.trim();
  const q = QUARTER.exec(h);
  if (q) return quarterLane(Number(q[2]), Number(q[1]));
  const d = DATE.exec(h);
  if (d) return quarterLane(Number(d[1]), Math.floor((Number(d[2]) - 1) / 3) + 1);
  if (/^ongoing$/i.test(h)) return { key: "ongoing", label: "Ongoing", order: 1e6 };
  if (h === "" || /^unscheduled$/i.test(h)) {
    return { key: "unscheduled", label: "Unscheduled", order: 1e6 + 2 };
  }
  return { key: `other:${h}`, label: h, order: 1e6 + 1 };
}

function quarterLane(year: number, quarter: number): Lane {
  return { key: `${year}-Q${quarter}`, label: `Q${quarter} ${year}`, order: year * 4 + quarter };
}

export interface Timeline {
  lanes: Lane[];
  themes: string[];
  /** cells[theme][laneKey] = rows */
  cells: Map<string, Map<string, RoadmapRow[]>>;
}

/** Themes (Area) × lanes (quarters, then non-dated horizons). */
export function timeline(rows: RoadmapRow[]): Timeline {
  const lanes = new Map<string, Lane>();
  const cells = new Map<string, Map<string, RoadmapRow[]>>();
  for (const row of rows) {
    const lane = laneFor(row.horizon);
    lanes.set(lane.key, lane);
    const theme = row.area || "Unthemed";
    if (!cells.has(theme)) cells.set(theme, new Map());
    const byLane = cells.get(theme) as Map<string, RoadmapRow[]>;
    byLane.set(lane.key, [...(byLane.get(lane.key) ?? []), row]);
  }
  return {
    lanes: [...lanes.values()].sort((a, b) => a.order - b.order),
    themes: [...cells.keys()],
    cells,
  };
}

/** One activity stream for an item: every linked ref's comments, newest first. */
export function commentStream(links: Link[], comments: Comment[]): Comment[] {
  const wanted = new Set(links.map((l) => l.ref));
  return comments
    .filter((c) => wanted.has(c.ref))
    .sort((a, b) => b.createdAt.localeCompare(a.createdAt) || a.id.localeCompare(b.id));
}

/** Every distinct ref across a set of items. */
export function allRefs(itemsWithLinks: { links: Link[] }[]): RefKey[] {
  return [...new Set(itemsWithLinks.flatMap((i) => i.links.map((l) => l.ref)))];
}

// ---------------------------------------------------------------------------
// Quota view derivations (#851).
// ---------------------------------------------------------------------------

import type { QuotaPool, QuotaWindow } from "./model.ts";

/** Percent used for a window (100 when the limit is unknown). */
export function windowPercent(w: QuotaWindow): number {
  if (w.limit == null || w.limit <= 0) return 100;
  return (w.used / w.limit) * 100;
}

/** Near-limit threshold, in percent. */
export const NEAR_PCT = 85;

/** A pool is walled when any window is at or over its limit. */
export function poolState(p: QuotaPool): "healthy" | "near" | "walled" {
  for (const w of p.windows) if (windowPercent(w) >= 100) return "walled";
  for (const w of p.windows) if (windowPercent(w) >= NEAR_PCT) return "near";
  return "healthy";
}

// ---------------------------------------------------------------------------
// Attention view derivations (#852). The classification rules combine
// server signals (status, message growth, active tool calls) with the
// watch-cron heuristics already proven in the id-train watches.
// ---------------------------------------------------------------------------

import type { AttentionEntry, AttentionState, SessionFeedEntry } from "./model.ts";

const ORDER: Record<AttentionState, number> = {
  "stopped-for-question": 0,
  errored: 1,
  "needs-poke": 2,
  "stuck-retrying": 3,
  "stalled-on-cli": 4,
  "in-motion": 5,
};

/** A shell call older than this is "stalled on CLI". */
export const CLI_STALL_MS = 30 * 60_000;

/** Retry-loop detection: this many consecutive identical errors. */
export const RETRY_LOOP_THRESHOLD = 3;

/**
 * Classify one session feed entry. Ordering matters: the first rule that
 * matches wins, so a walled-spin with a question still surfaces the question.
 */
export function classifyAttention(e: SessionFeedEntry, now = Date.now()): AttentionEntry {
  let state: AttentionState = "in-motion";
  let evidence = "Messages/tool calls progressing.";

  if (e.awaitingQuestion) {
    state = "stopped-for-question";
    evidence = `Waiting on: ${e.awaitingQuestion}`;
  } else if (e.lastTurnError) {
    state = "errored";
    evidence = `Last turn error: ${e.lastTurnError}`;
  } else if ((e.repeatedErrorCount ?? 0) >= RETRY_LOOP_THRESHOLD) {
    state = "stuck-retrying";
    evidence = `${e.repeatedErrorCount} consecutive identical errors, no progress.`;
  } else if (e.longRunningCall && Date.parse(e.longRunningCall.startedAt) + CLI_STALL_MS < now) {
    state = "stalled-on-cli";
    evidence = `CLI running long: ${e.longRunningCall.command ?? "unknown"} (since ${e.longRunningCall.startedAt}).`;
  } else if (!e.lastActivityAt || e.activeToolCalls === 0) {
    state = "needs-poke";
    evidence = "Idle with no completion signal and no question.";
  }

  return { ...e, state, order: ORDER[state], evidence };
}

/** Sort a feed by attention (most urgent first). */
export function attentionSorted(feed: SessionFeedEntry[], now = Date.now()): AttentionEntry[] {
  return feed.map((e) => classifyAttention(e, now)).sort((a, b) => a.order - b.order);
}
