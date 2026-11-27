// Reference records for the content-addressed media/data plane
// (component F of #9078, issue #12803, slice 1 of #9084; design
// record doc/2026-11-23T03-04-00Z_design_content-addressed-media-data-plane,
// sections 4-6).
//
// The schema lives in references/migrations/0001_reference_record.sql
// and runs in each tenant database and in the public database of
// component E's data plane (#9083 section 2.2) -- never in the
// central tenancy DB. This module is the store and the access check
// over that schema, written against the same minimal SqlStorage
// interface as user-do.ts so it runs unchanged on a Durable Object's
// SQLite storage or any SQLite/Turso database behind that interface.
//
// The access rule (design section 5), evaluated against authoritative
// reference state at capability-minting time: a caller may read
// content C in partition P iff
//   1. a live (non-tombstoned) reference to C exists in a database
//      the caller is authorized to read, the reference's partition
//      equals P, and the reference's class permits the operation; or
//   2. a live publication reference to C exists in the public DB
//      (tier `public` only).
// A content hash is an identifier, never a credential: this check is
// the only path, and it never grants on hash presentation alone.
// Capability minting itself is slice #12807; this module is the
// check that slice must call.

import { FailClosed, parseBindingId, type Jurisdiction } from "./routing.ts";

export type { Jurisdiction };

/** Sensitivity tiers (design section 4). A partition is the pair
 * (jurisdiction, sensitivity tier); content never dedups across
 * partitions (#3401). */
export const SENSITIVITY_TIERS = ["public", "tenant-private", "tenant-restricted", "staging"] as const;
export type SensitivityTier = (typeof SENSITIVITY_TIERS)[number];

export interface Partition {
  jurisdiction: Jurisdiction;
  tier: SensitivityTier;
}

/** Reference classes (design sections 4-5). `standard` is the
 * ordinary tenant reference. `restricted` is the class capability
 * minting additionally requires for tenant-restricted content; it
 * may only be set by an authorized writer of the source content.
 * `publication` marks a publication reference: valid only in the
 * public database, only at tier `public`. */
export const REFERENCE_CLASSES = ["standard", "restricted", "publication"] as const;
export type ReferenceClass = (typeof REFERENCE_CLASSES)[number];

export interface ReferenceRow {
  reference_id: string;
  content_id: string;
  jurisdiction: Jurisdiction;
  sensitivity_tier: SensitivityTier;
  class: ReferenceClass;
  created_by: string;
  created_at: string;
  tombstoned_at: string | null;
}

export type ReferenceEventType = "reference.created" | "reference.removed";

export interface ReferenceEvent {
  seq: number;
  event_id: string;
  event_type: ReferenceEventType;
  reference_id: string;
  content_id: string;
  jurisdiction: Jurisdiction;
  sensitivity_tier: SensitivityTier;
  class: ReferenceClass;
  actor: string;
  occurred_at: string;
}

interface SqlStorage {
  exec(query: string, ...params: unknown[]): { toArray(): Record<string, unknown>[] };
}

const CONTENT_ID_RE = /^[0-9a-f]{64}$/;

/** Content IDs are #9106 Part A content addresses; in this plane the
 * sha256 shape the landed `blobs` table already uses. Validation is
 * shape-only -- a well-formed hash still grants nothing by itself. */
export function parseContentId(raw: string): string {
  if (!CONTENT_ID_RE.test(raw)) {
    throw new FailClosed(400, "invalid_content_id", "content id must be 64 lowercase hex characters");
  }
  return raw;
}

function randomHexId(): string {
  const bytes = new Uint8Array(16);
  crypto.getRandomValues(bytes);
  return Array.from(bytes, (b) => b.toString(16).padStart(2, "0")).join("");
}

/** The DDL of references/migrations/0001_reference_record.sql, one
 * statement per entry. The migration file is the canonical artifact
 * for Turso-side application; this copy bootstraps stores that own
 * their schema (the DO pattern of user-do.ts). A test asserts the
 * two cannot drift. */
