/**
 * D1 tenant hot-cache invalidation (#9134 design §4; implementation
 * #13106, follow-on 5 of 6 under #9134).
 *
 * §4 names three invalidation mechanisms. Mechanism 1 (watermark
 * advance) is lazy and needs no code here: the freshness evaluator
 * (#13101) makes Class B entries stale by rule when the tenant's
 * watermark moves. This module implements mechanisms 2 and 3:
 *
 *   2. Tenant invalidation events (eager, for Class C). The ingest /
 *      write path emits a per-tenant event (tenant id + new
 *      watermark) on completion of a write batch. The v1 transport
 *      is piggyback on the existing ingest completion records
 *      (design Q2 default) — no new transport is built. Consumption
 *      drops the tenant's Class C entries and sets the tenant's
 *      forced-revalidation flag (§2 rule 3) until the next
 *      successful revalidation.
 *   3. Operator purge. A per-tenant purge deletes all classes for
 *      one tenant and sets forced-revalidation. It exists for
 *      incident/support use, is audited (operator, tenant, reason),
 *      and is never invoked by automated paths: this module exposes
 *      it as a store operation only — no route, cron, or request
 *      path in the Worker calls it.
 *
 * Class A entries are never invalidated by mechanisms 1–2: a
 * content-addressed body is either the bytes its hash names or it is
 * corrupt, and corruption is a storage-integrity matter, not a
 * freshness one (§4). The purge is the documented exception: §4
 * mechanism 3 deletes all classes, because an incident purge (e.g. a
 * mis-tiered ingest that must stop being served immediately) must
 * not leave the tenant's rows behind; the purge test pins that
 * all-classes delete as the incident path.
 *
 * The state this module writes is read by the other follow-ons: the
 * freshness evaluator consumes `lastInvalidationAt` for the Class C
 * rule (an event that post-dates an entry makes it stale — this
 * covers entries filled between an event's emission and its
 * consumption on the piggyback transport), and read-path
 * integration (#13105) consults `isForcedRevalidation` for §2 rule 3
 * and calls `noteRevalidation` after a successful revalidation.
 *
 * Like the store (#13100), the whole tier sits behind the
 * HOT_CACHE_ENABLED flag (default off): `invalidationStore` returns
 * undefined when the tier is disabled or unbound.
 */

/** A per-tenant invalidation event (design §4 mechanism 2). */
export interface TenantInvalidationEvent {
  tenantId: string;
  /** The tenant's corpus watermark after the completed write batch. */
  newWatermark: string;
  /** Unix seconds; when the ingest / write path emitted the event. */
  emittedAt: number;
}

/**
 * The v1 piggyback contract (design Q2): an ingest completion record
 * is the record the ingest path already produces when a write batch
 * completes; an invalidation event rides on it as an `invalidation`
 * field with snake_case keys. A record without the field carries no
 * event and yields undefined. A record whose `invalidation` field is
 * present but malformed throws — a half-read event must never be
 * silently dropped, because the Class C drop is a correctness path.
 */
export function invalidationEventFromCompletionRecord(record: unknown): TenantInvalidationEvent | undefined {
  if (typeof record !== "object" || record === null) {
    throw new TypeError("ingest completion record must be an object");
  }
  const raw = (record as { invalidation?: unknown }).invalidation;
  if (raw === undefined || raw === null) return undefined;
  if (typeof raw !== "object") {
    throw new TypeError("ingest completion record: invalidation must be an object");
  }
  const fields = raw as { tenant_id?: unknown; new_watermark?: unknown; emitted_at?: unknown };
  if (typeof fields.tenant_id !== "string" || fields.tenant_id.length === 0) {
    throw new TypeError("ingest completion record: invalidation.tenant_id must be a non-empty string");
  }
  if (typeof fields.new_watermark !== "string" || fields.new_watermark.length === 0) {
    throw new TypeError("ingest completion record: invalidation.new_watermark must be a non-empty string");
  }
  if (typeof fields.emitted_at !== "number" || !Number.isFinite(fields.emitted_at)) {
    throw new TypeError("ingest completion record: invalidation.emitted_at must be a finite number (unix seconds)");
  }
  return { tenantId: fields.tenant_id, newWatermark: fields.new_watermark, emittedAt: fields.emitted_at };
}

