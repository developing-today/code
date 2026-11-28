// GC mark-and-sweep worker for the content-addressed media/data plane
// (component F of #9078, issue #12806, slice 4 of #9084; design record
// doc/2026-11-23T03-04-00Z_design_content-addressed-media-data-plane,
// section 6 "GC design").
//
// One sweeper serves one partition (jurisdiction, sensitivity tier).
// It reads candidates from the derived reference/location index
// (#12805), decides every delete against authoritative reference rows
// (the ReferenceStores of #12803), and removes replicas only through
// the object-plane adapter's fenced, idempotent delete (#12804).
//
// The protocol, per partition and per epoch:
//
//   1. Mark. `mark` opens epoch E and moves content the index sees as
//      unreferenced into `candidate` state, stamped with E and a grace
//      deadline. Content the index sees as referenced again leaves
//      `candidate` state: referencing is idempotent and never needs to
//      know GC exists.
//   2. Grace. Candidates wait the policy grace (default 7 days for the
//      tenant tiers, 24 hours for `public`). A new reference at any
//      time clears the candidate (`noteReferenced`, or the next mark).
//   3. Re-verify. `sweep` re-reads the authoritative rows of every
//      database handed to it for each candidate past grace. Any live
//      reference rescues it. The index being stale can delay a
//      collection; it can never cause one.
//   4. Fenced delete. A cleared candidate is fenced first: its state
//      becomes `fenced` with a number from the partition's sequence,
//      synchronously, before the adapter is called. Reference creation
//      for content that is a candidate past grace passes through the
//      same sequence (`admitReference`): a create admitted before the
//      fence clears the candidate (re-verify would have seen it
//      anyway), and one that arrives after it is told to re-stage the
//      bytes, which the writer holds by definition. A re-upload, never
//      data loss.
//   5. Audit. Every delete records an audit row: content, partition,
//      epoch, the re-verify evidence, and whether a replica existed.
//
// Reference counters (#9137) may order candidates but are never
// delete evidence, so this module takes none. Sole-referencer erasure
// in `tenant-restricted` (design section 6, "Deletion and privacy
// semantics") may shorten grace to zero by policy, under the same
// fence; erasure of shared bytes is not possible, and the honest
// outcome (reference removal plus a recorded erasure note) is what
// `requestErasure` returns in that case.
//
// Partition isolation (#3401): the sweeper refuses any index, adapter
// or reference store from another partition, so a sweep can neither
// see nor delete another partition's replicas.

import { FailClosed, parseBindingId } from "./routing.ts";
import { parseContentId, type Partition, type ReferenceStore, type SensitivityTier } from "./references.ts";
import type { DeleteAuditEvent, ObjectPlaneAdapter } from "./object-plane.ts";
import type { ReferenceIndex } from "./reference-index.ts";

interface SqlStorage {
  exec(query: string, ...params: unknown[]): { toArray(): Record<string, unknown>[] };
}

const HOUR_MS = 60 * 60 * 1000;
const DAY_MS = 24 * HOUR_MS;

/** Starting grace defaults (design section 6, step 2). Policy-owned by
 *  component E; these are the record's starting values. */
export const DEFAULT_GRACE_MS: Readonly<Record<Exclude<SensitivityTier, "staging">, number>> = {
  public: DAY_MS,
  "tenant-private": 7 * DAY_MS,
  "tenant-restricted": 7 * DAY_MS,
};

export interface GcPolicy {
  /** Grace period for candidates in this partition, in milliseconds. */
  graceMs: number;
  /** Whether a sole-referencer erasure request in `tenant-restricted`
   *  may shorten grace to zero (design section 6). Off unless the
   *  policy turns it on. */
  soleReferencerErasure: boolean;
}

export type CandidateState = "candidate" | "live" | "fenced" | "deleted";

export interface CandidateRow {
  contentId: string;
  state: CandidateState;
  epoch: string;
  markedAt: string;
  graceUntil: string;
  /** Partition sequence number of the fence, once fenced. */
  fenceSeq: number | null;
  updatedAt: string;
}

