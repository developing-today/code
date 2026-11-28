// The Stage-3 forecast product (#12695, epic #12694; design
// doc/2026-11-22T22-35-00Z_design_quota-plane-stage-3, decisions
// D1/D2): each gatherer tick, after quota-current.json is written,
// the tick publishes quota-forecast.json beside it — the
// quota-analytics.ts pace projection promoted from display math to
// data — plus one Turso ledger row per pool/window (see
// buildForecastLedgerRows) so forecast accuracy (slice S2) joins
// forecasts to outcomes in one place.
//
// D2 discipline: the forecast model in v1 is the existing pace math,
// UNCHANGED. This module never fits a slope itself — it assembles
// the oracle's inputs (the current reading + the same-period series)
// and calls projectWindow. Per design §2 the history can support
// same-period linear pace only: 33 days of records is ~4-5 samples
// of each weekday per window, too thin for per-weekday profiles, so
// v1 deliberately has no weekday/hourly modelling — a forecast the
// data cannot support is an honesty state, not a tuned guess.
//
// Keying (the pool-state.json lesson from #1338): pools are keyed by
// the canonical provider id sessions carry (entry.providerId, which
// the gatherer already canonicalizes in quota-gather-map.ts), never
// by the aggregator's service ids; the service id rides on each pool
// for provenance and for the ledger join.
//
// Honesty rules, mirroring the neighbourhood:
// - Freshness is quota-current.ts's overlay rule (its entryState):
//   only a fresh ok reading is projected. A retained last-known
//   entry is stale BY DEFINITION (#1268), however recent its
//   measuredAt; an aged ok reading is stale; an errored/lapsed
//   provider read projects nothing.
// - A pool/window whose latest reading is not fresh-ok still gets a
//   forecast row — state "insufficient-history" with a reason naming
//   why, samples 0, every projection field null, pctNow null (a
//   stale percentage is never republished as current). measuredAt
//   and resetsAt still ride, so consumers can age the row by the
//   design's freshness contract: a forecast is consumable only
//   while its underlying reading is fresh.
// - Thin history is the oracle's own verdict: projectWindow's
//   "insufficient-history" (with its reason) passes through
//   verbatim, never papered over.
//
// Series join (design §2): snapshot service ids drifted across the
// period (antigravity vs _antigravity, command-code vs commandcode
// vs _commandcode, meta vs meta:high-usage, openai.main vs
// _openai.main). forecastSeriesKey is the explicit mapping: both the
// pool's service id and every record's service id fold through it,
// and a pool also accepts records under its sub-ids — the same join
// analytics-store.ts's seriesForWindow performs, so the forecast
// and the /quota display agree on the same inputs.
import { projectWindow, type PctPoint, type WindowProjection } from "./quota-analytics.ts";
import { QUOTA_CURRENT_FRESH_MS, QUOTA_CURRENT_FUTURE_SKEW_MS } from "./quota-current.ts";
import type { QuotaCurrentEntry, QuotaCurrentFile, QuotaCurrentWindow, SnapshotRecord } from "./model.ts";

/** Turso ledger source for forecast rows: distinct from the reading
 * rows' GATHERER_SOURCE so accuracy scoring selects forecasts
 * without payload sniffing; the shape rides in the payload. */
export const FORECAST_LEDGER_SOURCE = "quota-forecast";

export interface QuotaForecastWindow extends WindowProjection {
  name: string;
  /** The current reading the projection is from; null when no fresh
   * ok reading exists (never a stale percentage republished). */
  pctNow: number | null;
  /** The reading's own measuredAt (the winning window's stamp). */
  measuredAt: string;
  /** The window's reset instant as read this tick. */
  resetsAt: string | null;
}

export interface QuotaForecastPool {
  providerId: string;
  serviceId: string;
  plan: string | null;
  windows: QuotaForecastWindow[];
}

export interface QuotaForecastFile {
  version: 1;
  generatedAt: string;
  source: "gather-quota";
  /** Pools keyed by canonical provider id; a provider with several
   * pool rows (split pools, per-workspace rows) carries them all. */
  pools: Record<string, QuotaForecastPool[]>;
}

/**
 * The explicit serviceId rename mapping (design §2), as one folding
 * function applied to BOTH sides of the join: a leading underscore
 * (the #1823 config-id rename), the colon/dot drift, and the
 * hyphen drift (command-code) all fold away. Distinct pools stay
 * distinct — the fold only removes spelling, never structure.
 */
export function forecastSeriesKey(serviceId: string): string {
  return serviceId.replace(/^_/, "").replace(/:/g, ".").replace(/-/g, "");
}

/** Window-name -> snapshot metric ids, mirroring analytics-store.ts
 * WINDOW_METRIC_ALIASES: the two joins must agree, or the forecast
 * and the display would fit different series for the same window. */
const WINDOW_METRIC_ALIASES: Record<string, string[]> = {
  "5-hour": ["5h", "5-hour"],
  "7d": ["7d", "weekly"],
  weekly: ["weekly", "7d"],
  monthly: ["monthly"],
  api: ["api"],
  vibe: ["vibe"],
};

