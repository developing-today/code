// Derived per-partition reference/location index for the
// content-addressed media/data plane (component F of #9078, issue
// #12805, slice 3 of #9084; design record
// doc/2026-11-23T03-04-00Z_design_content-addressed-media-data-plane,
// section 6 "Home of reference tracking" and the drift risk).
//
// What it is: for one partition (jurisdiction, sensitivity tier), a
// map content ID -> the set of databases holding a live reference to
// it, plus the replica locations the object plane has published it
// at. It is built only from the reference-change outbox each
// database's ReferenceStore emits (#12803) and from object-plane
// publish/delete notices (#12804). It exists so the GC sweep (#12806)
// can find candidates without scanning every database.
//
// What it is not: evidence for a delete. The index is a cache. The
// sweep's delete decision re-reads authoritative reference rows
// (design section 6, step 3), so index drift can delay collection but
// never cause a wrongful delete. Nothing in this module deletes
// content.
//
// Alignment with the #9115 location index (id-store
// location_index.rs): the index is a rebuildable projection whose
// whole state is the fold of its input events; `rebuild` from the
// same outboxes yields exactly the incrementally folded index; each
// source keeps a reconciliation cursor; locations are typed locators
// in their canonical tagged text form (`r2/…`, `laptop/…`,
// `drive/…`); and a location leaves the fresh state only through an
// explicit event (a GC delete audit), never by omission.
//
// Delivery from the outbox is at-least-once. Every applied event
// records the source sequence number on its entry, and an event whose
// sequence is not newer than the entry's is ignored, so a replayed
// prefix converges to the same state.
//
// The reconciliation job compares the index with the authoritative
// live rows of each database in the partition, reports the drift
// (missing, stale and mismatched entries, and a ratio), records the
// report so drift can be measured over time before grace defaults are
// tightened (design risks), and optionally repairs the index.

import { FailClosed } from "./routing.ts";
import {
  parseContentId,
  type Partition,
  type ReferenceClass,
  type ReferenceEvent,
  type ReferenceRow,
  type ReferenceStore,
} from "./references.ts";
import type { DeleteAuditEvent } from "./object-plane.ts";

interface SqlStorage {
  exec(query: string, ...params: unknown[]): { toArray(): Record<string, unknown>[] };
}

/** The index's own schema. It lives with the index (a partition-level
 *  store), never inside the tenant or public databases it reads. */
export const REFERENCE_INDEX_SCHEMA_STATEMENTS: readonly string[] = [
  `CREATE TABLE IF NOT EXISTS reference_index_entry (
  db_id        TEXT NOT NULL,
  reference_id TEXT NOT NULL,
  content_id   TEXT NOT NULL,
  class        TEXT NOT NULL,
  live         INTEGER NOT NULL,
  source_seq   INTEGER NOT NULL,
  updated_at   TEXT NOT NULL,
  PRIMARY KEY (db_id, reference_id),
  CHECK (live IN (0, 1)),
  CHECK (class IN ('standard','restricted','publication'))
)`,
  `CREATE INDEX IF NOT EXISTS reference_index_entry_by_content
  ON reference_index_entry (content_id, live)`,
  `CREATE TABLE IF NOT EXISTS reference_index_cursor (
  db_id      TEXT PRIMARY KEY,
  last_seq   INTEGER NOT NULL,
  updated_at TEXT NOT NULL
)`,
  `CREATE TABLE IF NOT EXISTS reference_index_location (
  content_id TEXT NOT NULL,
  locator    TEXT NOT NULL,
  state      TEXT NOT NULL,
  provenance TEXT NOT NULL,
  updated_at TEXT NOT NULL,
  PRIMARY KEY (content_id, locator),
  CHECK (state IN ('fresh','deleted'))
)`,
  `CREATE TABLE IF NOT EXISTS reference_index_reconciliation (
  run_id          INTEGER PRIMARY KEY AUTOINCREMENT,
  reconciled_at   TEXT NOT NULL,
  dbs_scanned     INTEGER NOT NULL,
  authoritative   INTEGER NOT NULL,
  indexed         INTEGER NOT NULL,
  missing         INTEGER NOT NULL,
  stale           INTEGER NOT NULL,
  mismatched      INTEGER NOT NULL,
  drift_ratio     REAL NOT NULL,
  repaired        INTEGER NOT NULL
)`,
];

