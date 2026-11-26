/**
 * Retained Atlas objects (#9140 design record, build slice 1).
 *
 * Recommendations, search history and hot-data snapshots are stored as
 * versioned, timestamped snapshots that share one envelope (design §5.1),
 * each under a retention class (§5.3). This module is the envelope, the
 * four classes and the store over the `atlas_retention_classes` and
 * `atlas_retained_objects` tables (migrations/0003_retained_objects.sql).
 *
 * Nothing writes through it yet: each family's writer lands in its own
 * slice, behind its class's `enabled` flag, which defaults to off (§6).
 * Retirement belongs to the retention rule engine (slice 6); this module
 * deletes nothing.
 */

export type Family = "recommendation" | "search_history" | "hot_snapshot";

export type RetentionClassName = "hot-short" | "recommendation" | "search-history" | "snapshot-audit";

export const FAMILIES: readonly Family[] = ["recommendation", "search_history", "hot_snapshot"];

export const CLASS_NAMES: readonly RetentionClassName[] = [
  "hot-short",
  "recommendation",
  "search-history",
  "snapshot-audit",
];

const HOUR_MS = 60 * 60 * 1000;
const DAY_MS = 24 * HOUR_MS;

/** One class's retention rule. */
export interface RetentionClass {
  name: RetentionClassName;
  /** How long a version is kept before retirement may consider it. */
  windowMs: number;
  /** The newest N versions of an object are never retired. */
  keepNewest: number;
  /** Whether a search-history payload may keep its query text (Q1). */
  allowsQueryText: boolean;
  /** Writers stay off until a class is enabled (§6). */
  enabled: boolean;
}

/** The design defaults (§5.3). A class may only lengthen or raise them. */
export const DEFAULT_CLASSES: Readonly<Record<RetentionClassName, RetentionClass>> = {
  "hot-short": { name: "hot-short", windowMs: DAY_MS, keepNewest: 2, allowsQueryText: false, enabled: false },
  recommendation: {
    name: "recommendation",
    windowMs: 30 * DAY_MS,
    keepNewest: 10,
    allowsQueryText: false,
    enabled: false,
  },
  "search-history": {
    name: "search-history",
    windowMs: 90 * DAY_MS,
    keepNewest: 20,
    allowsQueryText: false,
    enabled: false,
  },
  "snapshot-audit": {
    name: "snapshot-audit",
    windowMs: 365 * DAY_MS,
    keepNewest: 50,
    allowsQueryText: false,
    enabled: false,
  },
};

/** The family a class holds; snapshot-audit holds any family. */
const CLASS_FAMILY: Readonly<Record<RetentionClassName, Family | undefined>> = {
  "hot-short": "hot_snapshot",
  recommendation: "recommendation",
  "search-history": "search_history",
  "snapshot-audit": undefined,
};

/** A typed refusal: `code` is stable, `message` is for people. */
export class RetainedObjectError extends Error {
  constructor(
    readonly code:
      | "retention_below_default"
      | "unknown_retention_class"
      | "unknown_family"
      | "family_class_mismatch"
      | "malformed_envelope"
      | "retention_class_disabled"
      | "version_conflict",
    message: string,
  ) {
    super(message);
    this.name = "RetainedObjectError";
  }
}

function isClassName(value: unknown): value is RetentionClassName {
  return typeof value === "string" && (CLASS_NAMES as readonly string[]).includes(value);
}

function isFamily(value: unknown): value is Family {
  return typeof value === "string" && (FAMILIES as readonly string[]).includes(value);
}

/**
 * The narrowing check (§5.3, rule engine invariant 2): a class rule may
 * lengthen the window or raise the floor, never go below the default.
 */
export function checkClass(rule: RetentionClass): RetentionClass {
  if (!isClassName(rule.name)) {
    throw new RetainedObjectError("unknown_retention_class", `no retention class ${String(rule.name)}`);
  }
  const base = DEFAULT_CLASSES[rule.name];
  if (!Number.isSafeInteger(rule.windowMs) || rule.windowMs < base.windowMs) {
    throw new RetainedObjectError(
      "retention_below_default",
      `class ${rule.name} keeps versions for at least ${base.windowMs} ms, not ${rule.windowMs}`,
    );
  }
  if (!Number.isSafeInteger(rule.keepNewest) || rule.keepNewest < base.keepNewest) {
    throw new RetainedObjectError(
      "retention_below_default",
      `class ${rule.name} keeps at least the newest ${base.keepNewest} versions, not ${rule.keepNewest}`,
    );
  }
  return rule;
}

/** One version of one retained object (§5.1). */
export interface Envelope<P = unknown> {
  objectId: string;
  family: Family;
  class: RetentionClassName;
  /** RFC 3339 time the snapshot was taken. */
  capturedAt: string;
  /** Monotonic per objectId, from 1. */
  version: number;
  /** What produced it: query ids, a retrieval-stats version id. */
  inputsRef: string;
  payload: P;
}

/** What a writer hands the store: the envelope without its version. */
export type EnvelopeDraft<P = unknown> = Omit<Envelope<P>, "version">;

