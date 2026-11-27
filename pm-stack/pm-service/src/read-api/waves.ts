/**
 * CF-5d (#14970) first read wave: the overview and queue views over
 * the tenant-DB projections the read API already serves.
 *
 * Both views are pure derivations over the merged pool and the
 * board summaries (registry + latest snapshot per board) — no new
 * projection, no new write path, no new SQL. The #14865
 * write-amplification check for this wave is therefore zero: the
 * collector's existing snapshot landing is the only writer, and
 * these routes issue the same SELECTs the four existing routes
 * already issue. Surfaces whose data is not in these projections
 * (quota, events, digest, reports, /aa, /chat, today) stay on their
 * current stacks until their own wave lands their projection; this
 * module does not fabricate them.
 *
 * Data-plane freshness is part of the served shape, per #14970's
 * acceptance: every wave response carries the per-board latest
 * snapshot stamps and the newest one as `freshness.latestSnapshotAt`
 * (null when no board has landed a snapshot — the honest state).
 */
import type { ProjectPoolItem } from "../collector/pool.ts";
import type { BoardSummary } from "./store.ts";

export interface SnapshotFreshness {
  boardId: string;
  snapshotId: string;
  generatedAt: string;
  writer: string;
  epoch: number;
}

export interface Freshness {
  /** Newest board snapshot stamp across the registry, or null. */
  latestSnapshotAt: string | null;
  /** One entry per board that has landed a snapshot, registry order. */
  snapshots: SnapshotFreshness[];
}

export function freshnessFromSummaries(summaries: readonly BoardSummary[]): Freshness {
  const snapshots: SnapshotFreshness[] = [];
  for (const summary of summaries) {
    if (!summary.snapshot) continue;
    snapshots.push({
      boardId: summary.board.id,
      snapshotId: summary.snapshot.snapshotId,
      generatedAt: summary.snapshot.generatedAt,
      writer: summary.snapshot.writer,
      epoch: summary.snapshot.epoch,
    });
  }
  const latestSnapshotAt = snapshots.reduce<string | null>(
    (latest, snap) => (latest === null || snap.generatedAt > latest ? snap.generatedAt : latest),
    null,
  );
  return { latestSnapshotAt, snapshots };
}

export interface ProgramGroup {
  /** The canonical Project ref for this Epic, whether or not it is in this pool. */
  ref: string;
  /** All pool rows for this Epic; empty when the header is outside the pool. */
  headers: ProjectPoolItem[];
  /** Non-Epic pool items whose nearest Epic ancestor is ref, in pool order. */
  items: ProjectPoolItem[];
}

export interface ProgramView {
  /** Epic groups sorted by ref; members preserve the source pool order. */
  epics: ProgramGroup[];
  /** Non-Epic items with no Epic ancestor, in source pool order. */
  unassigned: ProjectPoolItem[];
}

/** Build the program view from the collector's existing Epic ancestry.
 * Missing Epic headers remain explicit as empty arrays; the view never infers
 * membership from titles or invents a source item. */
export function programFromPool(pool: readonly ProjectPoolItem[]): ProgramView {
  const groups = new Map<string, ProgramGroup>();
  const unassigned: ProjectPoolItem[] = [];
  const groupFor = (ref: string): ProgramGroup => {
    let group = groups.get(ref);
    if (!group) {
      group = { ref, headers: [], items: [] };
      groups.set(ref, group);
    }
    return group;
  };

  for (const item of pool) {
    if (item.kind?.toLowerCase() === "epic") {
      groupFor(item.ref).headers.push(item);
    } else if (item.epicRef) {
      groupFor(item.epicRef).items.push(item);
    } else {
      unassigned.push(item);
    }
  }

  return {
    epics: [...groups.values()].sort((a, b) => (a.ref < b.ref ? -1 : a.ref > b.ref ? 1 : 0)),
    unassigned,
  };
}

const countBy = (pool: readonly ProjectPoolItem[], key: (item: ProjectPoolItem) => string | null) => {
  const counts = new Map<string, number>();
  for (const item of pool) {
    const value = key(item) ?? "(unset)";
    counts.set(value, (counts.get(value) ?? 0) + 1);
  }
  return Object.fromEntries([...counts.entries()].sort(([a], [b]) => a.localeCompare(b)));
};

export interface Overview {
  totals: {
    boards: number;
    boardsWithSnapshots: number;
    items: number;
    open: number;
    closed: number;
    merged: number;
    drafts: number;
  };
  counts: {
    state: Record<string, number>;
    status: Record<string, number>;
    area: Record<string, number>;
    horizon: Record<string, number>;
    kind: Record<string, number>;
    contentKind: Record<string, number>;
  };
}

/** The overview view: pool-wide totals and breakdowns over the
 * projections, plus the board summaries it was derived from. */
export function overviewFromPool(pool: readonly ProjectPoolItem[], summaries: readonly BoardSummary[]): Overview {
  return {
    totals: {
      boards: summaries.length,
      boardsWithSnapshots: summaries.filter((summary) => summary.snapshot !== null).length,
      items: pool.length,
      open: pool.filter((item) => item.state === "open").length,
      closed: pool.filter((item) => item.state === "closed").length,
      merged: pool.filter((item) => item.state === "merged").length,
      drafts: pool.filter((item) => item.draft === true).length,
    },
    counts: {
      state: countBy(pool, (item) => item.state),
      status: countBy(pool, (item) => item.status),
      area: countBy(pool, (item) => item.area),
      horizon: countBy(pool, (item) => item.horizon),
      kind: countBy(pool, (item) => item.kind),
      contentKind: countBy(pool, (item) => item.contentKind),
    },
  };
}

export interface QueueView {
  totalOpen: number;
  /** Open items grouped by project Status, pool order within a group. */
  byStatus: { status: string; refs: string[] }[];
  /** Open items blocked by at least one still-open pool item. A
   * blocker that is closed/merged or absent from the pool does not
   * hold the item — it is not part of the live queue's wait. */
  blocked: { ref: string; blockedBy: string[] }[];
  /** Refs of open items with no live blocker, in pool order: the
   * actionable head of the queue. */
  readyRefs: string[];
  /** Every open item, in pool order (the store's board/ref order). */
  items: ProjectPoolItem[];
}

/** The queue view (the console Now/queue family's hosted half):
 * the open pool, grouped and split by live blockers. */
export function queueFromPool(pool: readonly ProjectPoolItem[]): QueueView {
  const open = pool.filter((item) => item.state === "open");
  const openRefs = new Set(open.map((item) => item.ref));
  const blocked: QueueView["blocked"] = [];
  const readyRefs: string[] = [];
  for (const item of open) {
    const liveBlockers = (item.blockedBy ?? []).filter((ref) => openRefs.has(ref));
    if (liveBlockers.length) blocked.push({ ref: item.ref, blockedBy: liveBlockers });
    else readyRefs.push(item.ref);
  }
  const groups = new Map<string, string[]>();
  for (const item of open) {
    const status = item.status ?? "(unset)";
    const refs = groups.get(status) ?? [];
    refs.push(item.ref);
    groups.set(status, refs);
  }
  return {
    totalOpen: open.length,
    byStatus: [...groups.entries()].sort(([a], [b]) => a.localeCompare(b)).map(([status, refs]) => ({ status, refs })),
    blocked,
    readyRefs,
    items: open,
  };
}