export const REFERENCE_SCHEMA_STATEMENTS: readonly string[] = [
  `CREATE TABLE IF NOT EXISTS content_reference (
  reference_id     TEXT PRIMARY KEY,
  content_id       TEXT NOT NULL,
  jurisdiction     TEXT NOT NULL,
  sensitivity_tier TEXT NOT NULL,
  class            TEXT NOT NULL,
  created_by       TEXT NOT NULL,
  created_at       TEXT NOT NULL,
  tombstoned_at    TEXT,
  CHECK (jurisdiction IN ('us','eu','fedramp')),
  CHECK (sensitivity_tier IN ('public','tenant-private','tenant-restricted','staging')),
  CHECK (class IN ('standard','restricted','publication'))
)`,
  `CREATE UNIQUE INDEX IF NOT EXISTS content_reference_one_live
  ON content_reference (content_id, jurisdiction, sensitivity_tier, class, created_by)
  WHERE tombstoned_at IS NULL`,
  `CREATE INDEX IF NOT EXISTS content_reference_by_content
  ON content_reference (content_id, jurisdiction, sensitivity_tier)`,
  `CREATE TABLE IF NOT EXISTS reference_outbox (
  seq              INTEGER PRIMARY KEY AUTOINCREMENT,
  event_id         TEXT NOT NULL UNIQUE,
  event_type       TEXT NOT NULL,
  reference_id     TEXT NOT NULL,
  content_id       TEXT NOT NULL,
  jurisdiction     TEXT NOT NULL,
  sensitivity_tier TEXT NOT NULL,
  class            TEXT NOT NULL,
  actor            TEXT NOT NULL,
  occurred_at      TEXT NOT NULL,
  CHECK (event_type IN ('reference.created','reference.removed'))
)`,
];

function toReferenceRow(r: Record<string, unknown>): ReferenceRow {
  return {
    reference_id: String(r.reference_id),
    content_id: String(r.content_id),
    jurisdiction: String(r.jurisdiction) as Jurisdiction,
    sensitivity_tier: String(r.sensitivity_tier) as SensitivityTier,
    class: String(r.class) as ReferenceClass,
    created_by: String(r.created_by),
    created_at: String(r.created_at),
    tombstoned_at: r.tombstoned_at === null ? null : String(r.tombstoned_at),
  };
}

function toReferenceEvent(r: Record<string, unknown>): ReferenceEvent {
  return {
    seq: Number(r.seq),
    event_id: String(r.event_id),
    event_type: String(r.event_type) as ReferenceEventType,
    reference_id: String(r.reference_id),
    content_id: String(r.content_id),
    jurisdiction: String(r.jurisdiction) as Jurisdiction,
    sensitivity_tier: String(r.sensitivity_tier) as SensitivityTier,
    class: String(r.class) as ReferenceClass,
    actor: String(r.actor),
    occurred_at: String(r.occurred_at),
  };
}

export interface CreateReferenceInput {
  contentId: string;
  refClass: ReferenceClass;
  /** binding_id of the writer, via the hosting DB's authorized write
   * path (the caller of this store has already authenticated it). */
  createdBy: string;
  /** True only when createdBy is an authorized writer of the source
   * content. Required to mint a `restricted`-class reference
   * (design section 5); ignored for other classes. */
  canMintRestricted?: boolean;
  now?: string;
}

/** One database's reference store. The store is bound to the
 * database's own partition: rows it writes carry that partition, so
 * a reference can never be forged into a different partition from
 * inside this database (design section 5). Whether the database is
 * the public DB is a fact about the database, stated at
 * construction, not a per-call claim. */
export class ReferenceStore {
  private sql: SqlStorage;
  readonly dbId: string;
  readonly partition: Partition;
  readonly isPublicDb: boolean;
  private newId: () => string;

  constructor(
    sql: SqlStorage,
    opts: { dbId: string; partition: Partition; isPublicDb?: boolean; newId?: () => string },
  ) {
    this.sql = sql;
    this.dbId = opts.dbId;
    this.partition = opts.partition;
    this.isPublicDb = opts.isPublicDb ?? false;
    this.newId = opts.newId ?? randomHexId;
  }

  ensureSchema(): void {
    for (const stmt of REFERENCE_SCHEMA_STATEMENTS) this.sql.exec(stmt);
  }