/** The recorded invalidation state for one tenant. */
export interface TenantInvalidationState {
  tenantId: string;
  /** §2 rule 3: set by an event or a purge, cleared by the next successful revalidation. */
  forcedRevalidation: boolean;
  /** Unix seconds of the newest event or purge; null when none is recorded. */
  lastInvalidationAt: number | null;
  /** Watermark carried by the newest event; null when the newest invalidation was a purge. */
  lastInvalidatedWatermark: string | null;
  /** Unix seconds; when this state row last changed. */
  updatedAt: number;
}

/** One operator purge, as written to the audit table (design §4 mechanism 3). */
export interface PurgeAuditRecord {
  id: number;
  tenantId: string;
  operator: string;
  reason: string;
  /** Unix seconds. */
  purgedAt: number;
  entriesDeleted: number;
}

interface TenantStateRow {
  tenant_id: string;
  forced_revalidation: number;
  last_invalidation_at: number | null;
  last_invalidated_watermark: string | null;
  updated_at: number;
}

interface PurgeAuditRow {
  id: number;
  tenant_id: string;
  operator: string;
  reason: string;
  purged_at: number;
  entries_deleted: number;
}

export class InvalidationStore {
  constructor(private readonly db: D1Database) {}

  /**
   * Consume one invalidation event (design §4 mechanism 2): drop the
   * tenant's Class C entries and set forced-revalidation until the
   * next successful revalidation. Class A and Class B entries are
   * not deleted — Class A is exempt (§4) and Class B staleness is
   * the lazy watermark rule (mechanism 1).
   *
   * The drop is unconditional: deleting an aggregate is always
   * correctness-safe because it refills from the source. The
   * recorded state, by contrast, moves monotonically — an event that
   * arrives out of order on the piggyback transport never moves
   * `lastInvalidationAt` or the recorded watermark backwards.
   * Returns the number of Class C entries dropped.
   */
  async applyEvent(event: TenantInvalidationEvent): Promise<number> {
    const dropped = await this.db
      .prepare("DELETE FROM hot_cache WHERE tenant_id = ? AND class = 'C'")
      .bind(event.tenantId)
      .run();
    const current = await this.tenantState(event.tenantId);
    const keepCurrent =
      current !== undefined && current.lastInvalidationAt !== null && event.emittedAt < current.lastInvalidationAt;
    await this.db
      .prepare(
        "INSERT OR REPLACE INTO hot_cache_tenant_state (tenant_id, forced_revalidation, last_invalidation_at, last_invalidated_watermark, updated_at) VALUES (?, 1, ?, ?, ?)",
      )
      .bind(
        event.tenantId,
        keepCurrent ? current.lastInvalidationAt : event.emittedAt,
        keepCurrent ? current.lastInvalidatedWatermark : event.newWatermark,
        Math.max(current?.updatedAt ?? 0, event.emittedAt),
      )
      .run();
    return dropped.meta.changes;
  }

  /** The tenant's recorded invalidation state; undefined when nothing is recorded. */
  async tenantState(tenantId: string): Promise<TenantInvalidationState | undefined> {
    const row = await this.db
      .prepare(
        "SELECT tenant_id, forced_revalidation, last_invalidation_at, last_invalidated_watermark, updated_at FROM hot_cache_tenant_state WHERE tenant_id = ?",
      )
      .bind(tenantId)
      .first<TenantStateRow>();
    if (!row) return undefined;
    return {
      tenantId: row.tenant_id,
      forcedRevalidation: row.forced_revalidation !== 0,
      lastInvalidationAt: row.last_invalidation_at,
      lastInvalidatedWatermark: row.last_invalidated_watermark,
      updatedAt: row.updated_at,
    };
  }

  /** §2 rule 3: true while the tenant is in forced-revalidation state. */
  async isForcedRevalidation(tenantId: string): Promise<boolean> {
    const state = await this.tenantState(tenantId);
    return state?.forcedRevalidation ?? false;
  }

