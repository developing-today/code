// Stage-3 shadow placement (#12697, epic #12694; design
// doc/2026-11-22T22-35-00Z_design_quota-plane-stage-3, decision
// D3): each gatherer tick, beside the forecast product, the tick
// computes the placement ordering the forecast implies and logs it
// BESIDE the live ordering as a per-decision diff record —
// quota-shadow-placement.json (latest) plus one appended line per
// tick in quota-shadow-placement-history.jsonl, the accumulating
// record the weekly review (S7) scores. SHADOW ONLY: nothing in the
// live path reads this module's output, the live ordering below is
// a function of the measured data (quota-current) alone, and the
// shadow adjustment can demote a pool in the logged ordering but
// can never promote a measured demotion, never remove a pool, and
// never close anything. Live forecast-driven admission is S4's
// decision, taken on this record's evidence after at least a week
// of shadow diffs.
//
// The orderings. Placement ranks pools (keyed by the canonical
// provider id sessions carry — the pool-state.json / forecast
// keying) for new work:
// - Live: burn-by-expiry under today's measured doctrine. Tiers by
//   measured state — open (fresh ok, below the caution zone), then
//   unknown (no fresh ok reading: unknown is NOT closed, the pool
//   stays in the ordering, it just has no measured expiry to burn
//   by), then caution (fresh max percent at/over NEAR_PCT, the
//   >=85% zone derive.ts names — today's measured demotion), then
//   walled (a fresh window at 100%: closed until reset by the
//   standing doctrine). Within a tier: soonest window reset first
//   (burn the headroom that expires first — pools should reach
//   ~100% AT their reset), then fullest first, then provider id,
//   so the order is total and deterministic.
// - Shadow: the same ordering with ONE adjustment, applied before
//   the caution zone is reached: a pool in the open tier whose
//   consumed forecast projects a wall before its reset is demoted
//   into the caution tier. That is the whole forecast effect —
//   demotion in a logged ordering, nothing else.
//
// Forecast consumption rules (the design's freshness contract plus
// S2's named findings, #12696):
// - A forecast window is consumed only while its underlying reading
//   is fresh by quota-current.ts's overlay bounds; a stale forecast
//   degrades to NO forecast for that window — logged as an
//   exclusion, never silently used. (S1 only projects fresh-ok
//   readings, so in the tick path this is a belt-and-braces check
//   on the product's own contract.)
// - A window the forecast itself marks "insufficient-history" is
//   logged with its state and reason and never consumed: the
//   shadow invents no ordering from a series S2 could not score.
// - The bare opencode-go pool's MONTHLY forecast is never consumed:
//   S2's backtest showed that series is a cross-workspace mixing
//   artifact (the product assembles the bare pool from every
//   workspace's records; its monthly row scored precision 0.054
//   against per-workspace rows at 0.93-1.0). The per-workspace
//   pools (opencode-go.<workspace>) consume their monthly rows
//   normally. The exclusion is logged on the record, not silent.
// - Weekly-window projections run systematically late under the
//   linear pace (S2's other headline finding). That is calibration
//   knowledge for S7's review, not a consumption rule: v1 consumes
//   the projection as published and lets the shadow record itself
//   measure how the demotions fare.
//
// The record carries everything S7 needs to score shadow-vs-live
// after the fact: both orderings, the decision point's identity
// (the gatherer tick, with the current and forecast products'
// generatedAt stamps), and per pool the measured inputs, EVERY
// forecast window row considered with its honesty state and
// whether it was consumed or excluded (and why), both ranks, and
// the reasons for any movement. Pure: no I/O, no clock reads except
// through arguments — gather-quota.ts does the file IO.
import { NEAR_PCT } from "./derive.ts";
import { QUOTA_CURRENT_FRESH_MS, QUOTA_CURRENT_FUTURE_SKEW_MS } from "./quota-current.ts";
import type { QuotaForecastFile, QuotaForecastWindow } from "./quota-forecast.ts";
import type { QuotaCurrentEntry, QuotaCurrentFile } from "./model.ts";

/** Placement tier of one pool in one ordering. */
export type ShadowTier = "open" | "unknown" | "caution" | "walled";

const TIER_RANK: Record<ShadowTier, number> = { open: 0, unknown: 1, caution: 2, walled: 3 };

/** One forecast window row as the shadow considered it: the S1
 * projection fields verbatim, plus the consumption verdict. */