  /** Create a reference in this database. Idempotent: if the same
   * writer already holds a live reference with the same content,
   * partition and class, the existing row is returned with
   * `created: false` and no second outbox event is emitted (design
   * section 6: referencing is idempotent and never needs to know GC
   * exists). */
  createReference(input: CreateReferenceInput): { row: ReferenceRow; created: boolean } {
    const contentId = parseContentId(input.contentId);
    const createdBy = parseBindingId(input.createdBy);
    if (!REFERENCE_CLASSES.includes(input.refClass)) {
      throw new FailClosed(400, "invalid_reference_class", `unknown reference class: ${input.refClass}`);
    }
    if (this.partition.tier === "staging") {
      // Staged content is not yet referenceable (design section 4):
      // publication happens only after verification, elsewhere.
      throw new FailClosed(409, "staging_not_referenceable", "staged content cannot be referenced");
    }
    if (input.refClass === "restricted" && input.canMintRestricted !== true) {
      throw new FailClosed(
        403,
        "restricted_class_not_authorized",
        "minting a restricted-class reference requires an authorized writer of the source content",
      );
    }
    if (input.refClass === "publication" && !(this.isPublicDb && this.partition.tier === "public")) {
      throw new FailClosed(
        403,
        "publication_reference_outside_public_db",
        "a publication reference is only valid in the public database at tier public",
      );
    }
    const existing = this.sql
      .exec(
        `SELECT * FROM content_reference
          WHERE content_id = ? AND jurisdiction = ? AND sensitivity_tier = ?
            AND class = ? AND created_by = ? AND tombstoned_at IS NULL`,
        contentId,
        this.partition.jurisdiction,
        this.partition.tier,
        input.refClass,
        createdBy,
      )
      .toArray()[0];
    if (existing) return { row: toReferenceRow(existing), created: false };

    const now = input.now ?? new Date().toISOString();
    const row: ReferenceRow = {
      reference_id: this.newId(),
      content_id: contentId,
      jurisdiction: this.partition.jurisdiction,
      sensitivity_tier: this.partition.tier,
      class: input.refClass,
      created_by: createdBy,
      created_at: now,
      tombstoned_at: null,
    };
    this.sql.exec(
      `INSERT INTO content_reference
         (reference_id, content_id, jurisdiction, sensitivity_tier, class, created_by, created_at, tombstoned_at)
       VALUES (?, ?, ?, ?, ?, ?, ?, NULL)`,
      row.reference_id,
      row.content_id,
      row.jurisdiction,
      row.sensitivity_tier,
      row.class,
      row.created_by,
      row.created_at,
    );
    this.appendEvent("reference.created", row, createdBy, now);
    return { row, created: true };
  }

  /** Remove a reference: tombstone it and emit the outbox event the
   * index and GC consume. Revocation is reference removal (design
   * section 5): after this returns, the row grants nothing.
   * Removing an already-tombstoned reference is a no-op returning
   * false; removing an unknown id fails closed. */
  removeReference(referenceId: string, opts: { actor: string; now?: string }): boolean {
    const actor = parseBindingId(opts.actor);
    const found = this.sql
      .exec("SELECT * FROM content_reference WHERE reference_id = ?", referenceId)
      .toArray()[0];
    if (!found) {
      throw new FailClosed(404, "reference_not_found", `no reference ${referenceId} in this database`);
    }
    const row = toReferenceRow(found);
    if (row.tombstoned_at !== null) return false;
    const now = opts.now ?? new Date().toISOString();
    this.sql.exec(
      "UPDATE content_reference SET tombstoned_at = ? WHERE reference_id = ?",
      now,
      referenceId,
    );
    this.appendEvent("reference.removed", { ...row, tombstoned_at: now }, actor, now);
    return true;
  }

  /** All references (live and tombstoned) to a content ID in this
   * database, oldest first. The access check filters further; the
   * GC sweep uses the full set. */
  referencesFor(contentId: string): ReferenceRow[] {
    return this.sql
      .exec("SELECT * FROM content_reference WHERE content_id = ? ORDER BY created_at, reference_id", parseContentId(contentId))
      .toArray()
      .map(toReferenceRow);
  }