const RFC3339 = /^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}(\.\d+)?(Z|[+-]\d{2}:\d{2})$/;

/** Check a draft's shape and that its class holds its family. */
export function checkDraft<P>(draft: EnvelopeDraft<P>): EnvelopeDraft<P> {
  if (typeof draft.objectId !== "string" || draft.objectId.length === 0) {
    throw new RetainedObjectError("malformed_envelope", "objectId is empty");
  }
  if (!isFamily(draft.family)) {
    throw new RetainedObjectError("unknown_family", `no family ${String(draft.family)}`);
  }
  if (!isClassName(draft.class)) {
    throw new RetainedObjectError("unknown_retention_class", `no retention class ${String(draft.class)}`);
  }
  const holds = CLASS_FAMILY[draft.class];
  if (holds !== undefined && holds !== draft.family) {
    throw new RetainedObjectError("family_class_mismatch", `class ${draft.class} holds ${holds}, not ${draft.family}`);
  }
  if (!RFC3339.test(draft.capturedAt) || Number.isNaN(Date.parse(draft.capturedAt))) {
    throw new RetainedObjectError("malformed_envelope", `capturedAt ${draft.capturedAt} is not RFC 3339`);
  }
  if (typeof draft.inputsRef !== "string" || draft.inputsRef.length === 0) {
    throw new RetainedObjectError("malformed_envelope", "inputsRef is empty");
  }
  if (draft.payload === undefined) {
    throw new RetainedObjectError("malformed_envelope", "payload is missing");
  }
  return draft;
}

/**
 * The payload as stored: a search-history payload loses its `query` text
 * unless its class allows query text (NG5, Q1 default).
 */
export function storedPayload<P>(family: Family, rule: RetentionClass, payload: P): P {
  if (family !== "search_history" || rule.allowsQueryText) return payload;
  if (payload === null || typeof payload !== "object" || Array.isArray(payload)) return payload;
  if (!("query" in payload)) return payload;
  const { query: _dropped, ...rest } = payload as Record<string, unknown>;
  return rest as P;
}

interface ClassRow {
  class: string;
  window_ms: number;
  keep_newest: number;
  allows_query_text: number;
  enabled: number;
}

interface ObjectRow {
  object_id: string;
  version: number;
  family: string;
  class: string;
  captured_at: string;
  inputs_ref: string;
  payload: string;
}

function fromClassRow(row: ClassRow): RetentionClass {
  if (!isClassName(row.class)) {
    throw new RetainedObjectError("unknown_retention_class", `no retention class ${row.class}`);
  }
  return {
    name: row.class,
    windowMs: row.window_ms,
    keepNewest: row.keep_newest,
    allowsQueryText: row.allows_query_text === 1,
    enabled: row.enabled === 1,
  };
}

function fromObjectRow(row: ObjectRow): Envelope {
  if (!isFamily(row.family) || !isClassName(row.class)) {
    throw new RetainedObjectError("malformed_envelope", `stored row ${row.object_id}@${row.version} is malformed`);
  }
  return {
    objectId: row.object_id,
    version: row.version,
    family: row.family,
    class: row.class,
    capturedAt: row.captured_at,
    inputsRef: row.inputs_ref,
    payload: JSON.parse(row.payload) as unknown,
  };
}

export class RetainedObjectStore {
  constructor(
    private readonly db: D1Database,
    private readonly now: () => number = Date.now,
  ) {}

  /** Write each class at its default unless it is already set. */
  async seedDefaults(): Promise<void> {
    const at = this.now();
    await this.db.batch(
      CLASS_NAMES.map((name) => {
        const rule = DEFAULT_CLASSES[name];
        return this.db
          .prepare(
            "INSERT INTO atlas_retention_classes (class, window_ms, keep_newest, allows_query_text, enabled, updated_at_ms) VALUES (?, ?, ?, ?, ?, ?) ON CONFLICT (class) DO NOTHING",
          )
          .bind(name, rule.windowMs, rule.keepNewest, rule.allowsQueryText ? 1 : 0, rule.enabled ? 1 : 0, at);
      }),
    );
  }

  /** Set one class, refusing anything below its default. */
  async setClass(rule: RetentionClass): Promise<void> {
    checkClass(rule);
    await this.db
      .prepare(
        "INSERT INTO atlas_retention_classes (class, window_ms, keep_newest, allows_query_text, enabled, updated_at_ms) VALUES (?, ?, ?, ?, ?, ?) ON CONFLICT (class) DO UPDATE SET window_ms = excluded.window_ms, keep_newest = excluded.keep_newest, allows_query_text = excluded.allows_query_text, enabled = excluded.enabled, updated_at_ms = excluded.updated_at_ms",
      )
      .bind(rule.name, rule.windowMs, rule.keepNewest, rule.allowsQueryText ? 1 : 0, rule.enabled ? 1 : 0, this.now())
      .run();
  }

  async getClass(name: RetentionClassName): Promise<RetentionClass | undefined> {
    const row = await this.db
      .prepare(
        "SELECT class, window_ms, keep_newest, allows_query_text, enabled FROM atlas_retention_classes WHERE class = ?",
      )
      .bind(name)
      .first<ClassRow>();
    return row ? fromClassRow(row) : undefined;
  }