export interface ShadowForecastInput {
  serviceId: string;
  window: string;
  state: QuotaForecastWindow["state"];
  wallsBeforeReset: boolean | null;
  projectedWallAt: number | null;
  ratePtsPerHour: number | null;
  samples: number;
  pctNow: number | null;
  measuredAt: string;
  resetsAt: string | null;
  reason?: string;
  consumed: boolean;
  /** Why the row was not consumed, when it was not. */
  exclusion?: string;
}

export interface ShadowPoolRecord {
  providerId: string;
  serviceIds: string[];
  liveTier: ShadowTier;
  shadowTier: ShadowTier;
  liveRank: number;
  shadowRank: number;
  moved: "demoted" | "promoted" | "unchanged";
  demotedByForecast: boolean;
  measured: {
    /** Max pctOfLimit across fresh ok windows; null when no fresh
     * ok reading exists (never a stale number republished). */
    percent: number | null;
    /** The window attaining the max percent. */
    bindingWindow: string | null;
    /** The binding window's reset. */
    resetsAt: string | null;
    /** Soonest reset across the fresh ok percent windows — the
     * burn-by-expiry key. */
    soonestResetAt: string | null;
    /** pool-state.ts semantics: walled / near / healthy from a
     * fresh ok entry, else the newest entry's state ("stale" for a
     * stale ok). */
    state: string | null;
  };
  forecastInputs: ShadowForecastInput[];
  reasons: string[];
}

export interface ShadowPlacementRecord {
  version: 1;
  kind: "quota-shadow-placement";
  /** The tick instant this record was computed at. */
  generatedAt: string;
  decisionPoint: {
    kind: "gatherer-tick";
    at: string;
    currentGeneratedAt: string;
    forecastGeneratedAt: string | null;
  };
  liveOrdering: string[];
  shadowOrdering: string[];
  /** Per-pool records, in live-ordering order. */
  pools: ShadowPoolRecord[];
  summary: {
    pools: number;
    moved: number;
    demotedByForecast: number;
    identical: boolean;
  };
}

interface PoolMeasured {
  providerId: string;
  serviceIds: string[];
  percent: number | null;
  bindingWindow: string | null;
  bindingResetsAt: string | null;
  soonestResetMs: number | null;
  state: string | null;
  tier: ShadowTier;
}

/** quota-current.ts's freshness test, applied to one entry
 * (pool-state.ts's isFreshOk rule). */
function isFreshOk(entry: QuotaCurrentEntry, nowMs: number): boolean {
  if (entry.state !== "ok") return false;
  const t = Date.parse(entry.measuredAt);
  return Number.isFinite(t) && t - nowMs <= QUOTA_CURRENT_FUTURE_SKEW_MS && nowMs - t <= QUOTA_CURRENT_FRESH_MS;
}

/** The measured half of a pool: fresh-ok windows only. A stale or
 * errored entry asserts no percent and no wall (pool-state.ts's
 * rule, quota-exhaustion.ts's stale_source rule). */
function measurePool(providerId: string, entries: QuotaCurrentEntry[], nowMs: number): PoolMeasured {
  const serviceIds = [...new Set(entries.map((e) => e.serviceId))].sort();
  const freshOk = entries.filter((e) => isFreshOk(e, nowMs));
  let percent: number | null = null;
  let bindingWindow: string | null = null;
  let bindingResetsAt: string | null = null;
  let bindingResetMs = Infinity;
  let soonestResetMs: number | null = null;
  for (const entry of freshOk) {
    for (const w of entry.windows) {
      if (w.state !== "ok" || typeof w.pctOfLimit !== "number" || !Number.isFinite(w.pctOfLimit)) continue;
      const resetMs = w.resetsAt ? Date.parse(w.resetsAt) : NaN;
      if (Number.isFinite(resetMs) && (soonestResetMs === null || resetMs < soonestResetMs)) soonestResetMs = resetMs;
      // Binding window: max percent; ties to the earliest reset,
      // then the window name — order-independent.
      const better =
        percent === null ||
        w.pctOfLimit > percent ||
        (w.pctOfLimit === percent &&
          ((Number.isFinite(resetMs) ? resetMs : Infinity) < bindingResetMs ||
            ((Number.isFinite(resetMs) ? resetMs : Infinity) === bindingResetMs && (bindingWindow === null || w.name < bindingWindow))));
      if (better) {
        percent = w.pctOfLimit;
        bindingWindow = w.name;
        bindingResetsAt = w.resetsAt;
        bindingResetMs = Number.isFinite(resetMs) ? resetMs : Infinity;
      }
    }
  }
  if (freshOk.length > 0 && percent !== null) {
    return {
      providerId,
      serviceIds,
      percent,
      bindingWindow,
      bindingResetsAt,
      soonestResetMs,
      state: percent >= 100 ? "walled" : percent >= NEAR_PCT ? "near" : "healthy",
      tier: percent >= 100 ? "walled" : percent >= NEAR_PCT ? "caution" : "open",
    };
  }
  // No fresh ok percent: the newest entry's state rides, with a
  // stale "ok" named as what it is (pool-state.ts).
  const newest = entries.reduce((a, b) => (Date.parse(b.measuredAt) > Date.parse(a.measuredAt) ? b : a));
  return {
    providerId,
    serviceIds,
    percent: null,
    bindingWindow: null,
    bindingResetsAt: null,
    soonestResetMs: null,
    state: newest.state === "ok" ? "stale" : newest.state,
    tier: "unknown",
  };
}