export interface GcAuditRow {
  auditId: number;
  contentId: string;
  partition: Partition;
  epoch: string;
  reverifiedAt: string;
  deletedAt: string;
  existed: boolean;
  fenceSeq: number;
  reason: "grace" | "erasure";
  /** Databases whose authoritative rows the re-verify read. */
  dbsReverified: string[];
}

export interface ErasureNote {
  contentId: string;
  requester: string;
  notedAt: string;
  /** Live references held by others when the request was made. */
  otherLiveReferences: number;
}

export interface MarkResult {
  epoch: string;
  /** Content newly put in candidate state this epoch. */
  marked: string[];
  /** Candidates the index now shows as referenced again. */
  cleared: string[];
}

export interface SweepResult {
  /** Candidates past grace that re-verify found referenced. */
  rescued: string[];
  /** Candidates deleted under the fence. */
  deleted: string[];
  /** Candidates still inside their grace period. */
  waiting: string[];
}

export interface AdmitResult {
  /** `proceed`: create the reference as usual. `restage`: the bytes
   *  were fenced for delete before this reference arrived; the writer
   *  re-stages and publishes them again before referencing. */
  action: "proceed" | "restage";
  /** Partition sequence number of this admission. */
  seq: number;
}

export interface ErasureResult {
  erased: boolean;
  /** `deleted`: sole referencer, deleted at zero grace under the fence.
   *  `shared`: others still reference the bytes; an erasure note was
   *  recorded and the bytes stay. */
  outcome: "deleted" | "shared";
  audit: GcAuditRow | null;
}

/** The sweeper's own schema. It lives in a per-partition GC store,
 *  never inside the tenant or public databases it reads. */
export const GC_SCHEMA_STATEMENTS: readonly string[] = [
  `CREATE TABLE IF NOT EXISTS gc_epoch (
  epoch      TEXT PRIMARY KEY,
  ordinal    INTEGER NOT NULL UNIQUE,
  opened_at  TEXT NOT NULL,
  marked     INTEGER NOT NULL
)`,
  `CREATE TABLE IF NOT EXISTS gc_candidate (
  content_id  TEXT PRIMARY KEY,
  state       TEXT NOT NULL,
  epoch       TEXT NOT NULL,
  marked_at   TEXT NOT NULL,
  grace_until TEXT NOT NULL,
  fence_seq   INTEGER,
  updated_at  TEXT NOT NULL,
  CHECK (state IN ('candidate','live','fenced','deleted'))
)`,
  `CREATE TABLE IF NOT EXISTS gc_sequence (
  id  INTEGER PRIMARY KEY,
  seq INTEGER NOT NULL,
  CHECK (id = 1)
)`,
  `CREATE TABLE IF NOT EXISTS gc_audit (
  audit_id       INTEGER PRIMARY KEY AUTOINCREMENT,
  content_id     TEXT NOT NULL,
  jurisdiction   TEXT NOT NULL,
  sensitivity_tier TEXT NOT NULL,
  epoch          TEXT NOT NULL,
  reverified_at  TEXT NOT NULL,
  deleted_at     TEXT NOT NULL,
  existed        INTEGER NOT NULL,
  fence_seq      INTEGER NOT NULL,
  reason         TEXT NOT NULL,
  dbs_reverified TEXT NOT NULL,
  CHECK (existed IN (0, 1)),
  CHECK (reason IN ('grace','erasure'))
)`,
  `CREATE TABLE IF NOT EXISTS gc_erasure_note (
  note_id         INTEGER PRIMARY KEY AUTOINCREMENT,
  content_id      TEXT NOT NULL,
  requester       TEXT NOT NULL,
  noted_at        TEXT NOT NULL,
  other_live_refs INTEGER NOT NULL
)`,
];

function samePartition(a: Partition, b: Partition): boolean {
  return a.jurisdiction === b.jurisdiction && a.tier === b.tier;
}

