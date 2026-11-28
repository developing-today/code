/**
 * D1 hot-cache measurement (#13107; design #9134 §7, merged via
 * PR #13098). Follow-on 6 of 6 under #9134: the §7 counters surfaced
 * as a report the #9098 embed-storage economics prototype consumes.
 *
 * The design's value is a measured number, not an assertion: hits,
 * skips, revalidations per class and tenant; stale_serves (expected
 * zero — any non-zero value is a bug report, not a tuning signal,
 * §1); estimated Turso cost avoided as skips x the entry's recorded
 * fill cost, labelled an estimate with its method; D1 writes and
 * storage bytes per tenant vs quota.
 *
 * This module is the pure measurement core: an in-memory accumulator
 * the read path feeds (follow-on 4, #13105, owns the live recording;
 * §2's skip log is the feed) plus a pure report renderer over a
 * snapshot of the counters. It does no I/O, reads no clock, and
 * touches no D1 binding — the durable counters live wherever the
 * read path records them, and the report renders from whatever the
 * accumulator was told.
 *
 * Event semantics (the caller drives them; the module only counts):
 *
 *   hit           — a cache lookup found an entry for (tenant, key),
 *                   whether fresh or stale. Every skip and every
 *                   revalidation was a hit first, so in a coherent
 *                   sequence hits = skips + revalidations.
 *   skip          — a Turso hop was skipped: the entry was fresh
 *                   under §1 and served from the cache (§2).
 *   revalidation  — a found entry was stale and the read went to
 *                   Turso to revalidate.
 *   staleServe    — a serve that violated the §1 freshness bound.
 *                   A defect, counted separately (§1, §7).
 *   fill          — an entry was (re)filled into D1: one D1 write,
 *                   payloadBytes written, and the fill-time source
 *                   cost recorded against the key for later skips.
 *   evict         — an entry left D1: one eviction, bytes freed, and
 *                   the key's recorded fill cost forgotten (an
 *                   evicted entry cannot be skipped).
 *
 * The estimated Turso cost avoided is the sum, over skips, of the
 * skipped entry's fill-time source cost (latency ms, rows read).
 * A skip whose entry has no recorded fill cost — filled before the
 * accumulator started, in another isolate, or before a restart —
 * contributes 0 and is counted as unknown, never invented. The
 * accumulator is per-isolate in-memory state, so the estimate is
 * partial by construction; the report says so in its method.
 */

import type { HotCacheClass } from "./hot_cache";

/** Source cost of one query, captured at fill time (§7). */
export interface FillCost {
  /** Source latency in milliseconds for the single query at fill time. */
  latencyMs: number;
  /** Rows read from the source for the single query at fill time. */
  rowsRead: number;
}

/**
 * The §7 counters for one (class) or one (tenant) slice. All fields
 * are cumulative counts since the accumulator was created, except
 * the cost-avoided fields, which are sums over skips.
 */
export interface ClassCounters {
  hits: number;
  skips: number;
  revalidations: number;
  staleServes: number;
  fills: number;
  /** Skips whose entry had no recorded fill cost (contributed 0). */
  skipsWithUnknownFillCost: number;
  /** Estimated Turso latency avoided by this slice's skips (ms). */
  costAvoidedLatencyMs: number;
  /** Estimated Turso rows-read avoided by this slice's skips. */
  costAvoidedRowsRead: number;
  /** Skips that contributed a known fill cost to the estimate. */
  costAvoidedSkips: number;
  /** D1 writes (fills) performed for this slice. */
  d1Writes: number;
  /** D1 bytes written by this slice's fills. */
  d1BytesWritten: number;
  /** D1 bytes freed by this slice's evictions. */
  d1BytesEvicted: number;
}

function zeroCounters(): ClassCounters {
  return {
    hits: 0,
    skips: 0,
    revalidations: 0,
    staleServes: 0,
    fills: 0,
    skipsWithUnknownFillCost: 0,
    costAvoidedLatencyMs: 0,
    costAvoidedRowsRead: 0,
    costAvoidedSkips: 0,
    d1Writes: 0,
    d1BytesWritten: 0,
    d1BytesEvicted: 0,
  };
}

