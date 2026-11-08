// Historical burn + measured burn rate derivation (#3933, the #851
// remainder). Pure functions over pool-window observations: the
// fleet-feed collector persists one observation per pool window per
// run (its own state file, beside the charges-surfaced state), and
// this module turns that series into
//
//   - BurnSegment[] — completed reset periods only. A period is
//     complete when a later observation shows the window reset
//     (a different resetsAt, or used dropping below the period's
//     last reading). The segment's tokens are the period's final
//     observed `used`; from/to bound the observations of that
//     period. The in-progress period is never emitted as history.
//   - burnTokensPerMin per pool — a MEASURED rate: the token delta
//     between the two most recent observations of one tokens-unit
//     window inside the same reset period, over a named window of
//     at most RATE_WINDOW_MS. No pair, no rate: the field is
//     omitted, never estimated. Only tokens-unit windows qualify —
//     the field is tokens/min by definition (model.ts QuotaPool),
//     and a pct/usd/requests window cannot honestly produce one.
//
// Honest-absence contract (#3933 acceptance): pools and windows
// without supporting observations yield nothing. Segments exist
// only for tokens-unit windows for the same reason — BurnSegment
// .tokens must be tokens, not a relabelled percent or dollar.
import type { BurnSegment, QuotaPool } from "./model.ts";

/** One pool window as observed at one collection instant. */
export interface PoolWindowObservation {
  pool: string;
  provider: string;
  window: string;
  unit: string;
  used: number;
  resetsAt: string | null;
  at: string;
}

/** The named measurement window for a burn rate: the two most
 * recent same-period observations must be at most this far apart
 * (and strictly ordered) for their delta to count as a rate. */
export const RATE_WINDOW_MS = 120 * 60 * 1000;

/** Observations to keep per pool+window in the collector state. */
export const MAX_OBSERVATIONS_PER_WINDOW = 500;

/** Observations older than this are pruned from collector state. */
export const OBSERVATION_RETENTION_MS = 35 * 24 * 60 * 60 * 1000;

export function observationsFromPools(pools: QuotaPool[], at: string): PoolWindowObservation[] {
  const out: PoolWindowObservation[] = [];
  for (const pool of pools) {
    for (const w of pool.windows) {
      if (!Number.isFinite(w.used) || w.used < 0) continue;
      out.push({
        pool: pool.id,
        provider: pool.provider,
        window: w.name,
        unit: w.unit,
        used: w.used,
        resetsAt: w.resetsAt ?? null,
        at,
      });
    }
  }
  return out;
}

function keyOf(o: PoolWindowObservation): string {
  return `${o.pool}${o.window}`;
}

/** Merge new observations into persisted state: dedupe identical
 * (pool, window, at) points (the later one wins), sort by time,
 * cap per window, and prune past retention. */
export function mergeObservations(
  prior: PoolWindowObservation[],
  fresh: PoolWindowObservation[],
  nowMs: number,
): PoolWindowObservation[] {
  const byKey = new Map<string, PoolWindowObservation>();
  for (const o of [...prior, ...fresh]) {
    if (!o || typeof o.pool !== "string" || typeof o.window !== "string") continue;
    if (!Number.isFinite(o.used) || o.used < 0) continue;
    const t = Date.parse(o.at);
    if (!Number.isFinite(t) || nowMs - t > OBSERVATION_RETENTION_MS) continue;
    byKey.set(`${keyOf(o)}${o.at}`, o);
  }
  const groups = new Map<string, PoolWindowObservation[]>();
  for (const o of byKey.values()) {
    const k = keyOf(o);
    const list = groups.get(k) ?? [];
    list.push(o);
    groups.set(k, list);
  }
  const out: PoolWindowObservation[] = [];
  for (const list of groups.values()) {
    list.sort((a, b) => Date.parse(a.at) - Date.parse(b.at));
    out.push(...list.slice(-MAX_OBSERVATIONS_PER_WINDOW));
  }
  out.sort((a, b) => Date.parse(a.at) - Date.parse(b.at) || keyOf(a).localeCompare(keyOf(b)));
  return out;
}

interface Period {
  resetsAt: string | null;
  first: PoolWindowObservation;
  last: PoolWindowObservation;
  complete: boolean;
}

/** Split one window's series into reset periods. A new period
 * starts when resetsAt changes to a different non-null value, or
 * when `used` drops below the previous reading (a reset the
 * schedule moved through between observations). */
function periodsOf(series: PoolWindowObservation[]): Period[] {
  const periods: Period[] = [];
  for (const o of series) {
    const cur = periods[periods.length - 1];
    if (!cur) {
      periods.push({ resetsAt: o.resetsAt, first: o, last: o, complete: false });
      continue;
    }
    const resetBoundary =
      (o.resetsAt !== null && cur.resetsAt !== null && o.resetsAt !== cur.resetsAt) || o.used < cur.last.used;
    if (resetBoundary) {
      cur.complete = true;
      periods.push({ resetsAt: o.resetsAt, first: o, last: o, complete: false });
    } else {
      cur.last = o;
      if (o.resetsAt !== null) cur.resetsAt = o.resetsAt;
    }
  }
  return periods;
}

export interface BurnHistory {
  segments: BurnSegment[];
  /** pool id -> measured tokens/min (only pools with a real pair). */
  burnTokensPerMin: Map<string, number>;
}

export function deriveBurnHistory(observations: PoolWindowObservation[]): BurnHistory {
  const groups = new Map<string, PoolWindowObservation[]>();
  for (const o of observations) {
    if (o.unit !== "tokens") continue;
    const k = keyOf(o);
    const list = groups.get(k) ?? [];
    list.push(o);
    groups.set(k, list);
  }
  const segments: BurnSegment[] = [];
  const rates = new Map<string, { rate: number; at: number }>();
  for (const series of groups.values()) {
    const sorted = [...series].sort((a, b) => Date.parse(a.at) - Date.parse(b.at));
    const periods = periodsOf(sorted);
    for (const p of periods) {
      if (p.complete && p.last.used > 0) {
        segments.push({
          provider: p.last.provider,
          pool: p.last.pool,
          from: p.first.at,
          to: p.last.at,
          tokens: p.last.used,
        });
      }
    }
    // Measured rate: the most recent same-period pair.
    const current = periods[periods.length - 1];
    if (current && current.first !== current.last) {
      const prev = sorted[sorted.length - 2];
      const lastObs = sorted[sorted.length - 1];
      if (prev && lastObs && prev !== lastObs) {
        // Both points must belong to the current (incomplete) period.
        const inCurrent = (o: PoolWindowObservation) =>
          Date.parse(o.at) >= Date.parse(current.first.at) && Date.parse(o.at) <= Date.parse(current.last.at);
        if (inCurrent(prev) && inCurrent(lastObs)) {
          const dtMs = Date.parse(lastObs.at) - Date.parse(prev.at);
          const delta = lastObs.used - prev.used;
          if (dtMs > 0 && dtMs <= RATE_WINDOW_MS && delta >= 0) {
            const rate = delta / (dtMs / 60000);
            const existing = rates.get(lastObs.pool);
            if (!existing || Date.parse(lastObs.at) > existing.at) {
              rates.set(lastObs.pool, { rate, at: Date.parse(lastObs.at) });
            }
          }
        }
      }
    }
  }
  segments.sort((a, b) => Date.parse(a.from) - Date.parse(b.from) || a.pool.localeCompare(b.pool));
  return {
    segments,
    burnTokensPerMin: new Map([...rates].map(([pool, v]) => [pool, v.rate])),
  };
}