function parseTime(raw: string, what: string): number {
  const ms = Date.parse(raw);
  if (Number.isNaN(ms)) throw new FailClosed(400, "gc_invalid_time", `${what} is not a parseable timestamp`);
  return ms;
}

function toCandidate(r: Record<string, unknown>): CandidateRow {
  return {
    contentId: String(r.content_id),
    state: String(r.state) as CandidateState,
    epoch: String(r.epoch),
    markedAt: String(r.marked_at),
    graceUntil: String(r.grace_until),
    fenceSeq: r.fence_seq === null || r.fence_seq === undefined ? null : Number(r.fence_seq),
    updatedAt: String(r.updated_at),
  };
}

function toAudit(r: Record<string, unknown>): GcAuditRow {
  return {
    auditId: Number(r.audit_id),
    contentId: String(r.content_id),
    partition: {
      jurisdiction: String(r.jurisdiction) as Partition["jurisdiction"],
      tier: String(r.sensitivity_tier) as SensitivityTier,
    },
    epoch: String(r.epoch),
    reverifiedAt: String(r.reverified_at),
    deletedAt: String(r.deleted_at),
    existed: Number(r.existed) === 1,
    fenceSeq: Number(r.fence_seq),
    reason: String(r.reason) as "grace" | "erasure",
    dbsReverified: JSON.parse(String(r.dbs_reverified)) as string[],
  };
}

/** The deletion side of the object-plane adapter: all the sweeper
 *  needs, so a test can hand it a recording fake. */
export type GcObjectPlane = Pick<ObjectPlaneAdapter, "partition" | "deleteReplica">;

export interface GcSweeperOpts {
  partition: Partition;
  index: ReferenceIndex;
  adapter: GcObjectPlane;
  /** The adapter's locator for replicas in this partition, in the
   *  index's tagged text form (`r2/…`). Deletes are noted on the index
   *  at this location. */
  locator: string;
  policy?: Partial<GcPolicy>;
}

export class GcSweeper {
  private sql: SqlStorage;
  readonly partition: Partition;
  readonly policy: GcPolicy;
  private index: ReferenceIndex;
  private adapter: GcObjectPlane;
  private locator: string;

  constructor(sql: SqlStorage, opts: GcSweeperOpts) {
    if (opts.partition.tier === "staging") {
      // Object-store lifecycle reaps staging; GC never runs there.
      throw new FailClosed(409, "gc_staging_partition", "staging is reaped by lifecycle rules, not by the GC sweep");
    }
    if (!samePartition(opts.index.partition, opts.partition)) {
      throw new FailClosed(
        409,
        "gc_index_partition_mismatch",
        "the reference index is not in this sweeper's partition",
      );
    }
    if (!samePartition(opts.adapter.partition, opts.partition)) {
      throw new FailClosed(
        409,
        "gc_adapter_partition_mismatch",
        "the object-plane adapter is not in this sweeper's partition",
      );
    }
    const graceMs = opts.policy?.graceMs ?? DEFAULT_GRACE_MS[opts.partition.tier];
    if (!Number.isFinite(graceMs) || graceMs < 0) {
      throw new FailClosed(400, "gc_invalid_grace", "grace must be a non-negative number of milliseconds");
    }
    this.sql = sql;
    this.partition = opts.partition;
    this.index = opts.index;
    this.adapter = opts.adapter;
    this.locator = opts.locator;
    this.policy = { graceMs, soleReferencerErasure: opts.policy?.soleReferencerErasure ?? false };
  }

  ensureSchema(): void {
    for (const stmt of GC_SCHEMA_STATEMENTS) this.sql.exec(stmt);
    this.sql.exec("INSERT OR IGNORE INTO gc_sequence (id, seq) VALUES (1, 0)");
  }

  private requireStores(stores: ReferenceStore[]): void {
    if (stores.length === 0) {
      throw new FailClosed(400, "gc_no_authoritative_dbs", "re-verify needs the partition's authoritative databases");
    }
    for (const store of stores) {
      if (!samePartition(store.partition, this.partition)) {
        throw new FailClosed(
          409,
          "gc_store_partition_mismatch",
          `database ${store.dbId} is not in this sweeper's partition`,
        );
      }
    }
  }

