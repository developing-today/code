// Versioned item-history capture for forecasting (#3981, slice 1).
//
// The forecasting design (doc/2026-11-01T21-07-00Z_design_pm-stack_
// forecasting.md §1, §3, §5) names one prerequisite before any ETA or
// stall-risk number may be claimed: a single versioned item-history
// source that records, at each complete weekly cutoff, the observed
// item scope and state, closure/reopen transitions, project/epic
// membership, effort, and blocker edges — with source freshness and
// coverage attached, so an incomplete week is excluded rather than
// silently read as zero throughput. This module is that source: the
// record shape, the capture from the live project projection, an
// append-only JSONL store, and the pure derivations forecasting and
// #1281 flow analytics share. There is deliberately no second
// collector or store: the writer is whatever job already assembles
// the project pool (the seam is `captureItemHistory`), and readers
// consume `ItemHistoryStore`.
//
// Honesty rules baked into the shape (from the issue's acceptance):
// - A cutoff whose source was unavailable is recorded with
//   `coverage.unavailable: true` and `complete: false`. Unavailable is
//   a distinct state from zero: derivations skip incomplete cutoffs
//   entirely instead of counting them as zero-completion weeks, while
//   a complete cutoff with no transitions IS a preserved zero week.
// - Transitions are *observed* transitions between consecutive
//   complete cutoffs, stamped with the observing cutoff — never an
//   exact historical event time backfilled from current state.
// - A blocker edge's age is a lower bound from the first cutoff of
//   its current uninterrupted episode (`firstObservedAt`): a complete
//   cutoff that observes the item without the edge ends the episode,
//   so a re-added edge starts a new one. The bound is labelled as
//   such everywhere it is returned. GitHub does not publish edge event times to this
//   aggregator, so nothing here pretends otherwise.
//
// Ownership/retention: the store file lives beside the other durable
// aggregator stores (default `item-history.jsonl` in the aggregator
// data directory; `ITEM_HISTORY_FILE` overrides). It is append-only;
// compaction/retention follows the usage-snapshot precedent
// (snapshots.ts): keep raw cutoffs for at least 13 months so 8-week
// windows and backtests (§5 of the design) always have their inputs.

import { appendFile, mkdir, readFile } from "node:fs/promises";
import { dirname, join } from "node:path";
import type { GhKind, GhState, ProjectPoolItem, RefKey } from "./model.ts";
import { isIsoDate } from "./validate.ts";

export const ITEM_HISTORY_SCHEMA_VERSION = 1;

/** One item as observed at a cutoff. */
export interface ItemHistoryItem {
  ref: RefKey;
  repo: string;
  number: number;
  /** Content kind from the project pool. The forecasting unit is
   * the native child issue (design SS1): PRs are recorded — their
   * state is a real observation — but never counted as completed
   * units in the derivations. Absent on lines written before kind
   * was recorded; such items are treated as issues. */
  kind?: GhKind;
  /** GitHub state at observation time ("merged" for merged PRs). */
  state: GhState;
  /** Project Status single-select value; null when unset. Distinct
   * from GitHub state — the two disagree sometimes, and both are
   * recorded so the disagreement stays visible downstream. */
  status: string | null;
  /** The `effort/*` label suffix (xs/s/m/l/xl), or null when unsized. */
  effort: string | null;
  parentRef?: RefKey;
  epicRef?: RefKey;
  /** Refs this item was observed blocked by at the cutoff, sorted. */
  blockedBy: RefKey[];
}

/** Source freshness/coverage for one cutoff. */
export interface ItemHistoryCoverage {
  /** Collector/source name (e.g. "projects-pool"). */
  source: string;
  /** When the source data was fetched. */
  fetchedAt: string;
  /** True only when the source was available for the full window
   * this cutoff closes. Incomplete cutoffs are stored (so the gap is
   * visible) but excluded from every derivation. */
  complete: boolean;
  /** True when the source could not be read at all. Distinct from a
   * complete observation of an empty scope. */
  unavailable: boolean;
}

/** One weekly cutoff: the full observed scope at `cutoffAt`. */
export interface ItemHistoryCutoff {
  schemaVersion: number;
  cutoffAt: string;
  coverage: ItemHistoryCoverage;
  items: ItemHistoryItem[];
}

/** The effort label suffix on an item, or null when unsized. */
export function effortOf(labels: string[] | undefined): string | null {
  for (const label of labels ?? []) {
    if (label.startsWith("effort/")) return label.slice("effort/".length);
  }
  return null;
}

/**
 * Capture one cutoff from the live project projection. Pure: the
 * caller supplies the pool items it already fetched plus the
 * coverage facts about that fetch, and decides when a cutoff is due
 * (the coverage weekly anchor, coverage.ts, is the natural clock).
 */