function addInto(into: ClassCounters, from: ClassCounters): void {
  into.hits += from.hits;
  into.skips += from.skips;
  into.revalidations += from.revalidations;
  into.staleServes += from.staleServes;
  into.fills += from.fills;
  into.skipsWithUnknownFillCost += from.skipsWithUnknownFillCost;
  into.costAvoidedLatencyMs += from.costAvoidedLatencyMs;
  into.costAvoidedRowsRead += from.costAvoidedRowsRead;
  into.costAvoidedSkips += from.costAvoidedSkips;
  into.d1Writes += from.d1Writes;
  into.d1BytesWritten += from.d1BytesWritten;
  into.d1BytesEvicted += from.d1BytesEvicted;
}

/** Plain-data snapshot of the accumulator, for the #9098 prototype. */
export interface MeasurementSnapshot {
  /** Counters per class, summed over all tenants. */
  byClass: Record<HotCacheClass, ClassCounters>;
  /** Counters per tenant, summed over all classes. */
  byTenant: Record<string, ClassCounters>;
  /** The full matrix: counters per (tenant, class). */
  byTenantClass: Record<string, Record<HotCacheClass, ClassCounters>>;
  /** D1 storage bytes currently held per tenant (fills minus evictions). */
  storedBytesByTenant: Record<string, number>;
  /** Estimated Turso cost avoided across all skips (see the method). */
  tursoCostAvoided: FillCost & { skips: number };
}

interface RecordedEntry {
  entryClass: HotCacheClass;
  fillCost: FillCost;
  payloadBytes: number;
}

const CLASSES: HotCacheClass[] = ["A", "B", "C"];

function clampNonNegative(value: number): number {
  return Number.isFinite(value) && value > 0 ? value : 0;
}

function clampCost(cost: FillCost): FillCost {
  return { latencyMs: clampNonNegative(cost.latencyMs), rowsRead: clampNonNegative(cost.rowsRead) };
}

/**
 * The §7 counters. One instance per Worker isolate: fills recorded
 * here are the only fill costs later skips can draw on, so a skip in
 * an isolate that never saw the fill contributes 0 (counted as
 * unknown, never invented).
 */
export class HotCacheMeasurement {
  private readonly byTenantClass = new Map<string, Map<HotCacheClass, ClassCounters>>();
  private readonly entries = new Map<string, RecordedEntry>();

  private cell(tenantId: string, entryClass: HotCacheClass): ClassCounters {
    let byClass = this.byTenantClass.get(tenantId);
    if (!byClass) {
      byClass = new Map<HotCacheClass, ClassCounters>();
      this.byTenantClass.set(tenantId, byClass);
    }
    let cell = byClass.get(entryClass);
    if (!cell) {
      cell = zeroCounters();
      byClass.set(entryClass, cell);
    }
    return cell;
  }

  private static entryKey(tenantId: string, cacheKey: string): string {
    return `${tenantId}\n${cacheKey}`;
  }

  /** A cache lookup found an entry for (tenant, key). */
  recordHit(tenantId: string, entryClass: HotCacheClass): void {
    this.cell(tenantId, entryClass).hits += 1;
  }

  /**
   * A Turso hop was skipped: the entry was fresh and served from the
   * cache (§2). Adds the entry's recorded fill cost to the estimated
   * cost avoided; an unrecorded entry contributes 0 and counts as
   * unknown.
   */
  recordSkip(tenantId: string, entryClass: HotCacheClass, cacheKey: string): void {
    const cell = this.cell(tenantId, entryClass);
    cell.skips += 1;
    const recorded = this.entries.get(HotCacheMeasurement.entryKey(tenantId, cacheKey));
    if (!recorded) {
      cell.skipsWithUnknownFillCost += 1;
      return;
    }
    cell.costAvoidedLatencyMs += recorded.fillCost.latencyMs;
    cell.costAvoidedRowsRead += recorded.fillCost.rowsRead;
    cell.costAvoidedSkips += 1;
  }