export type LocationState = "fresh" | "deleted";

export interface IndexedReference {
  dbId: string;
  referenceId: string;
  class: ReferenceClass;
}

export interface IndexedLocation {
  locator: string;
  state: LocationState;
  /** What last set the state: a publish notice or a GC delete audit. */
  provenance: string;
  updatedAt: string;
}

export interface IndexLookup {
  contentId: string;
  partition: Partition;
  /** Databases holding at least one live reference, sorted. */
  referencingDbs: string[];
  references: IndexedReference[];
  locations: IndexedLocation[];
}

export interface ConsumeResult {
  dbId: string;
  /** Events read from the outbox in this call. */
  read: number;
  /** Events that changed the index. */
  applied: number;
  /** Events ignored because the entry already reflects a newer or
   *  equal source sequence (replay after an interrupted batch). */
  replayed: number;
  /** The source cursor after this call. */
  cursor: number;
}

export interface DriftEntry {
  dbId: string;
  referenceId: string;
  contentId: string;
  /** For `mismatched`: what the index holds instead. */
  indexed?: { contentId: string; class: ReferenceClass };
  class: ReferenceClass;
}

export interface DriftReport {
  partition: Partition;
  reconciledAt: string;
  dbsScanned: string[];
  /** Databases handed to the job whose partition is not this index's.
   *  They are not read; listing them keeps a misrouted sweep visible. */
  dbsSkipped: string[];
  /** Live references in the authoritative rows. */
  authoritative: number;
  /** Live entries in the index for the scanned databases. */
  indexed: number;
  /** Live authoritatively, absent or not live in the index. */
  missing: DriftEntry[];
  /** Live in the index, not live authoritatively. */
  stale: DriftEntry[];
  /** Live in both, but content or class differ. */
  mismatched: DriftEntry[];
  /** (missing + stale + mismatched) / the number of distinct live
   *  references on either side; 0 when both sides are empty. */
  driftRatio: number;
  /** Content IDs whose referencing set the drift touches, sorted. */
  affectedContent: string[];
  repaired: boolean;
}

export interface DriftHistoryRow {
  runId: number;
  reconciledAt: string;
  dbsScanned: number;
  authoritative: number;
  indexed: number;
  missing: number;
  stale: number;
  mismatched: number;
  driftRatio: number;
  repaired: boolean;
}

const LOCATOR_TAGS = ["r2", "laptop", "drive"] as const;

/** Validate a typed locator in the #9115 canonical tagged text form
 *  (`<kind>/<fields…>`, id-core location_ref.rs). Only the shape is
 *  checked here; the kind tag is required and decisive. */
export function parseLocator(raw: string): string {
  const slash = raw.indexOf("/");
  const tag = slash > 0 ? raw.slice(0, slash) : "";
  const rest = slash > 0 ? raw.slice(slash + 1) : "";
  if (!(LOCATOR_TAGS as readonly string[]).includes(tag) || rest.length === 0 || /[\s\u0000-\u001f\u007f]/.test(raw)) {
    throw new FailClosed(400, "invalid_locator", "locator must be a tagged r2/, laptop/ or drive/ location");
  }
  return raw;
}

function samePartition(a: Partition, b: Partition): boolean {
  return a.jurisdiction === b.jurisdiction && a.tier === b.tier;
}

function key(dbId: string, referenceId: string): string {
  return `${dbId}\u0000${referenceId}`;
}

/** The derived reference/location index of one partition. */
export class ReferenceIndex {
  private sql: SqlStorage;
  readonly partition: Partition;

  constructor(sql: SqlStorage, opts: { partition: Partition }) {
    this.sql = sql;
    this.partition = opts.partition;
  }

  ensureSchema(): void {
    for (const stmt of REFERENCE_INDEX_SCHEMA_STATEMENTS) this.sql.exec(stmt);
  }

  private requirePartition(store: ReferenceStore): void {
    if (!samePartition(store.partition, this.partition)) {
      throw new FailClosed(
        409,
        "index_partition_mismatch",
        `database ${store.dbId} is in partition ${store.partition.jurisdiction}/${store.partition.tier}, ` +
          `this index is ${this.partition.jurisdiction}/${this.partition.tier}`,
      );
    }
  }