function comparePools(a: PoolMeasured, tierOf: (p: PoolMeasured) => ShadowTier, b: PoolMeasured): number {
  const tierDiff = TIER_RANK[tierOf(a)] - TIER_RANK[tierOf(b)];
  if (tierDiff !== 0) return tierDiff;
  const expiryA = a.soonestResetMs ?? Infinity;
  const expiryB = b.soonestResetMs ?? Infinity;
  if (expiryA !== expiryB) return expiryA - expiryB;
  const pctA = a.percent ?? -Infinity;
  const pctB = b.percent ?? -Infinity;
  if (pctA !== pctB) return pctB - pctA;
  return a.providerId < b.providerId ? -1 : a.providerId > b.providerId ? 1 : 0;
}

function groupByProvider(current: QuotaCurrentFile): Map<string, QuotaCurrentEntry[]> {
  const byProvider = new Map<string, QuotaCurrentEntry[]>();
  for (const entry of current.providers) {
    const list = byProvider.get(entry.providerId) ?? [];
    list.push(entry);
    byProvider.set(entry.providerId, list);
  }
  return byProvider;
}

/**
 * The live placement ordering: a function of the measured data
 * alone. Exported so the no-live-effect property is directly
 * testable — the shadow record's liveOrdering is this function's
 * output, whatever forecast was (or was not) supplied.
 */
export function liveOrderingFor(current: QuotaCurrentFile, now = new Date()): string[] {
  const nowMs = now.getTime();
  const measured = [...groupByProvider(current)].map(([providerId, entries]) => measurePool(providerId, entries, nowMs));
  return measured
    .sort((a, b) => comparePools(a, (p) => p.tier, b))
    .map((p) => p.providerId);
}

/** The S2 exclusion: the bare opencode-go pool's monthly series is
 * assembled from every workspace's records — a mixing artifact,
 * not a pool forecast. Per-workspace rows are unaffected. */
function isBareOpencodeGoMonthly(serviceId: string, windowName: string): boolean {
  return serviceId === "opencode-go" && windowName === "monthly";
}

/** Consider one provider's forecast rows: build the logged inputs
 * and decide whether the forecast demotes the pool in shadow. */
function considerForecast(
  forecast: QuotaForecastFile | null,
  providerId: string,
  nowMs: number,
): { inputs: ShadowForecastInput[]; demote: boolean; demoteReasons: string[] } {
  const inputs: ShadowForecastInput[] = [];
  const demoteReasons: string[] = [];
  let demote = false;
  for (const pool of forecast?.pools[providerId] ?? []) {
    for (const w of pool.windows) {
      let consumed = true;
      let exclusion: string | undefined;
      if (isBareOpencodeGoMonthly(pool.serviceId, w.name)) {
        consumed = false;
        exclusion =
          "S2 mixing artifact: the bare opencode-go pool's monthly forecast is assembled from every workspace's records and is not a pool forecast (per-workspace rows are the real ones) — never consumed";
      } else if (w.state === "insufficient-history") {
        consumed = false;
        exclusion = "forecast honesty state: insufficient-history — logged, never consumed";
      } else {
        const measuredMs = Date.parse(w.measuredAt);
        if (
          !Number.isFinite(measuredMs) ||
          measuredMs - nowMs > QUOTA_CURRENT_FUTURE_SKEW_MS ||
          nowMs - measuredMs > QUOTA_CURRENT_FRESH_MS
        ) {
          consumed = false;
          exclusion = "stale forecast: the underlying reading is outside the quota-current freshness bound — a stale forecast degrades to no forecast";
        }
      }
      inputs.push({
        serviceId: pool.serviceId,
        window: w.name,
        state: w.state,
        wallsBeforeReset: w.wallsBeforeReset,
        projectedWallAt: w.projectedWallAt,
        ratePtsPerHour: w.ratePtsPerHour,
        samples: w.samples,
        pctNow: w.pctNow,
        measuredAt: w.measuredAt,
        resetsAt: w.resetsAt,
        ...(w.reason ? { reason: w.reason } : {}),
        consumed,
        ...(exclusion ? { exclusion } : {}),
      });
      if (consumed && w.state === "projected" && w.wallsBeforeReset === true) {
        demote = true;
        demoteReasons.push(
          `forecast: ${pool.serviceId} ${w.name} projected to wall at ${
            w.projectedWallAt !== null ? new Date(w.projectedWallAt).toISOString() : "?"
          } before its reset ${w.resetsAt ?? "?"} — demoted in shadow before the caution zone`,
        );
      }
    }
  }
  return { inputs, demote, demoteReasons };
}