export function captureItemHistory(
  pool: ProjectPoolItem[],
  coverage: ItemHistoryCoverage,
  cutoffAt: string,
): ItemHistoryCutoff {
  const items = pool
    .map((item) => {
      const captured: ItemHistoryItem = {
        ref: item.ref,
        repo: item.repo,
        number: item.number,
        kind: item.contentKind,
        state: item.state,
        status: item.status,
        effort: effortOf(item.labels),
        blockedBy: [...(item.blockedBy ?? [])].sort(),
      };
      if (item.parentRef) captured.parentRef = item.parentRef;
      if (item.epicRef) captured.epicRef = item.epicRef;
      return captured;
    })
    .sort((a, b) => a.ref.localeCompare(b.ref));
  return {
    schemaVersion: ITEM_HISTORY_SCHEMA_VERSION,
    cutoffAt,
    coverage,
    items,
  };
}

/** Structural gate for one stored cutoff line. */
export function isItemHistoryCutoff(v: unknown): v is ItemHistoryCutoff {
  if (!v || typeof v !== "object") return false;
  const c = v as Record<string, unknown>;
  if (c.schemaVersion !== ITEM_HISTORY_SCHEMA_VERSION) return false;
  // Real calendar dates only (#5172): Date.parse rolls impossible dates
  // over (2026-02-30 -> Mar 2) and parses non-ISO strings in a
  // timezone-dependent way, so gate with isIsoDate like the sibling
  // gates (snapshots.ts, #2264/#2483).
  if (!isIsoDate(c.cutoffAt)) return false;
  const coverage = c.coverage as Record<string, unknown> | undefined;
  if (!coverage || typeof coverage !== "object") return false;
  if (typeof coverage.source !== "string" || !isIsoDate(coverage.fetchedAt)) return false;
  if (typeof coverage.complete !== "boolean" || typeof coverage.unavailable !== "boolean") {
    return false;
  }
  if (!Array.isArray(c.items)) return false;
  return c.items.every((raw) => {
    if (!raw || typeof raw !== "object") return false;
    const i = raw as Record<string, unknown>;
    return (
      typeof i.ref === "string" &&
      typeof i.repo === "string" &&
      Number.isInteger(i.number) &&
      (i.kind === undefined || i.kind === "issue" || i.kind === "pr") &&
      (i.state === "open" || i.state === "closed" || i.state === "merged") &&
      (typeof i.status === "string" || i.status === null) &&
      (typeof i.effort === "string" || i.effort === null) &&
      Array.isArray(i.blockedBy) &&
      (i.blockedBy as unknown[]).every((b) => typeof b === "string")
    );
  });
}

const DEFAULT_FILE = "item-history.jsonl";

/** Where the history store lives: `ITEM_HISTORY_FILE`, else
 * `item-history.jsonl` inside `dataDir` (the aggregator's durable
 * directory), else the bare file name. */
export function itemHistoryFile(dataDir?: string): string {
  const override = process.env.ITEM_HISTORY_FILE;
  if (override) return override;
  return dataDir ? join(dataDir, DEFAULT_FILE) : DEFAULT_FILE;
}

/** Append-only JSONL store, one cutoff per line. */
export class ItemHistoryStore {
  constructor(private readonly file: string) {}

  /** Append one cutoff. A cutoff for an already-recorded `cutoffAt`
   * is not written twice — re-running a collector for the same week
   * must not double-count the week. Returns false when skipped. */
  async append(cutoff: ItemHistoryCutoff): Promise<boolean> {
    const existing = await this.readAll();
    if (existing.some((c) => c.cutoffAt === cutoff.cutoffAt)) return false;
    await mkdir(dirname(this.file), { recursive: true });
    await appendFile(this.file, JSON.stringify(cutoff) + "\n", "utf8");
    return true;
  }

  /** All stored cutoffs, oldest first. Malformed lines are skipped —
   * one bad line must not blind every reader to the whole history —
   * but a missing file is simply an empty history. */
  async readAll(): Promise<ItemHistoryCutoff[]> {
    let text: string;
    try {
      text = await readFile(this.file, "utf8");
    } catch {
      return [];
    }
    const cutoffs: ItemHistoryCutoff[] = [];
    for (const line of text.split("\n")) {
      if (!line.trim()) continue;
      try {
        const parsed: unknown = JSON.parse(line);
        if (isItemHistoryCutoff(parsed)) cutoffs.push(parsed);
      } catch {
        // Skip the malformed line; see the doc comment above.
      }
    }
    return cutoffs.sort((a, b) => a.cutoffAt.localeCompare(b.cutoffAt));
  }
}