  /**
   * Append the next version of `draft.objectId`. With `expectedVersion`
   * the append happens only while that is the current version (0 for a
   * new object), so two writers cannot both land the same version.
   * Refuses while the class is disabled. Returns the stored envelope.
   */
  async append<P>(draft: EnvelopeDraft<P>, expectedVersion?: number): Promise<Envelope<P>> {
    checkDraft(draft);
    const rule = await this.getClass(draft.class);
    if (!rule?.enabled) {
      throw new RetainedObjectError("retention_class_disabled", `class ${draft.class} is not enabled for writes`);
    }
    const payload = storedPayload(draft.family, rule, draft.payload);
    const row = await this.db
      .prepare(
        `INSERT INTO atlas_retained_objects
           (object_id, version, family, class, captured_at, inputs_ref, payload, written_at_ms)
         SELECT ?1, current.version + 1, ?2, ?3, ?4, ?5, ?6, ?7
         FROM (SELECT COALESCE(MAX(version), 0) AS version FROM atlas_retained_objects WHERE object_id = ?1) AS current
         WHERE ?8 IS NULL OR current.version = ?8
         RETURNING version`,
      )
      .bind(
        draft.objectId,
        draft.family,
        draft.class,
        draft.capturedAt,
        draft.inputsRef,
        JSON.stringify(payload),
        this.now(),
        expectedVersion ?? null,
      )
      .first<{ version: number }>();
    if (!row) {
      const current = await this.current(draft.objectId);
      throw new RetainedObjectError(
        "version_conflict",
        `expected ${draft.objectId} at version ${expectedVersion}, found ${current?.version ?? 0}`,
      );
    }
    return { ...draft, payload, version: row.version };
  }

  /** The highest version of one object, if any. */
  async current(objectId: string): Promise<Envelope | undefined> {
    const row = await this.db
      .prepare(
        "SELECT object_id, version, family, class, captured_at, inputs_ref, payload FROM atlas_retained_objects WHERE object_id = ? ORDER BY version DESC LIMIT 1",
      )
      .bind(objectId)
      .first<ObjectRow>();
    return row ? fromObjectRow(row) : undefined;
  }

  /** One version of one object, if it is stored. */
  async version(objectId: string, version: number): Promise<Envelope | undefined> {
    const row = await this.db
      .prepare(
        "SELECT object_id, version, family, class, captured_at, inputs_ref, payload FROM atlas_retained_objects WHERE object_id = ? AND version = ?",
      )
      .bind(objectId, version)
      .first<ObjectRow>();
    return row ? fromObjectRow(row) : undefined;
  }

  /**
   * Mark retired every version of `objectId` that its class rule lets go
   * (design §5.3): older than the class window and outside the class's
   * keep-newest floor, counted over the object's versions. Retiring is the
   * cold step; nothing is deleted. Returns the versions retired by this
   * call, oldest first.
   */
  async retireExpired(objectId: string): Promise<number[]> {
    const at = this.now();
    const { results } = await this.db
      .prepare(
        `UPDATE atlas_retained_objects AS o
           SET retired_at_ms = ?2
         WHERE o.object_id = ?1
           AND o.retired_at_ms IS NULL
           AND o.version <= (SELECT MAX(version) FROM atlas_retained_objects WHERE object_id = ?1)
                            - (SELECT c.keep_newest FROM atlas_retention_classes AS c WHERE c.class = o.class)
           AND o.written_at_ms <= ?2 - (SELECT c.window_ms FROM atlas_retention_classes AS c WHERE c.class = o.class)
         RETURNING version`,
      )
      .bind(objectId, at)
      .all<{ version: number }>();
    return results.map((row) => row.version).sort((a, b) => a - b);
  }

  /** The versions of one object that are not retired, oldest first. */
  async live(objectId: string): Promise<Envelope[]> {
    const { results } = await this.db
      .prepare(
        "SELECT object_id, version, family, class, captured_at, inputs_ref, payload FROM atlas_retained_objects WHERE object_id = ? AND retired_at_ms IS NULL ORDER BY version",
      )
      .bind(objectId)
      .all<ObjectRow>();
    return results.map(fromObjectRow);
  }

  /** The retired versions of one object, oldest first. */
  async retired(objectId: string): Promise<number[]> {
    const { results } = await this.db
      .prepare(
        "SELECT version FROM atlas_retained_objects WHERE object_id = ? AND retired_at_ms IS NOT NULL ORDER BY version",
      )
      .bind(objectId)
      .all<{ version: number }>();
    return results.map((row) => row.version);
  }

  /** Every stored version of one object, oldest first. */
  async history(objectId: string): Promise<Envelope[]> {
    const { results } = await this.db
      .prepare(
        "SELECT object_id, version, family, class, captured_at, inputs_ref, payload FROM atlas_retained_objects WHERE object_id = ? ORDER BY version",
      )
      .bind(objectId)
      .all<ObjectRow>();
    return results.map(fromObjectRow);
  }
}
