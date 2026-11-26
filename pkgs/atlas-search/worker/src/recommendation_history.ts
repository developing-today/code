/**
 * The recommendation family of retained objects (#9140 design record,
 * build slice 2): one `recommendation` version per recompute.
 *
 * A recompute is a new pinned snapshot in the recommendation accumulator
 * (`recommendation_do.ts`). Its envelope names the parent snapshot as its
 * inputs, and its payload is the ordered item refs with their scores and
 * the producing rule id, so the same parent reproduces the same payload
 * (design §7, slice 2 acceptance).
 *
 * The writer is off unless the accumulator has a `RETAINED` D1 binding and
 * the `recommendation` class is enabled there (design §6).
 */
import type { RecommendationSnapshot } from "./recommendation_state";
import { RetainedObjectStore, type Envelope, type EnvelopeDraft } from "./retained_object";

export interface RecommendationPayload {
  /** The rule or model that produced the order (Q2: compare within one id). */
  rule_id: string;
  parent_snapshot_id: string;
  input_digest: string;
  items: Array<{ id: string; score_micros: number }>;
}

/** The logical object a tenant and subject's recommendations version under. */
export function recommendationObjectId(tenant: string, subject: string): string {
  return `recommendation:${encodeURIComponent(tenant)}/${encodeURIComponent(subject)}`;
}

export function recommendationInputsRef(parentSnapshotId: string): string {
  return `parent-snapshot:${parentSnapshotId}`;
}

/** The envelope draft for one recompute. */
export function recommendationDraft(
  snapshot: RecommendationSnapshot,
  tenant: string,
  subject: string,
  capturedAt: Date,
): EnvelopeDraft<RecommendationPayload> {
  const body = snapshot.parent.body;
  return {
    objectId: recommendationObjectId(tenant, subject),
    family: "recommendation",
    class: "recommendation",
    capturedAt: capturedAt.toISOString(),
    inputsRef: recommendationInputsRef(snapshot.parent_snapshot_id),
    payload: {
      rule_id: body.algorithm,
      parent_snapshot_id: snapshot.parent_snapshot_id,
      input_digest: body.input_digest,
      items: snapshot.ordered.map((entry) => ({ id: entry.id, score_micros: entry.score_micros })),
    },
  };
}

/**
 * Whether a stored version is what `snapshot` produces: the same inputs,
 * rule and ordered items.
 */
export function reproduces(stored: Envelope, snapshot: RecommendationSnapshot): boolean {
  const expected = recommendationDraft(snapshot, "", "", new Date(0));
  return (
    stored.family === "recommendation" &&
    stored.inputsRef === expected.inputsRef &&
    JSON.stringify(stored.payload) === JSON.stringify(expected.payload)
  );
}

export type RecordOutcome = { recorded: true; version: number } | { recorded: false; reason: "off" };

/**
 * Append one recompute when the class is enabled. Returns what happened;
 * a store failure is thrown to the caller.
 */
export async function recordRecommendation(
  db: D1Database | undefined,
  snapshot: RecommendationSnapshot,
  tenant: string,
  subject: string,
  now: () => number = Date.now,
): Promise<RecordOutcome> {
  if (!db) return { recorded: false, reason: "off" };
  const store = new RetainedObjectStore(db, now);
  const rule = await store.getClass("recommendation");
  if (!rule?.enabled) return { recorded: false, reason: "off" };
  const stored = await store.append(recommendationDraft(snapshot, tenant, subject, new Date(now())));
  return { recorded: true, version: stored.version };
}