  /** The consumer cursor for a source database (0 before any event). */
  cursor(dbId: string): number {
    const row = this.sql.exec("SELECT last_seq FROM reference_index_cursor WHERE db_id = ?", dbId).toArray()[0];
    return row ? Number(row.last_seq) : 0;
  }

  private setCursor(dbId: string, seq: number, now: string): void {
    this.sql.exec(
      `INSERT INTO reference_index_cursor (db_id, last_seq, updated_at) VALUES (?, ?, ?)
       ON CONFLICT(db_id) DO UPDATE SET last_seq = excluded.last_seq, updated_at = excluded.updated_at`,
      dbId,
      seq,
      now,
    );
  }

  /** Fold one outbox event. Returns true when the index changed. */
  private applyEvent(dbId: string, ev: ReferenceEvent, now: string): boolean {
    if (ev.jurisdiction !== this.partition.jurisdiction || ev.sensitivity_tier !== this.partition.tier) {
      // A store is bound to its partition, so this means a corrupt or
      // misrouted outbox. Refuse rather than index it somewhere else.
      throw new FailClosed(
        409,
        "index_event_partition_mismatch",
        `event ${ev.event_id} from ${dbId} is not in this index's partition`,
      );
    }
    const existing = this.sql
      .exec("SELECT source_seq FROM reference_index_entry WHERE db_id = ? AND reference_id = ?", dbId, ev.reference_id)
      .toArray()[0];
    if (existing && Number(existing.source_seq) >= ev.seq) return false;
    const live = ev.event_type === "reference.created" ? 1 : 0;
    this.sql.exec(
      `INSERT INTO reference_index_entry (db_id, reference_id, content_id, class, live, source_seq, updated_at)
       VALUES (?, ?, ?, ?, ?, ?, ?)
       ON CONFLICT(db_id, reference_id) DO UPDATE SET
         content_id = excluded.content_id, class = excluded.class, live = excluded.live,
         source_seq = excluded.source_seq, updated_at = excluded.updated_at`,
      dbId,
      ev.reference_id,
      ev.content_id,
      ev.class,
      live,
      ev.seq,
      now,
    );
    return true;
  }

  /** Consume a database's outbox from this index's cursor to its end,
   *  in batches. Safe to call repeatedly and after a crash: replayed
   *  events are recognised by their source sequence. */
  consume(store: ReferenceStore, opts: { batch?: number; now?: string } = {}): ConsumeResult {
    this.requirePartition(store);
    const batch = opts.batch ?? 500;
    if (!Number.isInteger(batch) || batch < 1) {
      throw new FailClosed(400, "invalid_batch", "batch must be a positive integer");
    }
    const now = opts.now ?? new Date().toISOString();
    let cursor = this.cursor(store.dbId);
    let read = 0;
    let applied = 0;
    let replayed = 0;
    for (;;) {
      const events = store.outboxAfter(cursor, batch);
      for (const ev of events) {
        read++;
        if (this.applyEvent(store.dbId, ev, now)) applied++;
        else replayed++;
        cursor = ev.seq;
        this.setCursor(store.dbId, cursor, now);
      }
      if (events.length < batch) break;
    }
    return { dbId: store.dbId, read, applied, replayed, cursor };
  }

  /** Drop everything the index holds for these databases and refold
   *  their whole outboxes. The result equals incremental folding of
   *  the same events (the #9115 rebuildable-projection property). */
  rebuild(stores: ReferenceStore[], opts: { batch?: number; now?: string } = {}): ConsumeResult[] {
    for (const store of stores) this.requirePartition(store);
    for (const store of stores) {
      this.sql.exec("DELETE FROM reference_index_entry WHERE db_id = ?", store.dbId);
      this.sql.exec("DELETE FROM reference_index_cursor WHERE db_id = ?", store.dbId);
    }
    return stores.map((store) => this.consume(store, opts));
  }