/** One week's observed transitions between two complete cutoffs. */
export interface WeeklyTransitions {
  /** The cutoff closing the week. */
  cutoffAt: string;
  /** Items observed open at the previous bounding cutoff and
   * closed/merged at this one. */
  completedRefs: RefKey[];
  /** Items observed closed/merged at the previous bounding cutoff
   * and open at this one. A reopened item leaves the completed
   * count until it closes again (the design's throughput unit). */
  reopenedRefs: RefKey[];
}

/**
 * Observed transitions per complete week. Only complete cutoffs take
 * part: an incomplete cutoff neither contributes a week nor breaks
 * the chain — the next complete cutoff is compared against the last
 * complete one, and the gap stays visible in the stored cutoffs.
 * Complete weeks with no transitions appear with empty lists: a
 * zero-completion week is a real observation, preserved as such.
 */
/**
 * Observed transitions per complete week. A week exists only where
 * two *consecutive stored* cutoffs are both complete: an incomplete
 * cutoff breaks the chain, and transitions that span it fall in an
 * unknown interval attributed to no week — the next complete cutoff
 * becomes a fresh baseline instead of silently absorbing the gap
 * weeks' completions into its own bucket. Complete weeks with no
 * transitions appear with empty lists: a zero-completion week is a
 * real observation, preserved as such. Only issue items count as
 * units (design SS1): a PR closing or merging is not a completed
 * unit, and an issue plus its PR never double-count.
 */
export function weeklyTransitions(cutoffs: ItemHistoryCutoff[]): WeeklyTransitions[] {
  const ordered = [...cutoffs].sort((a, b) => a.cutoffAt.localeCompare(b.cutoffAt));
  const weeks: WeeklyTransitions[] = [];
  for (let i = 1; i < ordered.length; i++) {
    const prev = ordered[i - 1]!;
    const cur = ordered[i]!;
    if (!prev.coverage.complete || prev.coverage.unavailable) continue;
    if (!cur.coverage.complete || cur.coverage.unavailable) continue;
    const before = new Map(prev.items.map((item) => [item.ref, item.state]));
    const completedRefs: RefKey[] = [];
    const reopenedRefs: RefKey[] = [];
    for (const item of cur.items) {
      if (item.kind === "pr") continue;
      const was = before.get(item.ref);
      if (was === "open" && item.state !== "open") completedRefs.push(item.ref);
      if (was !== undefined && was !== "open" && item.state === "open") {
        reopenedRefs.push(item.ref);
      }
    }
    weeks.push({
      cutoffAt: cur.cutoffAt,
      completedRefs: completedRefs.sort(),
      reopenedRefs: reopenedRefs.sort(),
    });
  }
  return weeks;
}

/** Completed units per complete week, in order — the throughput
 * series the forecasting estimator consumes (design §1). */
export function weeklyCompletions(cutoffs: ItemHistoryCutoff[]): { cutoffAt: string; completed: number }[] {
  return weeklyTransitions(cutoffs).map((week) => ({
    cutoffAt: week.cutoffAt,
    completed: week.completedRefs.length,
  }));
}

/**
 * The first observation of the CURRENT episode of one blocker edge:
 * the earliest cutoff of the latest uninterrupted run that recorded
 * `blockerRef` in `ref`'s blockedBy. A complete cutoff that observes
 * the item without the edge ends the episode, so an edge removed and
 * later re-added starts a new episode — the age of the new episode
 * is never backdated across the removal. (An incomplete cutoff
 * establishes neither presence-to-absence nor a reset: its scope is
 * partial.) The returned age is a LOWER BOUND on the current
 * episode's true age — the edge may predate the episode's first
 * observation — and is labelled as such; no exact event time is
 * fabricated. Null when the edge has no current episode (never
 * observed, or observed removed at a later complete cutoff).
 */
export function blockerFirstObserved(
  cutoffs: ItemHistoryCutoff[],
  ref: RefKey,
  blockerRef: RefKey,
): { firstObservedAt: string; lowerBoundAgeDays: number; lowerBound: true } | null {
  const ordered = [...cutoffs].sort((a, b) => a.cutoffAt.localeCompare(b.cutoffAt));
  let episodeStart: string | null = null;
  for (const cutoff of ordered) {
    const item = cutoff.items.find((i) => i.ref === ref);
    if (!item) continue;
    if (item.blockedBy.includes(blockerRef)) {
      if (episodeStart === null) episodeStart = cutoff.cutoffAt;
    } else if (cutoff.coverage.complete && !cutoff.coverage.unavailable) {
      episodeStart = null;
    }
  }
  if (episodeStart === null) return null;
  const last = ordered[ordered.length - 1]!;
  const ageMs = Date.parse(last.cutoffAt) - Date.parse(episodeStart);
  return {
    firstObservedAt: episodeStart,
    lowerBoundAgeDays: Math.max(0, Math.floor(ageMs / 86_400_000)),
    lowerBound: true,
  };
}
