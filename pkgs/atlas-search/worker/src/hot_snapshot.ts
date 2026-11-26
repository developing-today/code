/**
 * The hot-snapshot family of retained objects (#9140 design record, build
 * slice 4): a bounded window of the "what's hot" data, captured on the
 * recommendation accumulator's cadence and kept under the short
 * `hot-short` class (24 h window, keep newest 2).
 *
 * Each time the accumulator pins a new snapshot it captures the top of
 * the order (at most `HOT_WINDOW` items) as the next `hot_snapshot`
 * version for that tenant and subject, then retires the versions the
 * class lets go. Retiring marks a version cold; nothing is deleted.
 *
 * The writer is off unless the accumulator has a `RETAINED` D1 binding and
 * the `hot-short` class is enabled there (design §6).
 */
import type { RecommendationSnapshot } from "./recommendation_state";
import { RetainedObjectStore, type EnvelopeDraft } from "./retained_object";

/** How many of the top items one hot snapshot holds. */
export const HOT_WINDOW = 10;

export interface HotSnapshotPayload {
  parent_snapshot_id: string;
  window: number;
  items: Array<{ id: string; score_micros: number }>;
}

/** The logical object a tenant and subject's hot snapshots version under. */
export function hotSnapshotObjectId(tenant: string, subject: string): string {
  return `hot:${encodeURIComponent(tenant)}/${encodeURIComponent(subject)}`;
}

/** The envelope draft for one capture. */
export function hotSnapshotDraft(
  snapshot: RecommendationSnapshot,
  tenant: string,
  subject: string,
  capturedAt: Date,
): EnvelopeDraft<HotSnapshotPayload> {
  return {
    objectId: hotSnapshotObjectId(tenant, subject),
    family: "hot_snapshot",
    class: "hot-short",
    capturedAt: capturedAt.toISOString(),
    inputsRef: `parent-snapshot:${snapshot.parent_snapshot_id}`,
    payload: {
      parent_snapshot_id: snapshot.parent_snapshot_id,
      window: HOT_WINDOW,
      items: snapshot.ordered
        .slice(0, HOT_WINDOW)
        .map((entry) => ({ id: entry.id, score_micros: entry.score_micros })),
    },
  };
}

export type HotSnapshotOutcome =
  | { recorded: true; version: number; retired: number[] }
  | { recorded: false; reason: "off" };

/**
 * Capture one hot snapshot and retire what the class lets go. Returns what
 * happened; a store failure is thrown to the caller.
 */
export async function recordHotSnapshot(
  db: D1Database | undefined,
  snapshot: RecommendationSnapshot,
  tenant: string,
  subject: string,
  now: () => number = Date.now,
): Promise<HotSnapshotOutcome> {
  if (!db) return { recorded: false, reason: "off" };
  const store = new RetainedObjectStore(db, now);
  const rule = await store.getClass("hot-short");
  if (!rule?.enabled) return { recorded: false, reason: "off" };
  const stored = await store.append(
    hotSnapshotDraft(snapshot, tenant, subject, new Date(now())),
  );
  const retired = await store.retireExpired(stored.objectId);
  return { recorded: true, version: stored.version, retired };
}