  /** A found entry was stale and the read revalidated against Turso. */
  recordRevalidation(tenantId: string, entryClass: HotCacheClass): void {
    this.cell(tenantId, entryClass).revalidations += 1;
  }

  /**
   * A serve violated the §1 freshness bound. Expected value zero;
   * any non-zero reading is a bug report (§1), flagged by the report.
   */
  recordStaleServe(tenantId: string, entryClass: HotCacheClass): void {
    this.cell(tenantId, entryClass).staleServes += 1;
  }

  /**
   * An entry was (re)filled into D1: one D1 write, payloadBytes
   * written, and the fill-time source cost recorded against the key.
   * A refill replaces the previous record: stored bytes move by the
   * size delta and later skips draw on the new fill cost.
   */
  recordFill(
    tenantId: string,
    entryClass: HotCacheClass,
    cacheKey: string,
    payloadBytes: number,
    fillCost: FillCost,
  ): void {
    const bytes = clampNonNegative(payloadBytes);
    const cost = clampCost(fillCost);
    const cell = this.cell(tenantId, entryClass);
    cell.fills += 1;
    cell.d1Writes += 1;
    cell.d1BytesWritten += bytes;
    this.entries.set(HotCacheMeasurement.entryKey(tenantId, cacheKey), {
      entryClass,
      fillCost: cost,
      payloadBytes: bytes,
    });
  }

  /**
   * An entry left D1: one eviction, bytes freed, and the key's
   * recorded fill cost forgotten. Evicting an unknown key still
   * counts the eviction but cannot free bytes the accumulator never
   * saw — stored bytes never go negative.
   */
  recordEvict(tenantId: string, entryClass: HotCacheClass, cacheKey: string, payloadBytes: number): void {
    const bytes = clampNonNegative(payloadBytes);
    const cell = this.cell(tenantId, entryClass);
    cell.d1BytesEvicted += bytes;
    this.entries.delete(HotCacheMeasurement.entryKey(tenantId, cacheKey));
  }

  /** A deep copy of the counters; later records do not rewrite it. */
  snapshot(): MeasurementSnapshot {
    const byClass: Record<HotCacheClass, ClassCounters> = {
      A: zeroCounters(),
      B: zeroCounters(),
      C: zeroCounters(),
    };
    const byTenant: Record<string, ClassCounters> = {};
    const byTenantClass: Record<string, Record<HotCacheClass, ClassCounters>> = {};
    const storedBytesByTenant: Record<string, number> = {};
    let avoidedLatencyMs = 0;
    let avoidedRowsRead = 0;
    let avoidedSkips = 0;

    for (const [tenantId, classes] of this.byTenantClass) {
      const tenantTotals = zeroCounters();
      const tenantCells: Record<HotCacheClass, ClassCounters> = {
        A: zeroCounters(),
        B: zeroCounters(),
        C: zeroCounters(),
      };
      for (const entryClass of CLASSES) {
        const cell = classes.get(entryClass);
        if (!cell) continue;
        tenantCells[entryClass] = { ...cell };
        addInto(byClass[entryClass], cell);
        addInto(tenantTotals, cell);
      }
      byTenant[tenantId] = tenantTotals;
      byTenantClass[tenantId] = tenantCells;
    }

    for (const [key, entry] of this.entries) {
      const tenantId = key.slice(0, key.indexOf("\n"));
      storedBytesByTenant[tenantId] = (storedBytesByTenant[tenantId] ?? 0) + entry.payloadBytes;
    }
    // Tenants with counters but no live entries still report storage.
    for (const tenantId of Object.keys(byTenant)) {
      storedBytesByTenant[tenantId] ??= 0;
      avoidedLatencyMs += byTenant[tenantId].costAvoidedLatencyMs;
      avoidedRowsRead += byTenant[tenantId].costAvoidedRowsRead;
      avoidedSkips += byTenant[tenantId].costAvoidedSkips;
    }

    return {
      byClass,
      byTenant,
      byTenantClass,
      storedBytesByTenant,
      tursoCostAvoided: { latencyMs: avoidedLatencyMs, rowsRead: avoidedRowsRead, skips: avoidedSkips },
    };
  }
}