/**
 * Build one tick's shadow-placement diff record from the tick's
 * quota-current file and forecast product (null when the forecast
 * failed this tick — the record still lands, with the shadow
 * ordering equal to the live one and no forecast inputs). Pure:
 * no I/O, no clock reads except through arguments.
 */
export function buildShadowPlacement(
  current: QuotaCurrentFile,
  forecast: QuotaForecastFile | null,
  now = new Date(),
): ShadowPlacementRecord {
  const nowMs = now.getTime();
  const byProvider = groupByProvider(current);
  const measured = [...byProvider].map(([providerId, entries]) => measurePool(providerId, entries, nowMs));

  const liveSorted = [...measured].sort((a, b) => comparePools(a, (p) => p.tier, b));
  const liveRank = new Map(liveSorted.map((p, i) => [p.providerId, i + 1]));

  const shadowTierOf = new Map<string, ShadowTier>();
  const perPool = new Map<string, ReturnType<typeof considerForecast>>();
  for (const pool of measured) {
    const considered = considerForecast(forecast, pool.providerId, nowMs);
    perPool.set(pool.providerId, considered);
    // The whole forecast effect: an open pool projected to wall
    // before its reset is demoted into the caution tier. Measured
    // tiers are never lifted, unknown pools have no consumable
    // forecast (S1 projects fresh-ok readings only), and no pool
    // is ever removed or closed by a forecast.
    shadowTierOf.set(pool.providerId, pool.tier === "open" && considered.demote ? "caution" : pool.tier);
  }
  const shadowSorted = [...measured].sort((a, b) => comparePools(a, (p) => shadowTierOf.get(p.providerId)!, b));
  const shadowRank = new Map(shadowSorted.map((p, i) => [p.providerId, i + 1]));

  const pools: ShadowPoolRecord[] = liveSorted.map((pool) => {
    const considered = perPool.get(pool.providerId)!;
    const lr = liveRank.get(pool.providerId)!;
    const sr = shadowRank.get(pool.providerId)!;
    const demotedByForecast = considered.demote && pool.tier === "open";
    const reasons: string[] = [];
    if (demotedByForecast) reasons.push(...considered.demoteReasons);
    else if (sr < lr) reasons.push("relative movement: a pool ranked above this pool in the live ordering was demoted by the forecast");
    else if (sr > lr) reasons.push("relative movement: the forecast demotions reordered the tiers around this pool");
    return {
      providerId: pool.providerId,
      serviceIds: pool.serviceIds,
      liveTier: pool.tier,
      shadowTier: shadowTierOf.get(pool.providerId)!,
      liveRank: lr,
      shadowRank: sr,
      moved: sr > lr ? "demoted" : sr < lr ? "promoted" : "unchanged",
      demotedByForecast,
      measured: {
        percent: pool.percent,
        bindingWindow: pool.bindingWindow,
        resetsAt: pool.bindingResetsAt,
        soonestResetAt: pool.soonestResetMs !== null ? new Date(pool.soonestResetMs).toISOString() : null,
        state: pool.state,
      },
      forecastInputs: considered.inputs,
      reasons,
    };
  });

  const liveOrdering = liveSorted.map((p) => p.providerId);
  const shadowOrdering = shadowSorted.map((p) => p.providerId);
  return {
    version: 1,
    kind: "quota-shadow-placement",
    generatedAt: now.toISOString(),
    decisionPoint: {
      kind: "gatherer-tick",
      at: now.toISOString(),
      currentGeneratedAt: current.generatedAt,
      forecastGeneratedAt: forecast?.generatedAt ?? null,
    },
    liveOrdering,
    shadowOrdering,
    pools,
    summary: {
      pools: pools.length,
      moved: pools.filter((p) => p.moved !== "unchanged").length,
      demotedByForecast: pools.filter((p) => p.demotedByForecast).length,
      identical: liveOrdering.every((id, i) => shadowOrdering[i] === id),
    },
  };
}