  /** Record that the object plane published a verified replica of the
   *  content at this location. */
  notePublished(contentId: string, locator: string, opts: { now?: string; provenance?: string } = {}): void {
    const id = parseContentId(contentId);
    const loc = parseLocator(locator);
    const now = opts.now ?? new Date().toISOString();
    this.sql.exec(
      `INSERT INTO reference_index_location (content_id, locator, state, provenance, updated_at)
       VALUES (?, ?, 'fresh', ?, ?)
       ON CONFLICT(content_id, locator) DO UPDATE SET
         state = 'fresh', provenance = excluded.provenance, updated_at = excluded.updated_at`,
      id,
      loc,
      opts.provenance ?? "published",
      now,
    );
  }

  /** Record a GC replica delete from the adapter's delete audit event.
   *  The audit must be for this index's partition. */
  noteDeleted(audit: DeleteAuditEvent, locator: string): void {
    const id = parseContentId(audit.contentId);
    const loc = parseLocator(locator);
    if (!samePartition(audit.partition, this.partition)) {
      throw new FailClosed(409, "index_audit_partition_mismatch", "delete audit is not in this index's partition");
    }
    this.sql.exec(
      `INSERT INTO reference_index_location (content_id, locator, state, provenance, updated_at)
       VALUES (?, ?, 'deleted', ?, ?)
       ON CONFLICT(content_id, locator) DO UPDATE SET
         state = 'deleted', provenance = excluded.provenance, updated_at = excluded.updated_at`,
      id,
      loc,
      `gc epoch ${audit.epoch}, reverified ${audit.reverifiedAt}`,
      audit.deletedAt,
    );
  }

  lookup(contentId: string): IndexLookup {
    const id = parseContentId(contentId);
    const references = this.sql
      .exec(
        `SELECT db_id, reference_id, class FROM reference_index_entry
          WHERE content_id = ? AND live = 1 ORDER BY db_id, reference_id`,
        id,
      )
      .toArray()
      .map((r) => ({
        dbId: String(r.db_id),
        referenceId: String(r.reference_id),
        class: String(r.class) as ReferenceClass,
      }));
    const locations = this.sql
      .exec(
        "SELECT locator, state, provenance, updated_at FROM reference_index_location WHERE content_id = ? ORDER BY locator",
        id,
      )
      .toArray()
      .map((r) => ({
        locator: String(r.locator),
        state: String(r.state) as LocationState,
        provenance: String(r.provenance),
        updatedAt: String(r.updated_at),
      }));
    const referencingDbs = [...new Set(references.map((r) => r.dbId))].sort();
    return { contentId: id, partition: this.partition, referencingDbs, references, locations };
  }

  /** Sweep candidates as the index sees them: content with a fresh
   *  location or a once-indexed reference, and no live reference now.
   *  Candidates only. The sweep must re-verify each against
   *  authoritative rows before any delete (design section 6). */
  unreferencedContent(): string[] {
    return this.sql
      .exec(
        `SELECT content_id FROM (
           SELECT content_id FROM reference_index_entry
           UNION SELECT content_id FROM reference_index_location WHERE state = 'fresh'
         )
         WHERE content_id NOT IN (SELECT content_id FROM reference_index_entry WHERE live = 1)
         ORDER BY content_id`,
      )
      .toArray()
      .map((r) => String(r.content_id));
  }