  /** Take the next number from the partition's serialization point. */
  private nextSeq(): number {
    this.sql.exec("UPDATE gc_sequence SET seq = seq + 1 WHERE id = 1");
    const row = this.sql.exec("SELECT seq FROM gc_sequence WHERE id = 1").toArray()[0];
    if (!row) throw new FailClosed(500, "gc_schema_missing", "gc_sequence is not initialized; call ensureSchema");
    return Number(row.seq);
  }

  candidate(contentId: string): CandidateRow | null {
    const row = this.sql
      .exec("SELECT * FROM gc_candidate WHERE content_id = ?", parseContentId(contentId))
      .toArray()[0];
    return row ? toCandidate(row) : null;
  }

  candidates(): CandidateRow[] {
    return this.sql.exec("SELECT * FROM gc_candidate ORDER BY content_id").toArray().map(toCandidate);
  }

  auditLog(): GcAuditRow[] {
    return this.sql.exec("SELECT * FROM gc_audit ORDER BY audit_id").toArray().map(toAudit);
  }

  erasureNotes(): ErasureNote[] {
    return this.sql
      .exec("SELECT * FROM gc_erasure_note ORDER BY note_id")
      .toArray()
      .map((r) => ({
        contentId: String(r.content_id),
        requester: String(r.requester),
        notedAt: String(r.noted_at),
        otherLiveReferences: Number(r.other_live_refs),
      }));
  }

  private openEpoch(now: string): string {
    const last = this.sql.exec("SELECT MAX(ordinal) AS n FROM gc_epoch").toArray()[0];
    const ordinal = last && last.n !== null && last.n !== undefined ? Number(last.n) + 1 : 1;
    const epoch = `${this.partition.jurisdiction}/${this.partition.tier}/${ordinal}`;
    this.sql.exec("INSERT INTO gc_epoch (epoch, ordinal, opened_at, marked) VALUES (?, ?, ?, 0)", epoch, ordinal, now);
    return epoch;
  }

  /** Whether the index still shows a fresh (published, not deleted)
   *  replica location for the content. */
  private hasFreshLocation(contentId: string): boolean {
    return this.index.lookup(contentId).locations.some((l) => l.state === "fresh");
  }

  /** Step 1: open an epoch and mark. Candidates come from the index
   *  (a cache); nothing here deletes. */
  mark(opts: { now?: string } = {}): MarkResult {
    const now = opts.now ?? new Date().toISOString();
    const nowMs = parseTime(now, "now");
    const epoch = this.openEpoch(now);
    const graceUntil = new Date(nowMs + this.policy.graceMs).toISOString();

    const cleared: string[] = [];
    for (const c of this.candidates()) {
      if (c.state !== "candidate") continue;
      if (this.index.lookup(c.contentId).referencingDbs.length > 0) {
        this.setState(c.contentId, "live", now);
        cleared.push(c.contentId);
      }
    }

    const marked: string[] = [];
    for (const contentId of this.index.unreferencedContent()) {
      const existing = this.candidate(contentId);
      if (existing?.state === "candidate" || existing?.state === "fenced") continue;
      // Content already deleted comes back only when it was published
      // again; otherwise there is nothing left to collect.
      if (existing?.state === "deleted" && !this.hasFreshLocation(contentId)) continue;
      this.sql.exec(
        `INSERT INTO gc_candidate (content_id, state, epoch, marked_at, grace_until, fence_seq, updated_at)
         VALUES (?, 'candidate', ?, ?, ?, NULL, ?)
         ON CONFLICT(content_id) DO UPDATE SET
           state = 'candidate', epoch = excluded.epoch, marked_at = excluded.marked_at,
           grace_until = excluded.grace_until, fence_seq = NULL, updated_at = excluded.updated_at`,
        contentId,
        epoch,
        now,
        graceUntil,
        now,
      );
      marked.push(contentId);
    }
    this.sql.exec("UPDATE gc_epoch SET marked = ? WHERE epoch = ?", marked.length, epoch);
    return { epoch, marked, cleared };
  }

