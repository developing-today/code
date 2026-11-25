/**
 * The async twin of the read API's store layer (CF-5c, #14969):
 * the same four loads as store.ts, over an AsyncSqlDb (the
 * libSQL/Turso HTTP adapter) instead of the synchronous SqlDb.
 *
 * Every query string and every row→shape mapping is imported from
 * store.ts / collector/store.ts — this module contributes only the
 * awaiting. The served shapes are therefore identical by
 * construction, and the parity suite (test/read-api-async.test.ts)
 * pins that claim against the synchronous handler route by route.
 */
import { REGISTRY_SQL, registryFromRows, type BoardRow } from "../collector/store.ts";
import type { ProjectPoolItem } from "../collector/pool.ts";
import type { AsyncSqlDb } from "../tenant-store/libsql-http.ts";
import type { BoardDef } from "../tenant-store/schema.ts";
import type { RoadmapRow } from "./genealogy.ts";
import type { ReleaseSnapshot } from "./releases.ts";
import {
  boardSummaryFromRow,
  poolFromRows,
  releaseSnapshotsFromRows,
  roadmapRowsFromRows,
  LATEST_SNAPSHOT_SQL,
  POOL_SQL,
  RELEASE_SNAPSHOTS_SQL,
  ROADMAP_SQL,
  type BoardSummary,
  type RoadmapRowRow,
  type SnapshotRow,
  type StoredSnapshotRow,
} from "./store.ts";

export async function loadRegistryAsync(db: AsyncSqlDb): Promise<BoardDef[]> {
  return registryFromRows(await db.all<BoardRow>(REGISTRY_SQL));
}

export async function loadBoardSummariesAsync(db: AsyncSqlDb): Promise<BoardSummary[]> {
  const boards = await loadRegistryAsync(db);
  const summaries: BoardSummary[] = [];
  for (const board of boards) {
    const rows = await db.all<SnapshotRow>(LATEST_SNAPSHOT_SQL, [board.id]);
    summaries.push(boardSummaryFromRow(board, rows[0]));
  }
  return summaries;
}

export async function loadPoolAsync(db: AsyncSqlDb): Promise<ProjectPoolItem[]> {
  return poolFromRows(await db.all<{ item_json: string }>(POOL_SQL));
}

export async function loadRoadmapRowsAsync(
  db: AsyncSqlDb,
  pool: readonly ProjectPoolItem[],
): Promise<RoadmapRow[]> {
  return roadmapRowsFromRows(await db.all<RoadmapRowRow>(ROADMAP_SQL), pool);
}

export async function loadReleaseSnapshotsAsync(
  db: AsyncSqlDb,
  pool: readonly ProjectPoolItem[],
): Promise<ReleaseSnapshot[]> {
  return releaseSnapshotsFromRows(await db.all<StoredSnapshotRow>(RELEASE_SNAPSHOTS_SQL), pool);
}