/**
 * The method behind the estimated Turso cost avoided, carried
 * verbatim into the report (§7: reported as an estimate with its
 * method, never as an exact figure).
 */
export const TURSO_COST_AVOIDED_METHOD =
  "sum over Turso-hop skips of the skipped entry's fill-time source cost (latency ms, rows read); " +
  "a fill-time cost is not the cost the hop would have paid now, and skips whose entry has no " +
  "recorded fill cost contribute 0, so this is an estimate, never an exact figure.";

export interface MeasurementReportOptions {
  /**
   * D1 storage quota in bytes per tenant — the §3 quota shares from
   * admission (#13103), supplied by the caller so this module stays
   * independent of the unmerged admission slice. A tenant with no
   * quota reports null and is never over-quota.
   */
  quotaBytesByTenant?: Record<string, number>;
  /**
   * Active config values at report time (TTL_B, heavy-query
   * threshold, tier weights, ...), echoed so the first tuning round
   * has its before-values on the record.
   */
  config?: Record<string, string | number>;
}

/** One slice of the report: counters plus the tuning ratios. */
export interface ClassReport extends ClassCounters {
  /** skips / hits; 0 when no hits were recorded. */
  skipRatio: number;
  /** skips / (skips + revalidations); 0 when neither was recorded. */
  serveRatio: number;
}

/** One tenant's slice: counters, ratios, and storage vs quota. */
export interface TenantReport extends ClassReport {
  storedBytes: number;
  quotaBytes: number | null;
  overQuota: boolean;
}

export interface StorageLine {
  tenantId: string;
  storedBytes: number;
  quotaBytes: number | null;
  overQuota: boolean;
}

/** The §7 report, as plain data for the #9098 prototype. */
export interface MeasurementReport {
  byClass: Record<HotCacheClass, ClassReport>;
  byTenant: Record<string, TenantReport>;
  totals: ClassReport;
  /** stale_serves: expected zero; a non-zero total is a defect (§1). */
  staleServes: { total: number; expectedZero: true; defect: boolean };
  /** Estimated Turso cost avoided, labelled an estimate (§7). */
  tursoCostAvoided: {
    kind: "estimate";
    method: string;
    latencyMs: number;
    rowsRead: number;
    skips: number;
    skipsWithUnknownFillCost: number;
  };
  /** D1 storage bytes per tenant vs quota, sorted by tenant. */
  storage: StorageLine[];
  config?: Record<string, string | number>;
}

function toClassReport(counters: ClassCounters): ClassReport {
  return {
    ...counters,
    skipRatio: counters.hits > 0 ? counters.skips / counters.hits : 0,
    serveRatio:
      counters.skips + counters.revalidations > 0 ? counters.skips / (counters.skips + counters.revalidations) : 0,
  };
}

/**
 * Render the §7 report from a snapshot of live counters. Pure: the
 * same snapshot always renders the same report.
 */