/** quota-current.ts's overlay freshness for one entry (its
 * entryState rule), reduced to the forecast's question: may this
 * entry's readings be projected from? */
type ReadingFreshness = "ok" | "stale" | "retained" | "lapsed" | "error";

function entryFreshness(entry: QuotaCurrentEntry, nowMs: number): ReadingFreshness {
  if (entry.state === "lapsed") return "lapsed";
  if (entry.state === "ok") {
    if (entry.retainedFromFailedTick) return "retained";
    const t = Date.parse(entry.measuredAt);
    return Number.isFinite(t) && t - nowMs <= QUOTA_CURRENT_FUTURE_SKEW_MS && nowMs - t <= QUOTA_CURRENT_FRESH_MS
      ? "ok"
      : "stale";
  }
  return "error";
}

function noFreshReason(freshness: ReadingFreshness, entry: QuotaCurrentEntry): string {
  switch (freshness) {
    case "retained":
      return "latest reading is retained last-known from a failed tick — stale by the quota-current overlay rule, never projected from";
    case "stale":
      return "latest reading is stale (outside the quota-current freshness bound) — a forecast is never projected from stale data";
    case "lapsed":
      return "provider session lapsed — no current reading to project from";
    default:
      return `provider read ${entry.state} this tick — no current reading to project from`;
  }
}

/** Percent points for one pool window from the snapshot history,
 * rename-joined (forecastSeriesKey + sub-id acceptance) with the
 * analytics-store metric matching and (ts, value) dedupe. */
function pointsForWindow(index: Map<string, SnapshotRecord[]>, poolServiceId: string, windowName: string): PctPoint[] {
  const poolKey = forecastSeriesKey(poolServiceId);
  const aliases = WINDOW_METRIC_ALIASES[windowName] ?? [windowName];
  const metrics = new Set(aliases);
  const points: PctPoint[] = [];
  const seen = new Set<string>();
  for (const [key, records] of index) {
    if (key !== poolKey && !key.startsWith(`${poolKey}.`)) continue;
    for (const r of records) {
      if (!metrics.has(r.metricId) && !r.metricId.endsWith(`:${windowName}`) && !r.metricId.endsWith(`:${aliases[0]}`))
        continue;
      const ts = Date.parse(r.ts);
      if (!Number.isFinite(ts)) continue;
      const dedupe = `${ts}|${r.value}`;
      if (seen.has(dedupe)) continue;
      seen.add(dedupe);
      points.push({ ts, pct: r.value as number, resetsAt: r.resetsAt ?? null });
    }
  }
  return points.sort((a, b) => a.ts - b.ts);
}

/** recordsFromEntries' pool-level pick, mirrored: the max-pct
 * reading is the binding one; lowest credentialId, then earliest
 * measuredAt, then lowest resetsAt break ties so the pick is
 * order-independent. */
function bindingReading(candidates: { entry: QuotaCurrentEntry; w: QuotaCurrentWindow }[]): {
  entry: QuotaCurrentEntry;
  w: QuotaCurrentWindow;
} {
  let best = candidates[0];
  for (const cand of candidates.slice(1)) {
    const pct = cand.w.pctOfLimit ?? -Infinity;
    const bestPct = best.w.pctOfLimit ?? -Infinity;
    if (pct !== bestPct) {
      if (pct > bestPct) best = cand;
      continue;
    }
    const cred = cand.entry.credentialId ?? "";
    const bestCred = best.entry.credentialId ?? "";
    if (cred !== bestCred) {
      if (cred < bestCred) best = cand;
      continue;
    }
    if (cand.w.measuredAt !== best.w.measuredAt) {
      if (cand.w.measuredAt < best.w.measuredAt) best = cand;
      continue;
    }
    if ((cand.w.resetsAt ?? "") < (best.w.resetsAt ?? "")) best = cand;
  }
  return best;
}

/**
 * Build one tick's forecast product from the tick's quota-current
 * file and the snapshot store's history (the caller merges this
 * tick's just-appended records in). Pure: no I/O, no clock reads
 * except through arguments — gather-quota.ts does the file IO.
 */