  private setState(contentId: string, state: CandidateState, now: string, fenceSeq: number | null = null): void {
    this.sql.exec(
      "UPDATE gc_candidate SET state = ?, fence_seq = ?, updated_at = ? WHERE content_id = ?",
      state,
      fenceSeq,
      now,
      contentId,
    );
  }

  /** A new reference to content clears its candidate state (step 2).
   *  Idempotent; content that is not a candidate is left alone.
   *  Returns true when a candidate was cleared. */
  noteReferenced(contentId: string, opts: { now?: string } = {}): boolean {
    const id = parseContentId(contentId);
    const c = this.candidate(id);
    if (c?.state !== "candidate") return false;
    this.setState(id, "live", opts.now ?? new Date().toISOString());
    return true;
  }

  /** The write path's pass through the partition's serialization point
   *  before it creates a reference (step 4). Before the fence, the
   *  reference clears any candidate state and proceeds. After it, the
   *  bytes are gone or going, and the writer re-stages them. */
  admitReference(contentId: string, opts: { now?: string } = {}): AdmitResult {
    const id = parseContentId(contentId);
    const now = opts.now ?? new Date().toISOString();
    const seq = this.nextSeq();
    const c = this.candidate(id);
    if (c?.state === "fenced" || c?.state === "deleted") return { action: "restage", seq };
    if (c?.state === "candidate") this.setState(id, "live", now);
    return { action: "proceed", seq };
  }

  /** Authoritative re-verify (step 3): live references to the content
   *  in this partition across the given databases. */
  private liveReferences(contentId: string, stores: ReferenceStore[]): { dbId: string; createdBy: string }[] {
    const live: { dbId: string; createdBy: string }[] = [];
    for (const store of stores) {
      for (const row of store.referencesFor(contentId)) {
        if (row.tombstoned_at !== null) continue;
        if (row.jurisdiction !== this.partition.jurisdiction || row.sensitivity_tier !== this.partition.tier) continue;
        live.push({ dbId: store.dbId, createdBy: row.created_by });
      }
    }
    return live;
  }

  /** Step 4 and 5: fence, delete through the adapter, audit, and note
   *  the delete on the index. The fence is written before the adapter
   *  is called, so an admission that arrives meanwhile sees it. */
  private async fencedDelete(
    contentId: string,
    epoch: string,
    reverifiedAt: string,
    reason: "grace" | "erasure",
    stores: ReferenceStore[],
  ): Promise<GcAuditRow> {
    const fenceSeq = this.nextSeq();
    this.setState(contentId, "fenced", reverifiedAt, fenceSeq);
    const { deleted } = await this.adapter.deleteReplica(contentId, { epoch, reverifiedAt }, { now: reverifiedAt });
    const audit: DeleteAuditEvent = {
      contentId,
      partition: this.partition,
      epoch,
      reverifiedAt,
      deletedAt: reverifiedAt,
      existed: deleted,
    };
    const dbs = stores.map((s) => s.dbId).sort();
    this.sql.exec(
      `INSERT INTO gc_audit
         (content_id, jurisdiction, sensitivity_tier, epoch, reverified_at, deleted_at, existed, fence_seq, reason, dbs_reverified)
       VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)`,
      contentId,
      this.partition.jurisdiction,
      this.partition.tier,
      epoch,
      reverifiedAt,
      audit.deletedAt,
      deleted ? 1 : 0,
      fenceSeq,
      reason,
      JSON.stringify(dbs),
    );
    this.index.noteDeleted(audit, this.locator);
    this.setState(contentId, "deleted", reverifiedAt, fenceSeq);
    const row = this.sql.exec("SELECT * FROM gc_audit WHERE fence_seq = ?", fenceSeq).toArray()[0];
    if (!row) throw new FailClosed(500, "gc_audit_missing", "the delete audit row was not recorded");
    return toAudit(row);
  }

