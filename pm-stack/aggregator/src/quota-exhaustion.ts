/**
 * Per-pool quota exhaustion intervals (#3983), composing the live
 * quota contract #3933 landed: pool identity, remaining/limit, reset
 * time, measured timestamped burn (PoolWindowObservation series),
 * source freshness, and explicit missing markers.
 *
 * The forecast complements — never replaces — the existing point
 * pace (digest) and near-limit/wall derivations, and adds no new
 * threshold or attention path: it only projects when the current
 * window fills at the measured burn-rate distribution, and says so
 * honestly as an interval (P50/P80 of the observed same-period pair
 * rates within RATE_WINDOW_MS).
 *
 * Honesty rules, mirroring burn-history: anything not measurable is
 * an explicit `unknown` with a reason — never "safe", never a
 * fabricated date. Rates come only from consecutive same-period
 * observation pairs (a pair straddling a reset measures nothing
 * about the current window). Queued work is never converted into
 * token demand; only measured burn is projected.
 */
import { RATE_WINDOW_MS, type PoolWindowObservation } from "./burn-history.ts";
import type { ExhaustionForecast, QuotaPool } from "./model.ts";

/** Minimum same-period pair rates before percentiles mean anything:
 * with fewer, the honest answer is an unknown, not a point dressed
 * up as an interval. */
export const MIN_RATE_SAMPLES = 2;

/** Nearest-rank percentile of an ascending-sorted sample. */
function percentile(sortedAsc: number[], p: number): number {
  const rank = Math.max(1, Math.ceil((p / 100) * sortedAsc.length));
  return sortedAsc[rank - 1];
}

export function deriveExhaustionForecasts(
  pools: QuotaPool[],
  observations: PoolWindowObservation[],
  now = new Date(),
): ExhaustionForecast[] {
  const nowMs = now.getTime();
  const byWindow = new Map<string, PoolWindowObservation[]>();
  for (const o of observations) {
    const k = `${o.pool}${o.window}`;
    const list = byWindow.get(k) ?? [];
    list.push(o);
    byWindow.set(k, list);
  }

  const out: ExhaustionForecast[] = [];
  for (const pool of pools) {
    const forecasts: ExhaustionForecast[] = [];
    for (const w of pool.windows) {
      const base = {
        pool: pool.id,
        provider: pool.provider,
        window: w.name,
        unit: w.unit,
        resetsAt: w.resetsAt ?? null,
        rateWindowMs: RATE_WINDOW_MS,
        source: pool.freshness?.source ?? null,
      };
      const unknown = (reason: NonNullable<ExhaustionForecast["reason"]>, remaining: number | null) => {
        forecasts.push({ ...base, state: "unknown", reason, remaining });
      };

      if (w.limit === null || !Number.isFinite(w.limit)) {
        unknown("missing_limit", null);
        continue;
      }
      const remaining = w.limit - w.used;
      // Freshness BEFORE the exhausted shortcut (Codex P2 on PR #8522):
      // a stale/errored pool can retain a last-known window at its
      // limit (quota-current.ts preserves dated windows for stale
      // reads and adopts retained windows on errors); if the quota
      // has since reset, that retained reading asserts nothing about
      // current exhaustion. Stale source = explicit unknown, always.
      if (pool.freshness && pool.freshness.state !== "ok") {
        unknown("stale_source", remaining);
        continue;
      }
      const resetMs = w.resetsAt ? Date.parse(w.resetsAt) : Number.NaN;
      if (Number.isFinite(resetMs) && resetMs <= nowMs) {
        unknown("reset_passed", remaining);
        continue;
      }
      if (remaining <= 0) {
        forecasts.push({ ...base, state: "exhausted", remaining, exhaustsAt: now.toISOString() });
        continue;
      }
      if (!w.resetsAt || !Number.isFinite(Date.parse(w.resetsAt))) {
        unknown("missing_reset", remaining);
        continue;
      }

      // Measured pair rates for the CURRENT period only: both sides
      // carry this window's resetsAt, are strictly ordered, and are
      // at most RATE_WINDOW_MS apart with a non-negative delta.
      // Filter to the current period BEFORE pairing: a previous-
      // reset observation interleaved in time must not break the
      // adjacency of consecutive current-period pairs (property
      // coverage, Codex P1 on PR #8522).
      const series = [...(byWindow.get(`${pool.id}${w.name}`) ?? [])]
        .filter((o) => o.resetsAt === w.resetsAt)
        .sort((a, b) => Date.parse(a.at) - Date.parse(b.at));
      const newest = series[series.length - 1];
      if (!newest || nowMs - Date.parse(newest.at) > RATE_WINDOW_MS) {
        unknown("no_measured_burn", remaining);
        continue;
      }
      const rates: number[] = [];
      for (let i = 1; i < series.length; i++) {
        const prev = series[i - 1];
        const last = series[i];
        if (prev.resetsAt !== w.resetsAt || last.resetsAt !== w.resetsAt) continue;
        const dtMs = Date.parse(last.at) - Date.parse(prev.at);
        const delta = last.used - prev.used;
        // Recency, not just separation (Codex P2 on PR #8522): a pair
        // whose later side is itself older than RATE_WINDOW_MS before
        // now measures obsolete activity, not current burn — two
        // qualifying pairs from yesterday plus one observation now
        // must not publish a forecast from yesterday's rates.
        const ageMs = nowMs - Date.parse(last.at);
        if (dtMs > 0 && dtMs <= RATE_WINDOW_MS && ageMs <= RATE_WINDOW_MS && delta >= 0) {
          rates.push(delta / (dtMs / 60000));
        }
      }
      if (rates.length === 0) {
        unknown("no_measured_burn", remaining);
        continue;
      }
      if (rates.length < MIN_RATE_SAMPLES) {
        unknown("insufficient_observations", remaining);
        continue;
      }
      rates.sort((a, b) => a - b);
      const p50 = percentile(rates, 50);
      const p80 = percentile(rates, 80);
      if (!(p50 > 0)) {
        unknown("zero_rate", remaining);
        continue;
      }
      const p50ExhaustsMs = nowMs + (remaining / p50) * 60000;
      const p80ExhaustsMs = nowMs + (remaining / p80) * 60000;
      forecasts.push({
        ...base,
        state: "forecast",
        remaining,
        sampleRates: rates.length,
        ratesPerMin: { p50, p80 },
        p50ExhaustsAt: new Date(p50ExhaustsMs).toISOString(),
        p80ExhaustsAt: new Date(p80ExhaustsMs).toISOString(),
        p50VsReset: p50ExhaustsMs < resetMs ? "before" : "after",
        p80VsReset: p80ExhaustsMs < resetMs ? "before" : "after",
      });
    }
    out.push(...forecasts);
    if (forecasts.length) pool.exhaustion = forecasts;
  }
  return out;
}
