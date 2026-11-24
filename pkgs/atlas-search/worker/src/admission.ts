/**
 * D1 hot-cache admission policy (#13103, follow-on 3 of 6 under
 * #9134): heat counters, heavy-query immediate admission, per-tenant
 * quotas, and the LRU eviction sweep — design §3 and §5
 * (doc/2026-11-23T08-06-00Z_design_atlas_d1_tenant_hot_cache).
 *
 * This module is pure policy: it imports nothing from the store
 * (follow-on 1, #13100) or the freshness evaluator (follow-on 2,
 * #13101), and its entry facts mirror the design §5 schema fields,
 * so the read-path integration (follow-on 4) wires it to whatever
 * store lands. Every threshold is a named config value (program
 * rule: everything behavioral is configuration); the defaults are
 * the design's, marked there as estimates to be re-tuned from the
 * first measurement round.
 */

export interface AdmissionConfig {
  /** Heat: requests inside the window that make a key eligible. Default 2 (§3). */
  heatRequests: number;
  /** Heat window, milliseconds. Default 10 minutes (§3). */
  heatWindowMs: number;
  /** Heavy query: source latency strictly above this (ms) admits on first request. Default 250 (§3). */
  heavyLatencyMs: number;
  /** Heavy query: rows-read strictly above this admits on first request. Default 10,000 (§3). */
  heavyRowsRead: number;
  /** Per-tenant quota weights by tier index. Default [1, 4, 16] across the #9083 tiers (§3). */
  tierWeights: readonly number[];
  /** Fraction of the total budget reserved per active tenant. Default 0.05 (§3). */
  tenantFloorFraction: number;
  /** Entry size cap, serialized bytes; larger entries are never admitted. Default 256 KiB (§3). */
  maxEntryBytes: number;
  /** Negative (empty) results are admitted with this TTL, ms. Default 30 s (§3). */
  negativeTtlMs: number;
}

export const DEFAULT_ADMISSION_CONFIG: AdmissionConfig = {
  heatRequests: 2,
  heatWindowMs: 10 * 60 * 1000,
  heavyLatencyMs: 250,
  heavyRowsRead: 10_000,
  tierWeights: [1, 4, 16],
  tenantFloorFraction: 0.05,
  maxEntryBytes: 256 * 1024,
  negativeTtlMs: 30 * 1000,
};

// ---------------------------------------------------------------------------
// Heat counters (§3: eligible after `heatRequests` requests inside
// `heatWindowMs`). In-memory and per-isolate: heat is evidence, not
// state of record — losing a counter on isolate recycle costs one
// extra source fill, never correctness. All times are caller-supplied
// milliseconds, so the tracker is deterministic under test.
// ---------------------------------------------------------------------------

export class HeatTracker {
  private readonly config: AdmissionConfig;
  private readonly hits = new Map<string, number[]>();

  constructor(config: AdmissionConfig) {
    this.config = config;
  }

  private slot(tenantId: string, cacheKey: string): string {
    return JSON.stringify([tenantId, cacheKey]);
  }

  /** Timestamps for the key that fall inside the window ending at `nowMs`. */
  private live(tenantId: string, cacheKey: string, nowMs: number): number[] {
    const slot = this.slot(tenantId, cacheKey);
    const kept = (this.hits.get(slot) ?? []).filter((ts) => nowMs - ts < this.config.heatWindowMs);
    if (kept.length === 0) this.hits.delete(slot);
    else this.hits.set(slot, kept);
    return kept;
  }

  /** Record one request at `nowMs`; returns the in-window count including it. */
  record(tenantId: string, cacheKey: string, nowMs: number): number {
    const kept = this.live(tenantId, cacheKey, nowMs);
    kept.push(nowMs);
    this.hits.set(this.slot(tenantId, cacheKey), kept);
    return kept.length;
  }

  /** In-window request count at `nowMs`, recording nothing. */
  count(tenantId: string, cacheKey: string, nowMs: number): number {
    return this.live(tenantId, cacheKey, nowMs).length;
  }

  /** Whether the key has demonstrated heat at `nowMs`. */
  eligible(tenantId: string, cacheKey: string, nowMs: number): boolean {
    return this.count(tenantId, cacheKey, nowMs) >= this.config.heatRequests;
  }

  /** Drop all counters (tests, operator resets). */
  clear(): void {
    this.hits.clear();
  }
}

// ---------------------------------------------------------------------------
// The admission decision (§3). One fill is judged on its facts: the
// size cap is absolute (a heavy or hot oversized entry is still
// refused — it would consume a tenant's whole quota), then the
// cost-based heavy exception, then heat.
// ---------------------------------------------------------------------------

export interface FillFacts {
  /** Serialized payload size, bytes. */
  payloadBytes: number;
  /** Source latency of the fill query, ms (estimate or measured). */
  sourceLatencyMs: number;
  /** Rows read at the source by the fill query (estimate or measured). */
  rowsRead: number;
  /** True when the result set is empty (a negative result). */
  isNegative: boolean;
  /** Requests for this key inside the heat window, including the current one. */
  heatCount: number;
}

export type AdmissionDecision =
  | { admit: true; reason: "heavy" | "heat"; ttlMs: number | null }
  | { admit: false; reason: "oversized" | "cold"; ttlMs: null };

