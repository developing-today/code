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
//
// Live token source (Codex review on PR #7354): today's quota
// sources emit only percent ("other", limit 100) or USD windows —
// extractQuotaPools and the gatherer's percent windows alike — so
// window-derived rates/segments stay honestly empty until a
// tokens-unit window source exists. The collector's one real
// measured token stream is OpenCode session attribution
// (poolBurn over tokensBySession): PoolTokenObservation carries
// the per-pool sum of active sessions' cumulative tokens at each
// collection PLUS the per-session counters behind the sum, and
// deriveMeasuredRatesFromPoolTokens turns a pair of observations
// into the measured rate from the MATCHING session counters only
// (Codex round 2 on PR #7354): a session that resumed between
// collections must not inject its historical tokens into the
// delta, and a session that ended must not subtract its own. A
// pair whose counters cannot be compared — a missing per-session
// breakdown on either side, or every matching counter reset —
// yields no rate, never a guess. A rate is also only as current
// as its newest reading: when the caller supplies the collection
// instant, a pair whose last observation is older than
// RATE_WINDOW_MS relative to it yields no rate, however close
// the pair itself is (an idle pool or an unreachable server must
// not keep publishing its last throughput for the 35-day
// retention period).
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

/** One pool's live token total at one collection instant: the sum
 * of cumulative tokens over the sessions attributed to the pool
 * right now (the collector's poolBurn attribution). sessionTokens
 * carries the per-session cumulative counters behind the sum
 * (session id -> tokens) so rate derivation can compare MATCHING
 * sessions across a pair of observations; observations persisted
 * before the field existed do not carry it, and a pair involving
 * one of those cannot be compared honestly (no rate). */
export interface PoolTokenObservation {
  pool: string;
  provider: string;
  activeTokens: number;
  sessionTokens?: Record<string, number>;
  at: string;
}

export function mergePoolTokenObservations(
  prior: PoolTokenObservation[],
  fresh: PoolTokenObservation[],
  nowMs: number,
): PoolTokenObservation[] {
  const byKey = new Map<string, PoolTokenObservation>();
  for (const o of [...prior, ...fresh]) {
    if (!o || typeof o.pool !== "string") continue;
    if (!Number.isFinite(o.activeTokens) || o.activeTokens < 0) continue;
    const t = Date.parse(o.at);
    if (!Number.isFinite(t) || nowMs - t > OBSERVATION_RETENTION_MS) continue;
    const clean: PoolTokenObservation = { ...o };
    if (o.sessionTokens && typeof o.sessionTokens === "object") {
      const counters: Record<string, number> = {};
      for (const [id, tokens] of Object.entries(o.sessionTokens)) {
        if (typeof tokens === "number" && Number.isFinite(tokens) && tokens >= 0) counters[id] = tokens;
      }
      clean.sessionTokens = counters;
    } else {
      delete clean.sessionTokens;
    }
    byKey.set(`${o.pool}${o.at}`, clean);
  }
  const groups = new Map<string, PoolTokenObservation[]>();
  for (const o of byKey.values()) {
    const list = groups.get(o.pool) ?? [];
    list.push(o);
    groups.set(o.pool, list);
  }
  const out: PoolTokenObservation[] = [];
  for (const list of groups.values()) {
    list.sort((a, b) => Date.parse(a.at) - Date.parse(b.at));
    out.push(...list.slice(-MAX_OBSERVATIONS_PER_WINDOW));
  }
  out.sort((a, b) => Date.parse(a.at) - Date.parse(b.at) || a.pool.localeCompare(b.pool));
  return out;
}

/** Measured tokens/min from pool token observations: the delta
 * between the two most recent observations, at most
 * RATE_WINDOW_MS apart, computed from MATCHING session counters
 * only — the sessions present in BOTH observations, each
 * contributing its own counter delta (a counter that went down
 * was reset or replaced and contributes nothing measurable).
 * Aggregate totals are never diffed: the active set changes
 * between collections, so a totals delta counts a resumed
 * session's whole history as new burn and subtracts an ended
 * session's tokens from it. No comparable pair — either side
 * missing its per-session breakdown, no matching session, or
 * every matching counter reset — yields no rate for that pool.
 * When nowMs (the collection instant) is supplied, the pair must
 * also be CURRENT: a newest observation older than
 * RATE_WINDOW_MS relative to nowMs yields no rate, so an idle
 * pool or an unreachable server stops publishing its last
 * measured throughput instead of carrying it for the retention
 * period. */
export function deriveMeasuredRatesFromPoolTokens(
  observations: PoolTokenObservation[],
  nowMs?: number,
): Map<string, number> {
  const groups = new Map<string, PoolTokenObservation[]>();
  for (const o of observations) {
    const list = groups.get(o.pool) ?? [];
    list.push(o);
    groups.set(o.pool, list);
  }
  const rates = new Map<string, number>();
  for (const [pool, series] of groups) {
    const sorted = [...series].sort((a, b) => Date.parse(a.at) - Date.parse(b.at));
    const prev = sorted[sorted.length - 2];
    const last = sorted[sorted.length - 1];
    if (!prev || !last || prev === last) continue;
    if (nowMs !== undefined && Number.isFinite(nowMs) && nowMs - Date.parse(last.at) > RATE_WINDOW_MS) continue;
    const dtMs = Date.parse(last.at) - Date.parse(prev.at);
    if (!(dtMs > 0 && dtMs <= RATE_WINDOW_MS)) continue;
    if (!prev.sessionTokens || !last.sessionTokens) continue;
    let delta = 0;
    let measured = 0;
    for (const [id, before] of Object.entries(prev.sessionTokens)) {
      const after = last.sessionTokens[id];
      if (after === undefined) continue;
      const diff = after - before;
      if (diff < 0) continue; // counter reset/replaced: not measurable
      delta += diff;
      measured += 1;
    }
    if (measured === 0) continue;
    rates.set(pool, delta / (dtMs / 60000));
  }
  return rates;
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