  /** Steps 3 to 5 for every candidate past grace. `stores` must be the
   *  partition's authoritative databases; any from another partition
   *  is refused before anything is read. */
  async sweep(stores: ReferenceStore[], opts: { now?: string } = {}): Promise<SweepResult> {
    this.requireStores(stores);
    const now = opts.now ?? new Date().toISOString();
    const nowMs = parseTime(now, "now");
    const result: SweepResult = { rescued: [], deleted: [], waiting: [] };
    for (const c of this.candidates()) {
      if (c.state !== "candidate") continue;
      if (parseTime(c.graceUntil, "grace deadline") > nowMs) {
        result.waiting.push(c.contentId);
        continue;
      }
      if (this.liveReferences(c.contentId, stores).length > 0) {
        this.setState(c.contentId, "live", now);
        result.rescued.push(c.contentId);
        continue;
      }
      await this.fencedDelete(c.contentId, c.epoch, now, "grace", stores);
      result.deleted.push(c.contentId);
    }
    return result;
  }

  /** Sole-referencer erasure in `tenant-restricted` (design section 6).
   *  The requester removes their own reference first; this call then
   *  re-verifies. With no live reference left and the policy allowing
   *  it, grace is zero and the delete proceeds under the fence. With
   *  others still referencing the bytes, nothing is deleted and an
   *  erasure note is recorded. */
  async requestErasure(
    contentId: string,
    requester: string,
    stores: ReferenceStore[],
    opts: { now?: string } = {},
  ): Promise<ErasureResult> {
    const id = parseContentId(contentId);
    const who = parseBindingId(requester);
    this.requireStores(stores);
    if (this.partition.tier !== "tenant-restricted") {
      throw new FailClosed(
        409,
        "erasure_wrong_tier",
        "sole-referencer erasure applies to tenant-restricted content only",
      );
    }
    if (!this.policy.soleReferencerErasure) {
      throw new FailClosed(403, "erasure_not_enabled", "the partition policy does not allow zero-grace erasure");
    }
    const now = opts.now ?? new Date().toISOString();
    parseTime(now, "now");

    const everReferenced = stores.some((s) =>
      s
        .referencesFor(id)
        .some(
          (r) =>
            r.created_by === who &&
            r.jurisdiction === this.partition.jurisdiction &&
            r.sensitivity_tier === this.partition.tier,
        ),
    );
    if (!everReferenced) {
      throw new FailClosed(403, "erasure_not_a_referencer", "the requester holds no reference to this content");
    }
    const live = this.liveReferences(id, stores);
    if (live.some((r) => r.createdBy === who)) {
      throw new FailClosed(409, "erasure_reference_still_live", "remove the requester's own reference before erasure");
    }
    if (live.length > 0) {
      this.sql.exec(
        "INSERT INTO gc_erasure_note (content_id, requester, noted_at, other_live_refs) VALUES (?, ?, ?, ?)",
        id,
        who,
        now,
        live.length,
      );
      return { erased: false, outcome: "shared", audit: null };
    }
    const existing = this.candidate(id);
    if (existing?.state === "fenced" || existing?.state === "deleted") {
      return { erased: true, outcome: "deleted", audit: null };
    }
    const epoch = existing?.state === "candidate" ? existing.epoch : this.openEpoch(now);
    this.sql.exec(
      `INSERT INTO gc_candidate (content_id, state, epoch, marked_at, grace_until, fence_seq, updated_at)
       VALUES (?, 'candidate', ?, ?, ?, NULL, ?)
       ON CONFLICT(content_id) DO UPDATE SET
         state = 'candidate', epoch = excluded.epoch, grace_until = excluded.grace_until,
         fence_seq = NULL, updated_at = excluded.updated_at`,
      id,
      epoch,
      now,
      now,
      now,
    );
    const audit = await this.fencedDelete(id, epoch, now, "erasure", stores);
    return { erased: true, outcome: "deleted", audit };
  }
}