export function decideAdmission(facts: FillFacts, config: AdmissionConfig): AdmissionDecision {
  if (facts.payloadBytes > config.maxEntryBytes) {
    return { admit: false, reason: "oversized", ttlMs: null };
  }
  // Negative results ride the same gates (a single poll is one-hit
  // traffic like any other); admission just shortens their life to
  // the negative TTL so a miss expires quickly once ingestion lands.
  const ttlMs = facts.isNegative ? config.negativeTtlMs : null;
  if (facts.sourceLatencyMs > config.heavyLatencyMs || facts.rowsRead > config.heavyRowsRead) {
    return { admit: true, reason: "heavy", ttlMs };
  }
  if (facts.heatCount >= config.heatRequests) {
    return { admit: true, reason: "heat", ttlMs };
  }
  return { admit: false, reason: "cold", ttlMs: null };
}

// ---------------------------------------------------------------------------
// Per-tenant quotas (§3): each active tenant first reserves the
// floor fraction of the total budget; the remainder is split by
// tier weight. If the reservations alone would exceed the budget
// (more active tenants than the floor admits), the floor is
// impossible and the whole budget splits by weight instead —
// shares always sum to the budget and never exceed it.
// ---------------------------------------------------------------------------

export interface TenantTier {
  tenantId: string;
  /** Index into `config.tierWeights`. */
  tier: number;
}

export function quotaShares(
  tenants: readonly TenantTier[],
  totalBudgetBytes: number,
  config: AdmissionConfig,
): Map<string, number> {
  const shares = new Map<string, number>();
  if (tenants.length === 0 || totalBudgetBytes <= 0) return shares;
  let totalWeight = 0;
  for (const t of tenants) {
    const w = config.tierWeights[t.tier];
    if (w === undefined) {
      throw new RangeError(
        `quotaShares: tenant ${t.tenantId} names tier ${t.tier}, outside config.tierWeights (0..${config.tierWeights.length - 1})`,
      );
    }
    totalWeight += w;
  }
  const floorBytes = Math.floor(config.tenantFloorFraction * totalBudgetBytes);
  const reserved = floorBytes * tenants.length;
  const useFloor = reserved <= totalBudgetBytes;
  const distributable = useFloor ? totalBudgetBytes - reserved : totalBudgetBytes;
  let assigned = 0;
  for (const t of tenants) {
    const base = useFloor ? floorBytes : 0;
    const share = base + Math.floor((distributable * config.tierWeights[t.tier]) / totalWeight);
    shares.set(t.tenantId, share);
    assigned += share;
  }
  // Flooring leaves a remainder smaller than the tenant count;
  // hand it to the first tenant so shares sum to the budget.
  const remainder = totalBudgetBytes - assigned;
  if (remainder > 0) {
    const first = tenants[0].tenantId;
    shares.set(first, (shares.get(first) ?? 0) + remainder);
  }
  return shares;
}

// ---------------------------------------------------------------------------
// The eviction sweep (§5). Quota binds only under pressure: while
// the database is within its total budget, unused quota is
// borrowable and no tenant sheds anything, however far over its
// own share it sits. Under pressure, each over-share tenant sheds
// its least-recently-hit entries — Class B and C before Class A
// (A bodies are re-derivable from the content store, so they are
// evicted last) — until it is back within its share. A tenant with
// no share entry has no quota and sheds everything.
// ---------------------------------------------------------------------------

export type EntryClass = "A" | "B" | "C";

/** The §5 schema facts the sweep judges on. Times are the schema's unix seconds. */
export interface EvictionEntry {
  tenantId: string;
  cacheKey: string;
  entryClass: EntryClass;
  payloadBytes: number;
  lastHitAt: number;
  hitCount: number;
}

function classRank(c: EntryClass): number {
  return c === "A" ? 1 : 0;
}

export function planEvictions(
  entries: readonly EvictionEntry[],
  shares: ReadonlyMap<string, number>,
  totalBudgetBytes: number,
): EvictionEntry[] {
  let totalBytes = 0;
  const byTenant = new Map<string, EvictionEntry[]>();
  for (const e of entries) {
    totalBytes += e.payloadBytes;
    const list = byTenant.get(e.tenantId);
    if (list) list.push(e);
    else byTenant.set(e.tenantId, [e]);
  }
  if (totalBytes <= totalBudgetBytes) return [];

  const plan: EvictionEntry[] = [];
  for (const tenantId of [...byTenant.keys()].sort()) {
    const list = byTenant.get(tenantId) ?? [];
    let tenantBytes = list.reduce((sum, e) => sum + e.payloadBytes, 0);
    const share = shares.get(tenantId) ?? 0;
    if (tenantBytes <= share) continue;
    const victims = [...list].sort(
      (a, b) =>
        classRank(a.entryClass) - classRank(b.entryClass) ||
        a.lastHitAt - b.lastHitAt ||
        a.hitCount - b.hitCount ||
        (a.cacheKey < b.cacheKey ? -1 : a.cacheKey > b.cacheKey ? 1 : 0),
    );
    for (const victim of victims) {
      if (tenantBytes <= share) break;
      plan.push(victim);
      tenantBytes -= victim.payloadBytes;
    }
  }
  return plan;
}
