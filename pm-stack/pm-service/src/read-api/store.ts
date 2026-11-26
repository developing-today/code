/**
 * The read API's tenant-DB surface: everything the four hosted read
 * endpoints serve, read out of slice 1's schema as slices 1–2 wrote
 * it. Read-only — this module issues SELECTs only; the collector and
 * reconcile own every write path.
 *
 * Sources, per endpoint:
 * - projects: pm_boards (the registry, via the collector's loader) +
 *   each board's latest pm_snapshots header + audit_json.
 * - item: the merged pool — pm_items holds current state per board
 *   (the collector replaces a board's rows when its snapshot lands),
 *   and item_json is the full ProjectPoolItem the derivation runs on.
 * - roadmap: pm_roadmap_rows, whose genealogy overlay columns
 *   (priority_ref / after_refs_json / release_ref) win per-field
 *   where set; unset fields are derived over the pool (genealogy.ts).
 * - releases: pm_release_snapshots joined to pm_milestones where a
 *   close-out snapshot is stored (the stored record is what §4 says
 *   the API serves); milestones with no stored snapshot get the
 *   retroactive computed accounting over the pool (releases.ts).
 *
 * The SQL and the row→shape mappings live here exactly once: the
 * synchronous loaders below and the async twins in async-store.ts
 * (CF-5c, #14969) run the same queries through the same mappings, so
 * the served shapes cannot drift between the two store layers.
 */
import type { MilestoneRef, ProjectAudit, ProjectPoolItem } from "../collector/pool.ts";
import { loadRegistry } from "../collector/store.ts";
import type { SqlDb } from "../tenant-store/migrate.ts";
import type { BoardDef } from "../tenant-store/schema.ts";
import { enrichRoadmapRows, type RoadmapRow } from "./genealogy.ts";
import {
  computeMilestoneSnapshot,
  releaseSnapshotFromStored,
  type CategoryCounts,
  type MilestoneKind,
  type ReleaseSnapshot,
} from "./releases.ts";

/** One board's entry in /api/projects.json's `projects[]` (#9060 §6):
 * the registry def plus the board's latest snapshot audit. A board
 * with no landed snapshot serves `snapshot: null, audit: null` —
 * the absence is the honest state, never a fabricated audit. */
export interface BoardSummary {
  board: BoardDef;
  snapshot: {
    snapshotId: string;
    generatedAt: string;
    reportedTotal: number;
    emittedEntries: number;
    writer: string;
    epoch: number;
  } | null;
  audit: ProjectAudit | null;
}

export interface SnapshotRow {
  snapshot_id: string;
  generated_at: string;
  reported_total: number;
  emitted_entries: number;
  writer: string;
  epoch: number;
  audit_json: string;
}

export const LATEST_SNAPSHOT_SQL = `SELECT snapshot_id, generated_at, reported_total, emitted_entries, writer, epoch, audit_json
       FROM pm_snapshots WHERE board_id = ?
       ORDER BY generated_at DESC, recorded_at DESC LIMIT 1`;

export function boardSummaryFromRow(board: BoardDef, row: SnapshotRow | undefined): BoardSummary {
  if (!row) return { board, snapshot: null, audit: null };
  return {
    board,
    snapshot: {
      snapshotId: row.snapshot_id,
      generatedAt: row.generated_at,
      reportedTotal: row.reported_total,
      emittedEntries: row.emitted_entries,
      writer: row.writer,
      epoch: row.epoch,
    },
    audit: JSON.parse(row.audit_json) as ProjectAudit,
  };
}

export function loadBoardSummaries(db: SqlDb): BoardSummary[] {
  return loadRegistry(db).map((board) =>
    boardSummaryFromRow(board, db.all<SnapshotRow>(LATEST_SNAPSHOT_SQL, [board.id])[0]),
  );
}

export const POOL_SQL = "SELECT item_json FROM pm_items ORDER BY board_id, ref";

/** The merged pool from its rows: every board's current pm_items
 * rows, in the collector's ProjectPoolItem shape (item_json is
 * byte-faithful). A row that does not parse is a store bug and
 * fails loudly, the same stance the collector's registry loader
 * takes — a silently dropped item would fabricate the pool's
 * contents. */
export function poolFromRows(rows: readonly { item_json: string }[]): ProjectPoolItem[] {
  return rows.map((row) => JSON.parse(row.item_json) as ProjectPoolItem);
}

/** The merged pool: every board's current pm_items rows, in the
 * collector's ProjectPoolItem shape (item_json is byte-faithful). */
export function loadPool(db: SqlDb): ProjectPoolItem[] {
  return poolFromRows(db.all<{ item_json: string }>(POOL_SQL));
}

export interface RoadmapRowRow {
  row_id: number;
  subject: string;
  title: string;
  area: string;
  status: string;
  horizon: string;
  horizon_error: string | null;
  source: string;
  notes: string;
  links_json: string;
  priority_ref: string | null;
  after_refs_json: string;
  release_ref: string | null;
  grist_url: string | null;
  file_url: string | null;
}

export const ROADMAP_SQL = `SELECT row_id, subject, title, area, status, horizon, horizon_error, source, notes,
            links_json, priority_ref, after_refs_json, release_ref, grist_url, file_url
     FROM pm_roadmap_rows ORDER BY row_id`;