  /** The reconciliation job: compare the index with the authoritative
   *  live rows of each given database in this partition, report and
   *  record the drift, and with `repair` make the index match. */
  reconcile(stores: ReferenceStore[], opts: { repair?: boolean; now?: string } = {}): DriftReport {
    const now = opts.now ?? new Date().toISOString();
    const scanned = stores.filter((s) => samePartition(s.partition, this.partition));
    const skipped = stores.filter((s) => !samePartition(s.partition, this.partition)).map((s) => s.dbId);

    const auth = new Map<string, { dbId: string; row: ReferenceRow }>();
    for (const store of scanned) {
      for (const row of store.liveReferences()) auth.set(key(store.dbId, row.reference_id), { dbId: store.dbId, row });
    }
    const indexed = new Map<string, { dbId: string; referenceId: string; contentId: string; class: ReferenceClass }>();
    for (const store of scanned) {
      for (const r of this.sql
        .exec(
          "SELECT reference_id, content_id, class FROM reference_index_entry WHERE db_id = ? AND live = 1",
          store.dbId,
        )
        .toArray()) {
        const referenceId = String(r.reference_id);
        indexed.set(key(store.dbId, referenceId), {
          dbId: store.dbId,
          referenceId,
          contentId: String(r.content_id),
          class: String(r.class) as ReferenceClass,
        });
      }
    }

    const missing: DriftEntry[] = [];
    const stale: DriftEntry[] = [];
    const mismatched: DriftEntry[] = [];
    for (const [k, { dbId, row }] of auth) {
      const ix = indexed.get(k);
      const entry: DriftEntry = { dbId, referenceId: row.reference_id, contentId: row.content_id, class: row.class };
      if (!ix) missing.push(entry);
      else if (ix.contentId !== row.content_id || ix.class !== row.class) {
        mismatched.push({ ...entry, indexed: { contentId: ix.contentId, class: ix.class } });
      }
    }
    for (const [k, ix] of indexed) {
      if (!auth.has(k))
        stale.push({ dbId: ix.dbId, referenceId: ix.referenceId, contentId: ix.contentId, class: ix.class });
    }
    const byKey = (a: DriftEntry, b: DriftEntry) =>
      a.dbId === b.dbId ? (a.referenceId < b.referenceId ? -1 : 1) : a.dbId < b.dbId ? -1 : 1;
    missing.sort(byKey);
    stale.sort(byKey);
    mismatched.sort(byKey);

    const union = new Set([...auth.keys(), ...indexed.keys()]).size;
    const drifted = missing.length + stale.length + mismatched.length;
    const driftRatio = union === 0 ? 0 : drifted / union;
    const affected = new Set<string>();
    for (const e of [...missing, ...stale, ...mismatched]) {
      affected.add(e.contentId);
      if (e.indexed) affected.add(e.indexed.contentId);
    }

    const repair = opts.repair === true;
    if (repair) {
      // Repaired entries take the source's current cursor as their
      // sequence, so any outbox event not yet consumed (sequence above
      // the cursor) still applies on top of the repair.
      for (const e of [...missing, ...mismatched]) {
        this.sql.exec(
          `INSERT INTO reference_index_entry (db_id, reference_id, content_id, class, live, source_seq, updated_at)
           VALUES (?, ?, ?, ?, 1, ?, ?)
           ON CONFLICT(db_id, reference_id) DO UPDATE SET
             content_id = excluded.content_id, class = excluded.class, live = 1,
             source_seq = excluded.source_seq, updated_at = excluded.updated_at`,
          e.dbId,
          e.referenceId,
          e.contentId,
          e.class,
          this.cursor(e.dbId),
          now,
        );
      }
      for (const e of stale) {
        this.sql.exec(
          "UPDATE reference_index_entry SET live = 0, source_seq = ?, updated_at = ? WHERE db_id = ? AND reference_id = ?",
          this.cursor(e.dbId),
          now,
          e.dbId,
          e.referenceId,
        );
      }
    }

    this.sql.exec(
      `INSERT INTO reference_index_reconciliation
         (reconciled_at, dbs_scanned, authoritative, indexed, missing, stale, mismatched, drift_ratio, repaired)
       VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)`,
      now,
      scanned.length,
      auth.size,
      indexed.size,
      missing.length,
      stale.length,
      mismatched.length,
      driftRatio,
      repair ? 1 : 0,
    );

    return {
      partition: this.partition,
      reconciledAt: now,
      dbsScanned: scanned.map((s) => s.dbId),
      dbsSkipped: skipped,
      authoritative: auth.size,
      indexed: indexed.size,
      missing,
      stale,
      mismatched,
      driftRatio,
      affectedContent: [...affected].sort(),
      repaired: repair,
    };
  }

  /** Recorded reconciliation runs, oldest first: the drift
   *  measurement the design asks for before grace defaults tighten. */
  driftHistory(): DriftHistoryRow[] {
    return this.sql
      .exec("SELECT * FROM reference_index_reconciliation ORDER BY run_id")
      .toArray()
      .map((r) => ({
        runId: Number(r.run_id),
        reconciledAt: String(r.reconciled_at),
        dbsScanned: Number(r.dbs_scanned),
        authoritative: Number(r.authoritative),
        indexed: Number(r.indexed),
        missing: Number(r.missing),
        stale: Number(r.stale),
        mismatched: Number(r.mismatched),
        driftRatio: Number(r.drift_ratio),
        repaired: Number(r.repaired) === 1,
      }));
  }
}