export function renderMeasurementReport(
  snapshot: MeasurementSnapshot,
  options?: MeasurementReportOptions,
): MeasurementReport {
  const byClass: Record<HotCacheClass, ClassReport> = {
    A: toClassReport(snapshot.byClass.A),
    B: toClassReport(snapshot.byClass.B),
    C: toClassReport(snapshot.byClass.C),
  };
  const totalsCounters = zeroCounters();
  for (const entryClass of CLASSES) addInto(totalsCounters, snapshot.byClass[entryClass]);
  const totals = toClassReport(totalsCounters);

  const quotas = options?.quotaBytesByTenant ?? {};
  const byTenant: Record<string, TenantReport> = {};
  const storage: StorageLine[] = [];
  for (const tenantId of Object.keys(snapshot.byTenant).sort()) {
    const storedBytes = snapshot.storedBytesByTenant[tenantId] ?? 0;
    // Indexing the record yields number under this tsconfig, so widen
    // explicitly: a missing quota is undefined, not a TS2367 comparison.
    const quota: number | undefined = quotas[tenantId];
    const quotaBytes = quota !== undefined && Number.isFinite(quota) && quota >= 0 ? quota : null;
    const overQuota = quotaBytes !== null && storedBytes > quotaBytes;
    byTenant[tenantId] = {
      ...toClassReport(snapshot.byTenant[tenantId]),
      storedBytes,
      quotaBytes,
      overQuota,
    };
    storage.push({ tenantId, storedBytes, quotaBytes, overQuota });
  }

  let unknownSkips = 0;
  for (const entryClass of CLASSES) unknownSkips += snapshot.byClass[entryClass].skipsWithUnknownFillCost;

  return {
    byClass,
    byTenant,
    totals,
    staleServes: {
      total: totals.staleServes,
      expectedZero: true as const,
      defect: totals.staleServes > 0,
    },
    tursoCostAvoided: {
      kind: "estimate",
      method: TURSO_COST_AVOIDED_METHOD,
      latencyMs: snapshot.tursoCostAvoided.latencyMs,
      rowsRead: snapshot.tursoCostAvoided.rowsRead,
      skips: snapshot.tursoCostAvoided.skips,
      skipsWithUnknownFillCost: unknownSkips,
    },
    storage,
    ...(options?.config ? { config: { ...options.config } } : {}),
  };
}

/** The human-readable rendering of a report, for the issue record. */
export function formatMeasurementReport(report: MeasurementReport): string {
  const lines: string[] = ["D1 hot-cache measurement report (#13107; design #9134 §7)", ""];
  const t = report.totals;
  lines.push(
    `totals: hits=${t.hits} skips=${t.skips} revalidations=${t.revalidations} ` +
      `stale_serves=${t.staleServes} fills=${t.fills} ` +
      `skip_ratio=${t.skipRatio.toFixed(3)} serve_ratio=${t.serveRatio.toFixed(3)}`,
  );
  for (const entryClass of CLASSES) {
    const c = report.byClass[entryClass];
    lines.push(
      `class ${entryClass}: hits=${c.hits} skips=${c.skips} revalidations=${c.revalidations} ` +
        `stale_serves=${c.staleServes} fills=${c.fills} ` +
        `skip_ratio=${c.skipRatio.toFixed(3)} serve_ratio=${c.serveRatio.toFixed(3)}`,
    );
  }
  lines.push(
    report.staleServes.defect
      ? `stale_serves: ${report.staleServes.total} (expected 0 — DEFECT: any non-zero value files a defect, §1)`
      : `stale_serves: ${report.staleServes.total} (expected 0 — OK)`,
  );
  const avoided = report.tursoCostAvoided;
  lines.push(
    `estimated Turso cost avoided: ${avoided.latencyMs} ms latency, ${avoided.rowsRead} rows read ` +
      `over ${avoided.skips} skips (${avoided.skipsWithUnknownFillCost} skips with unknown fill cost contributed 0)`,
  );
  lines.push(`estimate method: ${avoided.method}`);
  if (report.storage.length > 0) {
    lines.push("D1 storage by tenant:");
    for (const line of report.storage) {
      const quota = line.quotaBytes === null ? "no quota supplied" : `quota ${line.quotaBytes} bytes`;
      lines.push(
        `  ${line.tenantId}: ${line.storedBytes} bytes stored, ${quota}` + (line.overQuota ? " — over quota" : ""),
      );
    }
  }
  if (report.config) {
    lines.push(`config at report time: ${JSON.stringify(report.config)}`);
  }
  return lines.join("\n");
}