/** Roadmap rows with the #9060 §6 overlay served: a stored overlay
 * field wins where set; an unset field is derived over the pool via
 * the genealogy enrichment, so a row whose tracks link resolves
 * serves its priority/afterRefs/releaseRef even before any overlay
 * is recorded for it. */
export function roadmapRowsFromRows(rows: readonly RoadmapRowRow[], pool: readonly ProjectPoolItem[]): RoadmapRow[] {
  const base: RoadmapRow[] = rows.map((row) => ({
    id: row.row_id,
    subject: row.subject,
    title: row.title,
    area: row.area,
    status: row.status,
    horizon: row.horizon,
    ...(row.horizon_error !== null ? { horizonError: row.horizon_error } : {}),
    source: row.source,
    notes: row.notes,
    links: JSON.parse(row.links_json) as RoadmapRow["links"],
    ...(row.grist_url !== null ? { gristUrl: row.grist_url } : {}),
    ...(row.file_url !== null ? { fileUrl: row.file_url } : {}),
  }));
  const derived = enrichRoadmapRows(base, pool);
  return derived.map((row, index) => {
    const stored = rows[index] as RoadmapRowRow;
    const storedAfterRefs = JSON.parse(stored.after_refs_json) as string[];
    return {
      ...row,
      priority: stored.priority_ref ?? row.priority ?? null,
      afterRefs: storedAfterRefs.length ? storedAfterRefs : (row.afterRefs ?? []),
      releaseRef: stored.release_ref ?? row.releaseRef ?? null,
    };
  });
}

export function loadRoadmapRows(db: SqlDb, pool: readonly ProjectPoolItem[]): RoadmapRow[] {
  return roadmapRowsFromRows(db.all<RoadmapRowRow>(ROADMAP_SQL), pool);
}

export interface StoredSnapshotRow {
  repo: string;
  title: string;
  kind: MilestoneKind;
  due_on: string | null;
  closed_at: string | null;
  release_date: string;
  issues_covered: number;
  issues_closed: number;
  issues_carried_over: number;
  designs_covered: number;
  designs_closed: number;
  designs_carried_over: number;
  epics_covered: number;
  epics_closed: number;
  epics_carried_over: number;
  carried_over_epics_json: string;
}

export const RELEASE_SNAPSHOTS_SQL = `SELECT s.repo, m.title, m.kind, m.due_on, m.closed_at, s.release_date,
            s.issues_covered, s.issues_closed, s.issues_carried_over,
            s.designs_covered, s.designs_closed, s.designs_carried_over,
            s.epics_covered, s.epics_closed, s.epics_carried_over,
            s.carried_over_epics_json
     FROM pm_release_snapshots s
     JOIN pm_milestones m ON m.repo = s.repo AND m.number = s.milestone_number`;

const storedCounts = (covered: number, closed: number, carriedOver: number): CategoryCounts => ({
  closed,
  carriedOver,
  total: covered,
});

/** Milestone close-out snapshots for /api/releases.json, from the
 * stored-snapshot rows: stored snapshots (pm_release_snapshots) are
 * the record where they exist; every other milestone the pool
 * carries gets the retroactive computed accounting (§4), keyed by
 * repo + title so two repos' same-titled milestones never merge.
 * Sorted by title, then repo — a stable served shape. */
export function releaseSnapshotsFromRows(
  storedRows: readonly StoredSnapshotRow[],
  pool: readonly ProjectPoolItem[],
): ReleaseSnapshot[] {
  const storedKeys = new Set(storedRows.map((row) => `${row.repo}\n${row.title}`));
  const keyed: { key: string; snapshot: ReleaseSnapshot }[] = storedRows.map((row) => ({
    key: `${row.repo}\n${row.title}`,
    snapshot: releaseSnapshotFromStored({
      milestone: { title: row.title, dueOn: row.due_on, closedAt: row.closed_at },
      kind: row.kind,
      releaseDate: row.release_date,
      counts: {
        issues: storedCounts(row.issues_covered, row.issues_closed, row.issues_carried_over),
        designs: storedCounts(row.designs_covered, row.designs_closed, row.designs_carried_over),
        epics: storedCounts(row.epics_covered, row.epics_closed, row.epics_carried_over),
      },
      carriedOverEpics: JSON.parse(row.carried_over_epics_json) as string[],
    }),
  }));

  const groups = new Map<string, { milestone: MilestoneRef; items: ProjectPoolItem[] }>();
  for (const item of pool) {
    if (!item.milestone) continue;
    const key = `${item.repo}\n${item.milestone.title}`;
    if (storedKeys.has(key)) continue;
    const entry = groups.get(key) ?? { milestone: item.milestone, items: [] };
    // Prefer the freshest milestone stamps the collector saw.
    entry.milestone = item.milestone;
    entry.items.push(item);
    groups.set(key, entry);
  }
  for (const [key, { milestone, items }] of groups) {
    keyed.push({ key, snapshot: computeMilestoneSnapshot(milestone, items) });
  }

  return keyed
    .sort((a, b) => a.snapshot.milestone.title.localeCompare(b.snapshot.milestone.title) || a.key.localeCompare(b.key))
    .map(({ snapshot }) => snapshot);
}

export function loadReleaseSnapshots(db: SqlDb, pool: readonly ProjectPoolItem[]): ReleaseSnapshot[] {
  return releaseSnapshotsFromRows(db.all<StoredSnapshotRow>(RELEASE_SNAPSHOTS_SQL), pool);
}
