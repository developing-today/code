/**
 * Admin before/after comparison over the `snapshot-audit` set (#9140
 * design record §5.4, build slice 5).
 *
 * An admin pins a stored version of any retained object into the audit
 * set, then compares two pinned versions of the same object. A pin is a
 * copy under `audit:<object id>` in the `snapshot-audit` class, whose
 * inputs name the source version, so the report reads only audit
 * envelopes and payloads: it still works after the source version has
 * been retired from its own, shorter class.
 *
 * Recommendation scores are compared only within one rule id (design Q2):
 * when the two versions name different rules, the report still lists
 * added, removed and moved items but no score changes.
 */
import { RetainedObjectError, RetainedObjectStore, type Envelope, type Family } from "./retained_object";

export type AuditErrorCode = "not_found" | "not_audited" | "family_mismatch";

export class AuditError extends Error {
  constructor(
    readonly code: AuditErrorCode,
    message: string,
  ) {
    super(message);
    this.name = "AuditError";
  }
}

/** The audit object that holds pins of `objectId`. */
export function auditObjectId(objectId: string): string {
  return `audit:${objectId}`;
}

function pinInputs(version: number): string {
  return `version:${version}`;
}

export interface Pin {
  object_id: string;
  version: number;
  audit_version: number;
  pinned: boolean;
}

/**
 * Copy one stored version into the audit set. Pinning the same version
 * again returns the existing pin. Refuses while `snapshot-audit` is
 * disabled.
 */
export async function pinForAudit(
  db: D1Database,
  objectId: string,
  version: number,
  now: () => number = Date.now,
): Promise<Pin> {
  const store = new RetainedObjectStore(db, now);
  const rule = await store.getClass("snapshot-audit");
  if (!rule?.enabled) {
    throw new RetainedObjectError("retention_class_disabled", "class snapshot-audit is not enabled for writes");
  }
  const audit = auditObjectId(objectId);
  const existing = await store.byInputs(audit, pinInputs(version));
  if (existing) return { object_id: objectId, version, audit_version: existing.version, pinned: false };
  const source = await store.version(objectId, version);
  if (!source) throw new AuditError("not_found", `${objectId} has no version ${version}`);
  const stored = await store.append({
    objectId: audit,
    family: source.family,
    class: "snapshot-audit",
    capturedAt: source.capturedAt,
    inputsRef: pinInputs(version),
    payload: source.payload,
  });
  return { object_id: objectId, version, audit_version: stored.version, pinned: true };
}

interface Item {
  id: string;
  score_micros: number;
}

export interface ItemChanges {
  /** Whether scores are comparable: the same rule produced both. */
  comparable: boolean;
  added: string[];
  removed: string[];
  moved: Array<{ id: string; from_rank: number; to_rank: number }>;
  rescored: Array<{ id: string; from_micros: number; to_micros: number }>;
}

export interface ComparisonReport {
  object_id: string;
  family: Family;
  from: { version: number; captured_at: string };
  to: { version: number; captured_at: string };
  /** Top-level payload fields whose values differ, by name. */
  fields_changed: string[];
  items?: ItemChanges;
  result_refs?: { added: string[]; removed: string[] };
}

function record(value: unknown): Record<string, unknown> {
  return value !== null && typeof value === "object" && !Array.isArray(value) ? (value as Record<string, unknown>) : {};
}

function items(payload: Record<string, unknown>): Item[] | undefined {
  const value = payload.items;
  if (!Array.isArray(value)) return undefined;
  return value.filter(
    (item): item is Item =>
      typeof item === "object" &&
      item !== null &&
      typeof (item as Item).id === "string" &&
      typeof (item as Item).score_micros === "number",
  );
}

function strings(value: unknown): string[] | undefined {
  return Array.isArray(value) ? value.filter((entry): entry is string => typeof entry === "string") : undefined;
}

function compareItems(before: Item[], after: Item[], comparable: boolean): ItemChanges {
  const beforeRank = new Map(before.map((item, index) => [item.id, index + 1]));
  const afterRank = new Map(after.map((item, index) => [item.id, index + 1]));
  const beforeScore = new Map(before.map((item) => [item.id, item.score_micros]));
  const changes: ItemChanges = { comparable, added: [], removed: [], moved: [], rescored: [] };
  for (const item of after) {
    const from = beforeRank.get(item.id);
    if (from === undefined) {
      changes.added.push(item.id);
      continue;
    }
    const to = afterRank.get(item.id)!;
    if (from !== to) changes.moved.push({ id: item.id, from_rank: from, to_rank: to });
    const old = beforeScore.get(item.id)!;
    if (comparable && old !== item.score_micros) {
      changes.rescored.push({ id: item.id, from_micros: old, to_micros: item.score_micros });
    }
  }
  for (const item of before) if (!afterRank.has(item.id)) changes.removed.push(item.id);
  return changes;
}

/** The before/after report for two envelopes of one family, `from` and `to` naming their source versions. */
export function compareEnvelopes(
  objectId: string,
  from: number,
  before: Envelope,
  to: number,
  after: Envelope,
): ComparisonReport {
  if (before.family !== after.family) {
    throw new AuditError("family_mismatch", `${before.family} and ${after.family} cannot be compared`);
  }
  const a = record(before.payload);
  const b = record(after.payload);
  const fields = [...new Set([...Object.keys(a), ...Object.keys(b)])].sort();
  const report: ComparisonReport = {
    object_id: objectId,
    family: before.family,
    from: { version: from, captured_at: before.capturedAt },
    to: { version: to, captured_at: after.capturedAt },
    fields_changed: fields.filter((field) => JSON.stringify(a[field]) !== JSON.stringify(b[field])),
  };
  const beforeItems = items(a);
  const afterItems = items(b);
  if (beforeItems && afterItems) {
    const comparable = before.family !== "recommendation" || a.rule_id === b.rule_id;
    report.items = compareItems(beforeItems, afterItems, comparable);
  }
  const beforeRefs = strings(a.result_refs);
  const afterRefs = strings(b.result_refs);
  if (beforeRefs && afterRefs) {
    const was = new Set(beforeRefs);
    const now = new Set(afterRefs);
    report.result_refs = {
      added: afterRefs.filter((ref) => !was.has(ref)),
      removed: beforeRefs.filter((ref) => !now.has(ref)),
    };
  }
  return report;
}

/** Compare two pinned versions of one object, reading only the audit set. */
export async function compareAudited(
  db: D1Database,
  objectId: string,
  from: number,
  to: number,
): Promise<ComparisonReport> {
  const store = new RetainedObjectStore(db);
  const audit = auditObjectId(objectId);
  const [before, after] = await Promise.all([
    store.byInputs(audit, pinInputs(from)),
    store.byInputs(audit, pinInputs(to)),
  ]);
  const missing = [before ? undefined : from, after ? undefined : to].filter((v) => v !== undefined);
  if (missing.length > 0) {
    throw new AuditError("not_audited", `${objectId} version ${missing.join(" and ")} is not pinned for audit`);
  }
  return compareEnvelopes(objectId, from, before!, to, after!);
}