  /**
   * The tenant's latest invalidation time (unix seconds), or null
   * when no event or purge is recorded — the seam the freshness
   * evaluator's Class C rule consumes.
   */
  async lastInvalidationAt(tenantId: string): Promise<number | null> {
    const state = await this.tenantState(tenantId);
    return state?.lastInvalidationAt ?? null;
  }

  /**
   * Record the next successful revalidation (§4): clears the
   * forced-revalidation flag. The invalidation history
   * (`lastInvalidationAt`, watermark) is kept — the Class C rule
   * still needs it for entries filled before the event. A tenant
   * with no recorded state is already clear; this is a no-op.
   */
  async noteRevalidation(tenantId: string, atSeconds: number): Promise<void> {
    await this.db
      .prepare("UPDATE hot_cache_tenant_state SET forced_revalidation = 0, updated_at = ? WHERE tenant_id = ?")
      .bind(atSeconds, tenantId)
      .run();
  }

  /**
   * The audited operator purge (design §4 mechanism 3): delete ALL
   * classes for one tenant, set forced-revalidation, and write the
   * audit record (operator, tenant, reason) in the same operation.
   * Incident/support use only — never invoked by automated paths.
   * Returns the audit record as written.
   */
  async purgeTenant(purge: {
    tenantId: string;
    operator: string;
    reason: string;
    atSeconds: number;
  }): Promise<PurgeAuditRecord> {
    const deleted = await this.db
      .prepare("DELETE FROM hot_cache WHERE tenant_id = ?")
      .bind(purge.tenantId)
      .run();
    const current = await this.tenantState(purge.tenantId);
    const keepCurrent =
      current !== undefined && current.lastInvalidationAt !== null && purge.atSeconds < current.lastInvalidationAt;
    await this.db
      .prepare(
        "INSERT OR REPLACE INTO hot_cache_tenant_state (tenant_id, forced_revalidation, last_invalidation_at, last_invalidated_watermark, updated_at) VALUES (?, 1, ?, ?, ?)",
      )
      .bind(
        purge.tenantId,
        keepCurrent ? current.lastInvalidationAt : purge.atSeconds,
        keepCurrent ? current.lastInvalidatedWatermark : null,
        Math.max(current?.updatedAt ?? 0, purge.atSeconds),
      )
      .run();
    const audit = await this.db
      .prepare(
        "INSERT INTO hot_cache_purge_audit (tenant_id, operator, reason, purged_at, entries_deleted) VALUES (?, ?, ?, ?, ?) RETURNING id, tenant_id, operator, reason, purged_at, entries_deleted",
      )
      .bind(purge.tenantId, purge.operator, purge.reason, purge.atSeconds, deleted.meta.changes)
      .first<PurgeAuditRow>();
    if (!audit) throw new Error("purge audit insert returned no row");
    return {
      id: audit.id,
      tenantId: audit.tenant_id,
      operator: audit.operator,
      reason: audit.reason,
      purgedAt: audit.purged_at,
      entriesDeleted: audit.entries_deleted,
    };
  }

  /** Audit records for one tenant, newest first. */
  async listPurges(tenantId: string): Promise<PurgeAuditRecord[]> {
    const result = await this.db
      .prepare(
        "SELECT id, tenant_id, operator, reason, purged_at, entries_deleted FROM hot_cache_purge_audit WHERE tenant_id = ? ORDER BY id DESC",
      )
      .bind(tenantId)
      .all<PurgeAuditRow>();
    return result.results.map((row) => ({
      id: row.id,
      tenantId: row.tenant_id,
      operator: row.operator,
      reason: row.reason,
      purgedAt: row.purged_at,
      entriesDeleted: row.entries_deleted,
    }));
  }
}

/**
 * The invalidation store for a request environment, or undefined
 * when the tier is disabled or unbound — same gating as the cache
 * store (#13100).
 */
export function invalidationStore(env: {
  HOT_CACHE?: D1Database;
  HOT_CACHE_ENABLED?: string;
}): InvalidationStore | undefined {
  if (env.HOT_CACHE_ENABLED !== "true" || !env.HOT_CACHE) return undefined;
  return new InvalidationStore(env.HOT_CACHE);
}