export function buildQuotaForecast(
  file: QuotaCurrentFile,
  history: readonly SnapshotRecord[],
  now = new Date(),
): QuotaForecastFile {
  const nowMs = now.getTime();

  // Index the percent history by folded series key once; each pool
  // then reads its own key plus sub-id keys.
  const index = new Map<string, SnapshotRecord[]>();
  for (const r of history) {
    if (typeof r.value !== "number" || r.unit !== "pct") continue;
    const key = forecastSeriesKey(r.serviceId);
    const list = index.get(key) ?? [];
    list.push(r);
    index.set(key, list);
  }

  // Group the tick's entries: providerId -> serviceId -> entries.
  const byProvider = new Map<string, Map<string, QuotaCurrentEntry[]>>();
  for (const entry of file.providers) {
    let byService = byProvider.get(entry.providerId);
    if (!byService) {
      byService = new Map();
      byProvider.set(entry.providerId, byService);
    }
    const list = byService.get(entry.serviceId) ?? [];
    list.push(entry);
    byService.set(entry.serviceId, list);
  }

  const pools: Record<string, QuotaForecastPool[]> = {};
  for (const providerId of [...byProvider.keys()].sort()) {
    const byService = byProvider.get(providerId) as Map<string, QuotaCurrentEntry[]>;
    const providerPools: QuotaForecastPool[] = [];
    for (const serviceId of [...byService.keys()].sort()) {
      const entries = byService.get(serviceId) as QuotaCurrentEntry[];
      const windowNames = new Set<string>();
      for (const e of entries) for (const w of e.windows) windowNames.add(w.name);

      const windows: QuotaForecastWindow[] = [];
      for (const name of [...windowNames].sort()) {
        const candidates = entries.flatMap((entry) =>
          entry.windows.filter((w) => w.name === name).map((w) => ({ entry, w })),
        );
        const points = pointsForWindow(index, serviceId, name);
        const freshOk = candidates.filter(
          (c) => entryFreshness(c.entry, nowMs) === "ok" && c.w.state === "ok" && c.w.pctOfLimit != null,
        );
        if (freshOk.length > 0) {
          const { w } = bindingReading(freshOk);
          const projection = projectWindow({
            points,
            pctNow: w.pctOfLimit as number,
            resetsAt: w.resetsAt,
            now: nowMs,
          });
          windows.push({ name, ...projection, pctNow: w.pctOfLimit, measuredAt: w.measuredAt, resetsAt: w.resetsAt });
          continue;
        }
        // No fresh ok reading. A window with no percentage anywhere
        // (a label-only balance row) is not a quota window and gets
        // no forecast row; a real quota window gets the explicit
        // honesty row.
        const hasPct = candidates.some((c) => c.w.pctOfLimit != null) || points.length > 0;
        if (!hasPct) continue;
        const newest = candidates.reduce((a, b) => (Date.parse(b.w.measuredAt) > Date.parse(a.w.measuredAt) ? b : a));
        windows.push({
          name,
          state: "insufficient-history",
          ratePtsPerHour: null,
          samples: 0,
          projectedWallAt: null,
          durationUntilWallMs: null,
          wallsBeforeReset: null,
          reason: noFreshReason(entryFreshness(newest.entry, nowMs), newest.entry),
          pctNow: null,
          measuredAt: newest.w.measuredAt,
          resetsAt: newest.w.resetsAt,
        });
      }

      providerPools.push({
        providerId,
        serviceId,
        plan: entries.find((e) => e.planLabel)?.planLabel ?? null,
        windows,
      });
    }
    pools[providerId] = providerPools;
  }

  return { version: 1, generatedAt: file.generatedAt, source: "gather-quota", pools };
}

/** One Turso agent_quota_records-class ledger row (the gatherer's
 * sink shape: id / source / ts / plan / weekly_percent / resets /
 * payload / inserted_at, minus inserted_at which the sink stamps). */
export interface ForecastLedgerRow {
  id: string;
  source: string;
  ts: string;
  plan: string | null;
  weeklyPercent: number | null;
  resets: string | null;
  payload: Record<string, unknown>;
}

/**
 * The ledger half of D1: one row per pool/window per tick, beside
 * the reading rows in agent_quota_records, so the accuracy scoring
 * (S2, §7 of the design) joins forecasts to outcomes in one place.
 * The payload is the forecast contract — the same fields the
 * quota-forecast.json window carries, flattened with its pool
 * identity. weekly_percent follows the reading rows' convention
 * (the weekly window's own percent; null on other windows and on
 * honesty rows, whose pctNow is null by rule).
 */
export function buildForecastLedgerRows(forecast: QuotaForecastFile, tickAt: string): ForecastLedgerRow[] {
  const rows: ForecastLedgerRow[] = [];
  for (const providerId of Object.keys(forecast.pools).sort()) {
    for (const pool of forecast.pools[providerId]) {
      for (const w of pool.windows) {
        rows.push({
          id: `${pool.serviceId}:${w.name}:${tickAt}`,
          source: FORECAST_LEDGER_SOURCE,
          ts: tickAt,
          plan: pool.plan,
          weeklyPercent: w.name === "weekly" ? w.pctNow : null,
          resets: w.resetsAt,
          payload: {
            kind: "quota-forecast",
            version: 1,
            providerId: pool.providerId,
            serviceId: pool.serviceId,
            window: w.name,
            state: w.state,
            ratePtsPerHour: w.ratePtsPerHour,
            samples: w.samples,
            projectedWallAt: w.projectedWallAt,
            wallsBeforeReset: w.wallsBeforeReset,
            pctNow: w.pctNow,
            measuredAt: w.measuredAt,
            resetsAt: w.resetsAt,
            ...(w.reason ? { reason: w.reason } : {}),
          },
        });
      }
    }
  }
  return rows;
}