  /** Every live (non-tombstoned) reference in this database, in
   * content then reference order. This is the authoritative side of
   * the derived index's reconciliation sweep (#12805); a full scan,
   * meant for periodic partition sweeps, never the request path. */
  liveReferences(): ReferenceRow[] {
    return this.sql
      .exec("SELECT * FROM content_reference WHERE tombstoned_at IS NULL ORDER BY content_id, reference_id")
      .toArray()
      .map(toReferenceRow);
  }

  /** Outbox events after a consumer cursor (`seq`), oldest first.
   * Delivery is at-least-once; consumers dedupe by event_id. */
  outboxAfter(afterSeq: number, limit = 500): ReferenceEvent[] {
    return this.sql
      .exec("SELECT * FROM reference_outbox WHERE seq > ? ORDER BY seq LIMIT ?", afterSeq, limit)
      .toArray()
      .map(toReferenceEvent);
  }

  private appendEvent(type: ReferenceEventType, row: ReferenceRow, actor: string, now: string): void {
    this.sql.exec(
      `INSERT INTO reference_outbox
         (event_id, event_type, reference_id, content_id, jurisdiction, sensitivity_tier, class, actor, occurred_at)
       VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)`,
      this.newId(),
      type,
      row.reference_id,
      row.content_id,
      row.jurisdiction,
      row.sensitivity_tier,
      row.class,
      actor,
      now,
    );
  }
}

export interface AccessRequest {
  contentId: string;
  /** The partition of the OBJECT being read. A reference grants only
   * inside its own partition; a mismatch is a refusal, never a
   * fallback (design section 5). */
  partition: Partition;
  /** Databases the caller is authorized to read (component E check,
   * resolved by the caller of this function). */
  authorizedDbs: ReferenceStore[];
  /** The public database's store, when the deployment has one.
   * Consulted only for tier `public`, only for publication
   * references. */
  publicDb?: ReferenceStore;
}

export interface AccessDecision {
  allowed: boolean;
  via: "reference" | "publication" | null;
  /** The granting reference, when allowed. */
  referenceId: string | null;
  /** The database holding the granting reference, when allowed. */
  dbId: string | null;
  reason: string;
}

function classPermits(refClass: ReferenceClass, partition: Partition): boolean {
  // A publication reference grants only through rule 2, in the
  // public DB -- never as an ordinary in-DB reference.
  if (refClass === "publication") return false;
  // Tenant-restricted content: capability minting additionally
  // requires the restricted class on the reference record.
  if (partition.tier === "tenant-restricted") return refClass === "restricted";
  return refClass === "standard" || refClass === "restricted";
}

function samePartition(row: ReferenceRow, partition: Partition): boolean {
  return row.jurisdiction === partition.jurisdiction && row.sensitivity_tier === partition.tier;
}

/** The design section 5 access check. Pure over the given stores'
 * current rows: it reads authoritative reference state and grants
 * nothing that is not backed by a live row. */
export function checkAccess(req: AccessRequest): AccessDecision {
  const contentId = parseContentId(req.contentId);
  for (const store of req.authorizedDbs) {
    for (const row of store.referencesFor(contentId)) {
      if (row.tombstoned_at !== null) continue;
      if (!samePartition(row, req.partition)) continue;
      if (!classPermits(row.class, req.partition)) continue;
      return {
        allowed: true,
        via: "reference",
        referenceId: row.reference_id,
        dbId: store.dbId,
        reason: `live ${row.class} reference in authorized database ${store.dbId}`,
      };
    }
  }
  if (req.partition.tier === "public" && req.publicDb) {
    for (const row of req.publicDb.referencesFor(contentId)) {
      if (row.tombstoned_at !== null) continue;
      if (!samePartition(row, req.partition)) continue;
      if (row.class !== "publication") continue;
      return {
        allowed: true,
        via: "publication",
        referenceId: row.reference_id,
        dbId: req.publicDb.dbId,
        reason: "live publication reference in the public database",
      };
    }
  }
  return {
    allowed: false,
    via: null,
    referenceId: null,
    dbId: null,
    reason: "no live reference in an authorized database and no live publication reference",
  };
}
