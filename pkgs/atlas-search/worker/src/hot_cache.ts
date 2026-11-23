/**
 * D1 tenant hot cache store (#9134 design §5; implementation #13100).
 *
 * Primitives over the single shared `hot_cache` table (schema in
 * migrations/0001_hot_cache.sql): get / put / touch / evict for one
 * (tenant, key) row. The tenant is carried in the key, never in a
 * per-tenant database (design §5).
 *
 * This module is deliberately only the storage layer. Freshness
 * evaluation (design §1) is follow-on #13101, admission and the
 * eviction sweep (§3/§5) are #13103, and read-path integration is
 * #13105. Nothing reads through this store until those land, and the
 * whole tier sits behind the HOT_CACHE_ENABLED flag (default off):
 * with the flag off, `hotCacheStore` returns undefined and callers
 * keep the pre-cache path exactly — zero behavior change.
 */

export type HotCacheClass = "A" | "B" | "C";

export interface HotCachePut {
  tenantId: string;
  cacheKey: string;
  entryClass: HotCacheClass;
  payload: Uint8Array;
  sourceWatermark: string;
  /** Unix seconds; the fill time recorded by the filler. */
  filledAt: number;
}

export interface HotCacheEntry extends HotCachePut {
  payloadBytes: number;
  lastHitAt: number;
  hitCount: number;
}

interface HotCacheRow {
  tenant_id: string;
  cache_key: string;
  class: string;
  payload: ArrayBuffer;
  payload_bytes: number;
  source_watermark: string;
  filled_at: number;
  last_hit_at: number;
  hit_count: number;
}

export class HotCacheStore {
  constructor(private readonly db: D1Database) {}

  /** Read one entry; undefined when the (tenant, key) pair is absent. */
  async get(tenantId: string, cacheKey: string): Promise<HotCacheEntry | undefined> {
    const row = await this.db
      .prepare(
        "SELECT tenant_id, cache_key, class, payload, payload_bytes, source_watermark, filled_at, last_hit_at, hit_count FROM hot_cache WHERE tenant_id = ? AND cache_key = ?",
      )
      .bind(tenantId, cacheKey)
      .first<HotCacheRow>();
    if (!row) return undefined;
    return {
      tenantId: row.tenant_id,
      cacheKey: row.cache_key,
      entryClass: row.class as HotCacheClass,
      payload: new Uint8Array(row.payload),
      payloadBytes: row.payload_bytes,
      sourceWatermark: row.source_watermark,
      filledAt: row.filled_at,
      lastHitAt: row.last_hit_at,
      hitCount: row.hit_count,
    };
  }

  /**
   * Insert or replace one entry. A fresh fill starts its hit
   * bookkeeping over: last_hit_at is the fill time, hit_count is 0.
   */
  async put(entry: HotCachePut): Promise<void> {
    await this.db
      .prepare(
        "INSERT OR REPLACE INTO hot_cache (tenant_id, cache_key, class, payload, payload_bytes, source_watermark, filled_at, last_hit_at, hit_count) VALUES (?, ?, ?, ?, ?, ?, ?, ?, 0)",
      )
      .bind(
        entry.tenantId,
        entry.cacheKey,
        entry.entryClass,
        entry.payload,
        entry.payload.byteLength,
        entry.sourceWatermark,
        entry.filledAt,
        entry.filledAt,
      )
      .run();
  }

  /** Record a served hit; drives the §5 least-recently-hit eviction order. */
  async touch(tenantId: string, cacheKey: string, atSeconds: number): Promise<void> {
    await this.db
      .prepare(
        "UPDATE hot_cache SET last_hit_at = ?, hit_count = hit_count + 1 WHERE tenant_id = ? AND cache_key = ?",
      )
      .bind(atSeconds, tenantId, cacheKey)
      .run();
  }

  /** Delete one entry; true when a row was actually removed. */
  async evict(tenantId: string, cacheKey: string): Promise<boolean> {
    const result = await this.db
      .prepare("DELETE FROM hot_cache WHERE tenant_id = ? AND cache_key = ?")
      .bind(tenantId, cacheKey)
      .run();
    return result.meta.changes > 0;
  }
}

/** The config flag (design §8 item 1): only the literal "true" enables the tier. */
export function hotCacheEnabled(env: { HOT_CACHE_ENABLED?: string }): boolean {
  return env.HOT_CACHE_ENABLED === "true";
}

/**
 * The store for a request environment, or undefined when the tier is
 * disabled or unbound — the caller's signal to use the pre-cache path.
 */
export function hotCacheStore(env: {
  HOT_CACHE?: D1Database;
  HOT_CACHE_ENABLED?: string;
}): HotCacheStore | undefined {
  if (!hotCacheEnabled(env) || !env.HOT_CACHE) return undefined;
  return new HotCacheStore(env.HOT_CACHE);
}
