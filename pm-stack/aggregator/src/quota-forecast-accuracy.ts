// Forecast accuracy ledger + backtest harness for the Stage-3 quota
// plane (issue #12696, slice S2 of epic #12694; design
// doc/2026-11-22T22-35-00Z_design_quota-plane-stage-3 §2/§3/§7).
//
// What this scores: the v1 forecast model — quota-analytics.ts's
// projectWindow pace projection, unchanged (design D2) — as S1
// (#12695) publishes it per gatherer tick in quota-forecast.json and
// in the forecast ledger rows. The backtest replays the aggregator
// snapshot store (the append-only §4 store, usage-YYYY-MM.jsonl)
// through the same series assembly the product uses, issues the
// forecast the product would have issued at each reading, and scores
// it against the realized outcome of its reset period: did the
// window wall before its reset, and how far the projected wall
// instant was from the realized one.
//
// The accuracy ledger is the baselines object below
// (ForecastAccuracyLedger): per pool/window calibration in exactly
// the terms the weekly review (S7, design §7) reads —
// projected-vs-realized wall error, wall-before-reset precision and
// recall, first-call lead. The backtest produces the initial
// baselines from the store's history; the same scoring entry points
// score forecast-vs-outcome pairs as live ticks accumulate them.
//
// Honesty rules (mirroring quota-analytics.ts and design §2):
// - A pool/window the data cannot score is "unscoreable" with named
//   reasons — never accurate, never zero. Undefined precision or
//   recall (no positive predictions / no realized walls) stays null
//   with the reason recorded beside it.
// - The realized wall instant is the FIRST reading at/over 100%
//   before the reset: an upper bound on the true wall instant (the
//   wall happened between the previous reading and this one), so
//   signed wall errors are conservative in the late direction.
// - Forecasts are issued at every reading, as the product issues at
//   every tick; consecutive forecasts inside one period are
//   correlated, and the counts say so — the calibration describes
//   the product as published, not a thinned sample of it.
// - Readings already at/over 100% are detections, not predictions:
//   counted separately, never scored as forecasts.
// - A period's outcome is scored when it is realized: its reset has
//   passed, or it has already walled (a wall before the reset is a
//   realized outcome however far ahead the reset is). A period
//   neither walled nor reset is open and unscored. Nothing after
//   asOf is ever read — a replay with an earlier --as-of cannot see
//   the future.
// - The store span is 33 days: enough for same-period pace
//   calibration, too thin for per-weekday/hourly profiles (~4–5
//   samples of each weekday per window). This harness computes no
//   profile statistic, by design (§2; §8 alternative B).
//
// Pure functions over plain values — no I/O, no clock reads except
// through arguments. The CLI is scripts/quota-forecast-backtest.ts;
// the store loading there is SnapshotStore.readAll (read-only).

import {
  PROJECTION_PERIOD_TOLERANCE_MS,
  projectWindow,
  type PctPoint,
  type ProjectionState,
} from "./quota-analytics.ts";
import { forecastSeriesKey } from "./quota-forecast.ts";
import type { SnapshotRecord } from "./model.ts";

/** The model under score, named so a ledger is never read as scoring
 * some other forecaster: S1's product is projectWindow verbatim (D2). */
export const FORECAST_ACCURACY_MODEL = "quota-analytics.projectWindow pace v1 (design D2, via S1 #12695)";

/**
 * The serviceId fold is S1's own forecastSeriesKey, imported so the
 * backtest's join and the forecast product's join are the same
 * function by construction, never two copies that can drift: a
 * leading underscore (the #1823 config-id rename), the colon/dot
 * drift, and the hyphen drift all fold away. Distinct pools stay
 * distinct — the fold removes spelling, never structure. (The test
 * suite pins the fold table.)
 */
export const forecastAccuracySeriesKey = forecastSeriesKey;

/**
 * The explicit rename-join (design §2: "any backtest must join
 * across renames with an explicit mapping — slice S2 carries it as
 * code, not prose"). Raw store serviceId -> the pool it is a renamed
 * spelling of, for every rename the bare fold does NOT already join.
 * Each entry names its evidence. Renames the fold does join are not
 * repeated here: antigravity/_antigravity, command-code/commandcode/
 * _commandcode, meta:high-usage (a sub-id of meta under the fold),
 * openai.main/_openai.main, cline-pass:cline-pass (sub-id),
 * opendesign:go (sub-id), opencode-go:personal/:lab (sub-ids of the
 * bare opencode-go pool).
 */
export const SERVICE_ID_RENAME_ALIASES: Record<string, string> = {
  // The alternate ChatGPT account's gatherer-era spelling; the
  // pool's canonical id is _openai.alt (quota-current provider
  // _openai.alt, POOL_SPECS). The fold alone leaves it stranded as
  // "openai.alternate", a prefix of nothing.
  "openai.alternate": "_openai.alt",
  // Bare spellings are the main ChatGPT account's earlier writer
  // generations (recordsFromProviderUsage writes _openai.main); the
  // alternate account always carried an alt marker, never a bare id.
  openai: "_openai.main",
  _openai: "_openai.main",
  // Colon spellings of the two Antigravity door ids (POOL_SPECS
  // _antigravity.gemini / _antigravity.claude-gpt).
  "antigravity:gemini": "_antigravity.gemini",
  "antigravity:claude-gpt": "_antigravity.claude-gpt",
  // The session-provider spelling of the OpenCode Go workspaces:
  // quota-current carries providerId opencode.<ws> over serviceId
  // opencode-go.<ws>, and a few store records were written under the
  // provider spelling.
  "opencode.0x111dev": "opencode-go.0x111dev",
  "opencode.drewrypope": "opencode-go.drewrypope",
  "opencode.popesdrew": "opencode-go.popesdrew",
  "opencode.developing-today-0": "opencode-go.developing-today-0",
};

export interface ForecastPoolDef {
  /** Pool service id in the aggregator's spelling (POOL_SPECS,
   * quota-extract.ts) — the identity S1's forecast rows carry. */
  serviceId: string;
  /** Canonical provider id sessions carry (the quota-current /
   * forecast keying, design D1); null where no session provider
   * exists for the pool. */
  providerId: string | null;
}

/**
 * The scored pool register: the aggregator's pool set (POOL_SPECS)
 * plus the bare opencode-go service id, which is a live quota-current
 * pool S1 forecasts (its series accepts every workspace's records as
 * sub-ids, exactly as the product assembles them). Pools with no
 * percent windows in the store (deepseek) still appear in the ledger
 * — as unscoreable, with the reason — so the register's coverage is
 * itself reported, never silently partial.
 */
export const FORECAST_POOLS: ForecastPoolDef[] = [
  { serviceId: "meta", providerId: "_muse" },
  { serviceId: "anthropic:claude-pro", providerId: "_claude" },
  { serviceId: "_openai.main", providerId: "_openai.main" },
  { serviceId: "_openai.alt", providerId: "_openai.alt" },
  { serviceId: "opencode-go", providerId: "opencode-go" },
  { serviceId: "opencode-go.drewrypope", providerId: "opencode-go.drewrypope" },
  { serviceId: "opencode-go.popesdrew", providerId: "opencode-go.popesdrew" },
  { serviceId: "opencode-go.developing-today-0", providerId: "opencode-go.developing-today-0" },
  { serviceId: "opencode-go.0x111dev", providerId: "opencode-go.0x111dev" },
  { serviceId: "cline-pass", providerId: "cline-pass" },
  { serviceId: "_commandcode", providerId: "_commandcode" },
  { serviceId: "_antigravity.gemini", providerId: "_antigravity" },
  { serviceId: "_antigravity.claude-gpt", providerId: "_antigravity" },
  { serviceId: "opendesign", providerId: "opendesign" },
  { serviceId: "deepseek", providerId: "deepseek" },
  { serviceId: "mistral", providerId: "mistral" },
  { serviceId: "openrouter", providerId: "openrouter" },
];

/**
 * Canonical window -> the snapshot metric ids that spell it. Mirrors
 * WINDOW_METRIC_ALIASES in analytics-store.ts / quota-forecast.ts
 * (keyed there by view names): "weekly" is the canonical name here;
 * opendesign's quota-current spells the same window "7d" and the
 * store writes both spellings — one window, one baseline.
 */
export const FORECAST_WINDOW_METRICS: Record<string, string[]> = {
  "5h": ["5h", "5-hour"],
  weekly: ["weekly", "7d", "7-day"],
  monthly: ["monthly", "monthly credits"],
  api: ["api"],
  vibe: ["vibe"],
};

export const FORECAST_WINDOWS = Object.keys(FORECAST_WINDOW_METRICS);

/** Index percent records by folded series key, applying the rename
 * aliases first (an aliased record indexes under its pool's key, not
 * its own). Also returns, per key, the raw service ids folded in —
 * the ledger's provenance for the join. */
export function indexPercentRecords(records: readonly SnapshotRecord[]): {
  index: Map<string, SnapshotRecord[]>;
  joinedIds: Map<string, string[]>;
} {
  const index = new Map<string, SnapshotRecord[]>();
  const joined = new Map<string, Set<string>>();
  for (const r of records) {
    if (typeof r.value !== "number" || r.unit !== "pct") continue;
    const target = SERVICE_ID_RENAME_ALIASES[r.serviceId] ?? r.serviceId;
    const key = forecastAccuracySeriesKey(target);
    const list = index.get(key) ?? [];
    list.push(r);
    index.set(key, list);
    const ids = joined.get(key) ?? new Set<string>();
    ids.add(r.serviceId);
    joined.set(key, ids);
  }
  const joinedIds = new Map([...joined].map(([k, v]) => [k, [...v].sort()]));
  return { index, joinedIds };
}

/** Percent points for one pool window from the indexed history:
 * the product's own assembly (folded key + sub-id acceptance, the
 * alias metric set + ":window" suffix matching, (ts, value)
 * dedupe), identical to quota-forecast.ts's pointsForWindow so the
 * backtest scores the series the forecast is actually fit on. */
export function pointsForWindow(
  index: Map<string, SnapshotRecord[]>,
  poolServiceId: string,
  window: string,
): PctPoint[] {
  const poolKey = forecastAccuracySeriesKey(poolServiceId);
  const aliases = FORECAST_WINDOW_METRICS[window] ?? [window];
  const metrics = new Set(aliases);
  const points: PctPoint[] = [];
  const seen = new Set<string>();
  for (const [key, records] of index) {
    if (key !== poolKey && !key.startsWith(`${poolKey}.`)) continue;
    for (const r of records) {
      if (!metrics.has(r.metricId) && !aliases.some((a) => r.metricId.endsWith(`:${a}`))) continue;
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

export interface ResetPeriod {
  /** The period's reset instant (ms epoch); null when no reading in
   * the period carries one — such a period's outcome is unknowable
   * and it is never scored. */
  resetMs: number | null;
  resetIso: string | null;
  points: PctPoint[];
}

/**
 * Segment a window series into reset periods. Two reported resets
 * within PROJECTION_PERIOD_TOLERANCE_MS are the same period (the
 * tolerance's own rationale in quota-analytics.ts: one period's
 * reported reset jitters by seconds, adjacent periods differ by at
 * least the shortest window). A reading with no resetsAt joins the
 * period open at its instant, or a null-reset period when none is.
 */
export function segmentPeriods(points: readonly PctPoint[]): ResetPeriod[] {
  const periods: ResetPeriod[] = [];
  let current: ResetPeriod | null = null;
  for (const p of points) {
    const pReset = p.resetsAt ? Date.parse(p.resetsAt) : NaN;
    if (Number.isFinite(pReset)) {
      if (
        current === null ||
        current.resetMs === null ||
        Math.abs(pReset - current.resetMs) > PROJECTION_PERIOD_TOLERANCE_MS
      ) {
        current = { resetMs: pReset, resetIso: new Date(pReset).toISOString(), points: [] };
        periods.push(current);
      }
      current.points.push(p);
      continue;
    }
    if (current !== null && (current.resetMs === null || p.ts < current.resetMs)) {
      current.points.push(p);
      continue;
    }
    current = { resetMs: null, resetIso: null, points: [p] };
    periods.push(current);
  }
  return periods;
}

export type ForecastOutcome = "tp" | "fp" | "fn" | "tn";

/** One scored forecast: the accumulation unit of the running
 * accuracy ledger (the backtest emits these as facts; live scoring
 * appends the same shape as ticks accumulate). */
export interface ScoredForecast {
  pool: string;
  providerId: string | null;
  window: string;
  issuedAt: string;
  resetsAt: string;
  state: ProjectionState;
  projectedWallAt: string | null;
  wallsBeforeReset: boolean;
  realizedWallAt: string | null;
  outcome: ForecastOutcome;
  /** projected − realized wall, minutes; TP only, null otherwise. */
  errorMin: number | null;
}

export interface WindowScoring {
  facts: ScoredForecast[];
  /** Completed periods replayed for this window. */
  periodsScored: number;
  /** Completed periods whose window walled before its reset. */
  periodsWalled: number;
  /** Periods whose reset is still in the future at asOf AND which
   * have not walled: outcome not yet realized, excluded from
   * scoring. (A period that already walled is scored — its outcome
   * is realized however far ahead the reset is.) */
  periodsOpen: number;
  /** Periods with no usable reset: outcome unknowable. */
  periodsNoReset: number;
  forecastsIssued: number;
  forecastsInsufficient: number;
  /** Readings already at/over 100%: detections, not predictions. */
  detectionsWhileWalled: number;
  /** Signed wall errors (minutes) of the true positives. */
  wallErrorsMin: number[];
  /** Per walled period, minutes between its first true-positive
   * forecast and the realized wall (the §2 caution-margin input). */
  firstCallLeadsMin: number[];
}

/**
 * Replay one window series: issue the product's forecast at every
 * reading of every completed period and score it against the
 * period's realized outcome. The oracle is projectWindow itself —
 * this function fits nothing.
 */
export function scoreWindowSeries(args: {
  pool: ForecastPoolDef;
  window: string;
  points: readonly PctPoint[];
  asOfMs: number;
}): WindowScoring {
  const { pool, window, asOfMs } = args;
  const points = [...args.points].filter((p) => p.ts <= asOfMs).sort((a, b) => a.ts - b.ts);
  const out: WindowScoring = {
    facts: [],
    periodsScored: 0,
    periodsWalled: 0,
    periodsOpen: 0,
    periodsNoReset: 0,
    forecastsIssued: 0,
    forecastsInsufficient: 0,
    detectionsWhileWalled: 0,
    wallErrorsMin: [],
    firstCallLeadsMin: [],
  };
  for (const period of segmentPeriods(points)) {
    if (period.resetMs === null || period.resetIso === null) {
      out.periodsNoReset++;
      continue;
    }
    const walledPoint = period.points.find((p) => p.pct >= 100 && p.ts < (period.resetMs as number));
    const walledAt = walledPoint ? walledPoint.ts : null;
    if (period.resetMs > asOfMs && walledAt === null) {
      // Neither walled nor reset: the outcome is not yet realized.
      out.periodsOpen++;
      continue;
    }
    out.periodsScored++;
    if (walledAt !== null) out.periodsWalled++;
    let firstCallAt: number | null = null;
    for (const p of period.points) {
      if (p.ts >= period.resetMs) continue; // post-reset straggler: not a forecast of this period
      out.forecastsIssued++;
      if (p.pct >= 100) {
        out.detectionsWhileWalled++;
        continue;
      }
      const proj = projectWindow({ points, pctNow: p.pct, resetsAt: period.resetIso, now: p.ts });
      if (proj.state === "insufficient-history" || proj.wallsBeforeReset === null) {
        out.forecastsInsufficient++;
        continue;
      }
      const predicted = proj.wallsBeforeReset;
      const outcome: ForecastOutcome = predicted ? (walledAt !== null ? "tp" : "fp") : walledAt !== null ? "fn" : "tn";
      const errorMin =
        outcome === "tp" && proj.projectedWallAt !== null && walledAt !== null
          ? (proj.projectedWallAt - walledAt) / 60_000
          : null;
      if (errorMin !== null) out.wallErrorsMin.push(errorMin);
      if (outcome === "tp" && firstCallAt === null) firstCallAt = p.ts;
      out.facts.push({
        pool: pool.serviceId,
        providerId: pool.providerId,
        window,
        issuedAt: new Date(p.ts).toISOString(),
        resetsAt: period.resetIso,
        state: proj.state,
        projectedWallAt: proj.projectedWallAt !== null ? new Date(proj.projectedWallAt).toISOString() : null,
        wallsBeforeReset: predicted,
        realizedWallAt: walledAt !== null ? new Date(walledAt).toISOString() : null,
        outcome,
        errorMin,
      });
    }
    if (walledAt !== null && firstCallAt !== null) {
      out.firstCallLeadsMin.push((walledAt - firstCallAt) / 60_000);
    }
  }
  return out;
}

export interface ErrorDistribution {
  count: number;
  meanSignedMin: number;
  medianSignedMin: number;
  medianAbsMin: number;
  p90AbsMin: number;
}

function quantile(sorted: number[], q: number): number {
  if (sorted.length === 1) return sorted[0] as number;
  const pos = (sorted.length - 1) * q;
  const base = Math.floor(pos);
  const rest = pos - base;
  const next = sorted[base + 1];
  return next !== undefined
    ? (sorted[base] as number) + rest * (next - (sorted[base] as number))
    : (sorted[base] as number);
}

export function errorDistribution(errorsMin: readonly number[]): ErrorDistribution | null {
  if (!errorsMin.length) return null;
  const sorted = [...errorsMin].sort((a, b) => a - b);
  const abs = errorsMin.map(Math.abs).sort((a, b) => a - b);
  const round = (n: number) => Math.round(n * 10) / 10;
  return {
    count: sorted.length,
    meanSignedMin: round(sorted.reduce((s, n) => s + n, 0) / sorted.length),
    medianSignedMin: round(quantile(sorted, 0.5)),
    medianAbsMin: round(quantile(abs, 0.5)),
    p90AbsMin: round(quantile(abs, 0.9)),
  };
}

export interface WindowBaseline {
  window: string;
  scoreable: boolean;
  /** Why not, when scoreable is false; plus why any single metric
   * is null on a scoreable window. Empty when fully scored. */
  unscoreableReasons: string[];
  periodsTotal: number;
  periodsScored: number;
  periodsWalled: number;
  periodsOpen: number;
  periodsNoReset: number;
  forecastsIssued: number;
  forecastsScored: number;
  forecastsInsufficient: number;
  detectionsWhileWalled: number;
  tp: number;
  fp: number;
  fn: number;
  tn: number;
  /** Null when no positive prediction was scored (undefined, not 0). */
  precision: number | null;
  /** Null when no realized wall was scored (undefined, not 0). */
  recall: number | null;
  wallErrorMin: ErrorDistribution | null;
  firstCallLeadMin: { count: number; medianMin: number } | null;
}

export interface PoolBaseline {
  pool: string;
  providerId: string | null;
  /** Raw store service ids folded into this pool's series — the
   * rename-join's provenance, per pool. */
  serviceIdsJoined: string[];
  windows: WindowBaseline[];
  /** The pool's windows summed: the per-pool calibration S7 leads
   * with. Same shape as a window baseline minus window identity. */
  rollup: Omit<WindowBaseline, "window">;
}

export interface ForecastAccuracyLedger {
  version: 1;
  kind: "quota-forecast-accuracy";
  generatedAt: string;
  model: string;
  store: {
    records: number;
    pctRecords: number;
    fromIso: string | null;
    toIso: string | null;
  };
  pools: PoolBaseline[];
}

function baselineFromScoring(window: string, pointsCount: number, scoring: WindowScoring): WindowBaseline {
  const tp = scoring.facts.filter((f) => f.outcome === "tp").length;
  const fp = scoring.facts.filter((f) => f.outcome === "fp").length;
  const fn = scoring.facts.filter((f) => f.outcome === "fn").length;
  const tn = scoring.facts.filter((f) => f.outcome === "tn").length;
  const reasons: string[] = [];
  if (pointsCount === 0) reasons.push("no percent readings for this pool/window in the store");
  if (scoring.periodsScored === 0 && pointsCount > 0) {
    if (scoring.periodsNoReset > 0 && scoring.periodsOpen === 0) {
      reasons.push("no reading carries a resetsAt — reset periods cannot be delimited, so no outcome is knowable");
    } else if (scoring.periodsOpen > 0) {
      reasons.push("every reset period is still open at the store's last reading — no realized outcome yet");
    } else {
      reasons.push("no completed reset period in the store span");
    }
  }
  const scored = tp + fp + fn + tn;
  if (scoring.periodsScored > 0 && scored === 0) {
    reasons.push(
      "every issued forecast was insufficient-history — the pace was never measurable in a completed period",
    );
  }
  const precision = tp + fp > 0 ? Math.round((tp / (tp + fp)) * 1000) / 1000 : null;
  const recall = tp + fn > 0 ? Math.round((tp / (tp + fn)) * 1000) / 1000 : null;
  if (scored > 0 && precision === null) reasons.push("precision undefined: no positive predictions were scored");
  if (scored > 0 && recall === null) reasons.push("recall undefined: no realized wall was scored");
  const leads = [...scoring.firstCallLeadsMin].sort((a, b) => a - b);
  return {
    window,
    scoreable: scored > 0,
    unscoreableReasons: reasons,
    periodsTotal: scoring.periodsScored + scoring.periodsOpen + scoring.periodsNoReset,
    periodsScored: scoring.periodsScored,
    periodsWalled: scoring.periodsWalled,
    periodsOpen: scoring.periodsOpen,
    periodsNoReset: scoring.periodsNoReset,
    forecastsIssued: scoring.forecastsIssued,
    forecastsScored: scored,
    forecastsInsufficient: scoring.forecastsInsufficient,
    detectionsWhileWalled: scoring.detectionsWhileWalled,
    tp,
    fp,
    fn,
    tn,
    precision,
    recall,
    wallErrorMin: errorDistribution(scoring.wallErrorsMin),
    firstCallLeadMin: leads.length
      ? { count: leads.length, medianMin: Math.round(quantile(leads, 0.5) * 10) / 10 }
      : null,
  };
}

function rollupBaselines(windows: WindowBaseline[]): Omit<WindowBaseline, "window"> {
  const sum = (pick: (w: WindowBaseline) => number) => windows.reduce((s, w) => s + pick(w), 0);
  const tp = sum((w) => w.tp);
  const fp = sum((w) => w.fp);
  const fn = sum((w) => w.fn);
  const reasons: string[] = [];
  if (!windows.length) reasons.push("no percent windows for this pool in the store");
  if (windows.length && windows.every((w) => !w.scoreable)) {
    reasons.push("no window of this pool is scoreable over the store span");
  }
  const precision = tp + fp > 0 ? Math.round((tp / (tp + fp)) * 1000) / 1000 : null;
  const recall = tp + fn > 0 ? Math.round((tp / (tp + fn)) * 1000) / 1000 : null;
  if (tp + fp + fn + sum((w) => w.tn) > 0 && precision === null) {
    reasons.push("precision undefined: no positive predictions were scored");
  }
  if (tp + fp + fn + sum((w) => w.tn) > 0 && recall === null) {
    reasons.push("recall undefined: no realized wall was scored");
  }
  return {
    scoreable: windows.some((w) => w.scoreable),
    unscoreableReasons: reasons,
    periodsTotal: sum((w) => w.periodsTotal),
    periodsScored: sum((w) => w.periodsScored),
    periodsWalled: sum((w) => w.periodsWalled),
    periodsOpen: sum((w) => w.periodsOpen),
    periodsNoReset: sum((w) => w.periodsNoReset),
    forecastsIssued: sum((w) => w.forecastsIssued),
    forecastsScored: sum((w) => w.forecastsScored),
    forecastsInsufficient: sum((w) => w.forecastsInsufficient),
    detectionsWhileWalled: sum((w) => w.detectionsWhileWalled),
    tp,
    fp,
    fn,
    tn: sum((w) => w.tn),
    precision,
    recall,
    // The rollup does not re-derive error distributions from window
    // summaries (a median of medians is not a median); S7 reads
    // per-window distributions and the pooled facts for pool-level
    // error work. Null here is that statement, not missing data.
    wallErrorMin: null,
    firstCallLeadMin: null,
  };
}

/**
 * Run the backtest over snapshot records and build the accuracy
 * ledger + the scored-forecast facts. asOf defaults to the store's
 * last reading (never the wall clock) so a run over a fixed store
 * copy is reproducible byte-for-byte.
 */
export function buildForecastAccuracyLedger(
  records: readonly SnapshotRecord[],
  opts: { asOfMs?: number; generatedAt?: string } = {},
): { ledger: ForecastAccuracyLedger; facts: ScoredForecast[] } {
  const pct = records.filter((r) => typeof r.value === "number" && r.unit === "pct");
  const stamps = records.map((r) => Date.parse(r.ts)).filter(Number.isFinite);
  const toMs = stamps.length ? Math.max(...stamps) : 0;
  const fromMs = stamps.length ? Math.min(...stamps) : 0;
  const asOfMs = opts.asOfMs ?? toMs;
  const { index, joinedIds } = indexPercentRecords(records);
  const facts: ScoredForecast[] = [];
  const pools: PoolBaseline[] = [];
  for (const pool of FORECAST_POOLS) {
    const key = forecastAccuracySeriesKey(pool.serviceId);
    const windows: WindowBaseline[] = [];
    for (const window of FORECAST_WINDOWS) {
      const points = pointsForWindow(index, pool.serviceId, window);
      // A window with no readings gets no baseline row: the pool
      // rollup states "no percent windows" once, instead of every
      // pool carrying five empty windows.
      if (!points.length) continue;
      const scoring = scoreWindowSeries({ pool, window, points, asOfMs });
      facts.push(...scoring.facts);
      windows.push(baselineFromScoring(window, points.length, scoring));
    }
    pools.push({
      pool: pool.serviceId,
      providerId: pool.providerId,
      serviceIdsJoined: joinedIds.get(key) ?? [],
      windows,
      rollup: rollupBaselines(windows),
    });
  }
  facts.sort((a, b) => a.issuedAt.localeCompare(b.issuedAt) || a.pool.localeCompare(b.pool));
  return {
    ledger: {
      version: 1,
      kind: "quota-forecast-accuracy",
      generatedAt: opts.generatedAt ?? new Date(asOfMs).toISOString(),
      model: FORECAST_ACCURACY_MODEL,
      store: {
        records: records.length,
        pctRecords: pct.length,
        fromIso: stamps.length ? new Date(fromMs).toISOString() : null,
        toIso: stamps.length ? new Date(toMs).toISOString() : null,
      },
      pools,
    },
    facts,
  };
}

function fmtMetric(n: number | null): string {
  return n === null ? "—" : n.toFixed(3).replace(/0+$/, "").replace(/\.$/, ".0");
}

function fmtErr(d: ErrorDistribution | null): string {
  if (!d) return "—";
  return `n=${d.count}, median ${d.medianSignedMin}m, |median| ${d.medianAbsMin}m, p90|e| ${d.p90AbsMin}m`;
}

/** The human backtest report: the ledger's numbers with the method
 * and the limits stated where a reader meets them. */
export function renderBacktestReport(ledger: ForecastAccuracyLedger): string {
  const lines: string[] = [];
  lines.push("# Quota forecast backtest — Stage-3 S2 (#12696)");
  lines.push("");
  lines.push(`- Model under score: ${ledger.model}`);
  lines.push(
    `- Store: ${ledger.store.records} records (${ledger.store.pctRecords} percent readings), ${ledger.store.fromIso ?? "?"} → ${ledger.store.toIso ?? "?"}`,
  );
  lines.push(
    "- Method: replay — at every reading of every completed reset period, issue the forecast the product would have issued (projectWindow over the rename-joined same-period series), then score wall-before-reset against the period's realized outcome. Realized wall = first reading at/over 100% before the reset (an upper bound on the true wall instant). Readings already at/over 100% count as detections, not predictions.",
  );
  lines.push(
    "- Rename-join: explicit in code (SERVICE_ID_RENAME_ALIASES + the series fold) in src/quota-forecast-accuracy.ts; per-pool joined ids are listed in the ledger JSON.",
  );
  lines.push("");
  lines.push("## Per-pool calibration (rollup over windows)");
  lines.push("");
  lines.push(
    "| pool | scoreable | periods (scored/walled) | forecasts (scored/insufficient) | precision | recall | wall error (projected − realized) |",
  );
  lines.push("|---|---|---|---|---|---|---|");
  for (const p of ledger.pools) {
    const r = p.rollup;
    const errCell = r.tp > 0 ? `${r.tp} TP (distributions per window below)` : "—";
    lines.push(
      `| ${p.pool} | ${r.scoreable ? "yes" : "NO"} | ${r.periodsScored}/${r.periodsWalled} | ${r.forecastsScored}/${r.forecastsInsufficient} | ${fmtMetric(r.precision)} | ${fmtMetric(r.recall)} | ${errCell} |`,
    );
  }
  lines.push("");
  lines.push("## Per-window detail");
  lines.push("");
  for (const p of ledger.pools) {
    lines.push(`### ${p.pool}${p.providerId ? ` (provider ${p.providerId})` : ""}`);
    lines.push("");
    if (p.serviceIdsJoined.length) {
      lines.push(`Joined service ids: ${p.serviceIdsJoined.map((s) => `\`${s}\``).join(", ")}`);
      lines.push("");
    }
    if (!p.rollup.scoreable) {
      for (const reason of p.rollup.unscoreableReasons) lines.push(`- note: ${reason}`);
      if (p.rollup.unscoreableReasons.length) lines.push("");
    }
    for (const w of p.windows) {
      if (w.periodsTotal === 0 && w.forecastsIssued === 0 && !w.unscoreableReasons.length) continue;
      lines.push(
        `- **${w.window}** — scoreable: ${w.scoreable ? "yes" : "NO"}; periods scored ${w.periodsScored} (walled ${w.periodsWalled}, open ${w.periodsOpen}, no-reset ${w.periodsNoReset}); forecasts scored ${w.forecastsScored} (TP ${w.tp} / FP ${w.fp} / FN ${w.fn} / TN ${w.tn}), insufficient ${w.forecastsInsufficient}, detections ${w.detectionsWhileWalled}; precision ${fmtMetric(w.precision)}, recall ${fmtMetric(w.recall)}; wall error ${fmtErr(w.wallErrorMin)}; first-call lead ${
          w.firstCallLeadMin
            ? `median ${w.firstCallLeadMin.medianMin}m over ${w.firstCallLeadMin.count} walled periods`
            : "—"
        }`,
      );
      for (const reason of w.unscoreableReasons) lines.push(`  - note: ${reason}`);
    }
    lines.push("");
  }
  lines.push("## What this data cannot support");
  lines.push("");
  lines.push(
    "- Per-weekday or per-hour-of-day calibration: the store spans 33 days (~4–5 samples of each weekday per window). The design (§2) rules profile models out at this span, and this backtest computes no profile statistic.",
  );
  lines.push(
    "- Any pool/window marked unscoreable above: the ledger records the reason; an unscoreable window is not an accurate one and not a failed one — it is unmeasured.",
  );
  lines.push(
    "- Wall-error resolution finer than the reading cadence: the realized wall is known only at the first at/over-limit reading, so errors carry the gather cadence's quantization (15 minutes in the gatherer era, a day in the manual era).",
  );
  lines.push("");
  return lines.join("\n");
}

// ---------------------------------------------------------------------------
// downtime_window scoring (C4.1 criterion 2, #13257; design §3 + §7)
//
// Scores the C4 prediction records quota-downtime.ts issues
// (kind "downtime_window", source "quota-prediction", #13295) against
// realized complete-downtime windows. Pure functions over plain
// values — no I/O, no clock reads except through arguments.
//
// Design §3, downtime_window bullet, verbatim:
// - Start error and end error (signed minutes), plus interval
//   overlap (intersection ÷ union of predicted vs realized window).
// - A predicted window that never occurs scores as a false window;
//   a realized complete-downtime with no prediction scores as a
//   miss. Both count, per S2's precision/recall shape.
// - Honesty states (insufficient_history / no_basis — the §1
//   published states, never an invented range) are NOT scored: they
//   are the honest answer, and scoring them would pay for guessing
//   (the model_plan_arrival §3 precedent).
//
// Realized windows come from the same gatherer/pool-state history
// the forecast product already consumes — no second gatherer: the
// caller assembles per-provider walled intervals from the snapshot
// store (pct >= 100 readings, the same realized-wall definition S2
// uses) and joins them with deriveRealizedDowntimeWindows below,
// the realized-side mirror of deriveDowntimeWindowRecords'
// intersection. Open predictions are unscored, never assumed.

/** Minimal structural input: accepts C4PredictionRecord verbatim. */
export interface DowntimePredictionInput {
  id: string;
  state: string;
  prediction: { earliest: string; latest: string } | null;
}

export interface DowntimeRealizedWindow {
  /** ISO-8601 UTC bound of the realized all-walled interval. */
  start: string;
  end: string;
}

export type DowntimeScoreOutcome = "hit" | "false_window" | "miss";

export interface ScoredDowntimeWindow {
  /** The prediction record id for hits and false windows; null for misses. */
  id: string | null;
  outcome: DowntimeScoreOutcome;
  predictedEarliest: string | null;
  predictedLatest: string | null;
  realizedStart: string | null;
  realizedEnd: string | null;
  /** predicted earliest − realized start, signed minutes; hits only. */
  startErrorMin: number | null;
  /** predicted latest − realized end, signed minutes; hits only. */
  endErrorMin: number | null;
  /** Intersection ÷ union of predicted vs realized window; hits only. */
  overlapIoU: number | null;
}

export interface DowntimeWindowScoring {
  facts: ScoredDowntimeWindow[];
  /** Predicted records in a scored state (predicted with a range). */
  predictionsScored: number;
  /** Predicted records in an honesty state: published, never scored. */
  predictionsUnscored: number;
  hits: number;
  falseWindows: number;
  misses: number;
}

const MIN = 60_000;

/** States that publish no range: the honest answer, never scored (§1, §3). */
export function isDowntimeUnscoredState(state: string): boolean {
  return state === "insufficient_history" || state === "no_basis" || state === "adjustment_unavailable";
}

function downtimeOverlapIoU(pStart: number, pEnd: number, rStart: number, rEnd: number): number {
  const intersection = Math.max(0, Math.min(pEnd, rEnd) - Math.max(pStart, rStart));
  const union = pEnd - pStart + (rEnd - rStart) - intersection;
  if (!(union > 0)) return 0;
  return Math.round((intersection / union) * 1000) / 1000;
}

/**
 * Score one predicted window against its realized window: start
 * error and end error (signed minutes) plus interval overlap
 * (intersection ÷ union). Both intervals must be finite with
 * earliest < latest (the §1 refusal of degenerate ranges); NaN or
 * degenerate inputs score 0 overlap rather than throwing, so one
 * bad record cannot poison a ledger rollup.
 */
export function scoreDowntimePair(args: {
  predictedEarliest: string;
  predictedLatest: string;
  realizedStart: string;
  realizedEnd: string;
}): { startErrorMin: number; endErrorMin: number; overlapIoU: number } {
  const pStart = Date.parse(args.predictedEarliest);
  const pEnd = Date.parse(args.predictedLatest);
  const rStart = Date.parse(args.realizedStart);
  const rEnd = Date.parse(args.realizedEnd);
  if (!Number.isFinite(pStart) || !Number.isFinite(pEnd) || !Number.isFinite(rStart) || !Number.isFinite(rEnd)) {
    return { startErrorMin: 0, endErrorMin: 0, overlapIoU: 0 };
  }
  return {
    startErrorMin: Math.round(((pStart - rStart) / MIN) * 10) / 10,
    endErrorMin: Math.round(((pEnd - rEnd) / MIN) * 10) / 10,
    overlapIoU: downtimeOverlapIoU(pStart, pEnd, rStart, rEnd),
  };
}

/**
 * Join predicted downtime_window records to realized windows.
 * Greedy best-overlap match in prediction order: each predicted
 * range takes the unmatched realized window with the highest IoU
 * above 0; a predicted range with no overlapping realized window
 * scores as a false window; a realized window no prediction takes
 * scores as a miss. Honesty-state records (and predicted-state
 * records carrying no range) are counted in predictionsUnscored
 * and never scored.
 */
export function scoreDowntimeWindows(args: {
  predictions: readonly DowntimePredictionInput[];
  realized: readonly DowntimeRealizedWindow[];
}): DowntimeWindowScoring {
  const taken = new Array<boolean>(args.realized.length).fill(false);
  const out: DowntimeWindowScoring = {
    facts: [],
    predictionsScored: 0,
    predictionsUnscored: 0,
    hits: 0,
    falseWindows: 0,
    misses: 0,
  };
  for (const pred of args.predictions) {
    if (isDowntimeUnscoredState(pred.state) || pred.prediction === null) {
      out.predictionsUnscored++;
      continue;
    }
    out.predictionsScored++;
    const pStart = Date.parse(pred.prediction.earliest);
    const pEnd = Date.parse(pred.prediction.latest);
    let best = -1;
    let bestIoU = 0;
    if (Number.isFinite(pStart) && Number.isFinite(pEnd) && pEnd > pStart) {
      for (let i = 0; i < args.realized.length; i++) {
        if (taken[i]) continue;
        const r = args.realized[i] as DowntimeRealizedWindow;
        const rStart = Date.parse(r.start);
        const rEnd = Date.parse(r.end);
        if (!Number.isFinite(rStart) || !Number.isFinite(rEnd) || rEnd <= rStart) continue;
        const iou = downtimeOverlapIoU(pStart, pEnd, rStart, rEnd);
        if (iou > bestIoU) {
          bestIoU = iou;
          best = i;
        }
      }
    }
    if (best < 0) {
      out.falseWindows++;
      out.facts.push({
        id: pred.id,
        outcome: "false_window",
        predictedEarliest: pred.prediction.earliest,
        predictedLatest: pred.prediction.latest,
        realizedStart: null,
        realizedEnd: null,
        startErrorMin: null,
        endErrorMin: null,
        overlapIoU: null,
      });
      continue;
    }
    taken[best] = true;
    const r = args.realized[best] as DowntimeRealizedWindow;
    const pair = scoreDowntimePair({
      predictedEarliest: pred.prediction.earliest,
      predictedLatest: pred.prediction.latest,
      realizedStart: r.start,
      realizedEnd: r.end,
    });
    out.hits++;
    out.facts.push({
      id: pred.id,
      outcome: "hit",
      predictedEarliest: pred.prediction.earliest,
      predictedLatest: pred.prediction.latest,
      realizedStart: r.start,
      realizedEnd: r.end,
      startErrorMin: pair.startErrorMin,
      endErrorMin: pair.endErrorMin,
      overlapIoU: pair.overlapIoU,
    });
  }
  for (let i = 0; i < args.realized.length; i++) {
    if (taken[i]) continue;
    const r = args.realized[i] as DowntimeRealizedWindow;
    out.misses++;
    out.facts.push({
      id: null,
      outcome: "miss",
      predictedEarliest: null,
      predictedLatest: null,
      realizedStart: r.start,
      realizedEnd: r.end,
      startErrorMin: null,
      endErrorMin: null,
      overlapIoU: null,
    });
  }
  return out;
}

export interface DowntimeWindowBaseline {
  scoreable: boolean;
  unscoreableReasons: string[];
  predictionsScored: number;
  predictionsUnscored: number;
  hits: number;
  falseWindows: number;
  misses: number;
  /** Null when no hit was scored (undefined, not 0). */
  precision: number | null;
  /** Null when no realized window was scored (undefined, not 0). */
  recall: number | null;
  startErrorMin: ErrorDistribution | null;
  endErrorMin: ErrorDistribution | null;
  /** Mean intersection-over-union across hits; null with no hits. */
  meanOverlapIoU: number | null;
}

/** Roll up scored downtime facts the way S2 rolls up forecasts. */
export function summarizeDowntimeScoring(scoring: DowntimeWindowScoring): DowntimeWindowBaseline {
  const { hits, falseWindows, misses } = scoring;
  const reasons: string[] = [];
  if (scoring.predictionsScored === 0 && scoring.predictionsUnscored > 0) {
    reasons.push("every predicted record was an honesty state — published, never scored");
  }
  if (hits + falseWindows + misses === 0) {
    reasons.push("no predicted windows and no realized windows in the scored span");
  }
  const precision = hits + falseWindows > 0 ? Math.round((hits / (hits + falseWindows)) * 1000) / 1000 : null;
  const recall = hits + misses > 0 ? Math.round((hits / (hits + misses)) * 1000) / 1000 : null;
  if (hits + falseWindows + misses > 0 && precision === null) {
    reasons.push("precision undefined: no predicted windows were scored");
  }
  if (hits + falseWindows + misses > 0 && recall === null) {
    reasons.push("recall undefined: no realized windows were scored");
  }
  const hitsFacts = scoring.facts.filter((f) => f.outcome === "hit");
  const startErrors = hitsFacts.map((f) => f.startErrorMin).filter((n): n is number => n !== null);
  const endErrors = hitsFacts.map((f) => f.endErrorMin).filter((n): n is number => n !== null);
  const ious = hitsFacts.map((f) => f.overlapIoU).filter((n): n is number => n !== null);
  return {
    scoreable: hits + falseWindows + misses > 0,
    unscoreableReasons: reasons,
    predictionsScored: scoring.predictionsScored,
    predictionsUnscored: scoring.predictionsUnscored,
    hits,
    falseWindows,
    misses,
    precision,
    recall,
    startErrorMin: errorDistribution(startErrors),
    endErrorMin: errorDistribution(endErrors),
    meanOverlapIoU: ious.length ? Math.round((ious.reduce((s, n) => s + n, 0) / ious.length) * 1000) / 1000 : null,
  };
}

/**
 * Realized-side join: maximal intervals where every provider is
 * down. The realized mirror of deriveDowntimeWindowRecords'
 * intersection sweep (quota-downtime.ts §7), over caller-assembled
 * per-provider walled intervals — each provider's down-set is the
 * union of its pools' walled [wall, reset) spans from the same
 * snapshot store the forecast product reads (pct >= 100, the S2
 * realized-wall definition), so no second gatherer is involved.
 * Ends are ms epochs; Infinity (no reported reset) leaves the
 * interval unbounded and unscoreable as a realized window, and is
 * dropped — a realized outcome must be bounded to be scored.
 */
export function deriveRealizedDowntimeWindows(
  perProviderIntervals: readonly (readonly { start: number; end: number }[])[],
): { start: number; end: number }[] {
  if (perProviderIntervals.length === 0) return [];
  if (perProviderIntervals.some((ivs) => ivs.length === 0)) return [];
  const covers = (intervals: readonly { start: number; end: number }[], t: number) =>
    intervals.some((iv) => iv.start <= t && t < iv.end);
  const boundarySet = new Set<number>();
  for (const ivs of perProviderIntervals) {
    for (const iv of ivs) {
      if (!Number.isFinite(iv.start)) continue;
      boundarySet.add(iv.start);
      if (Number.isFinite(iv.end)) boundarySet.add(iv.end);
    }
  }
  const points = [...boundarySet].sort((a, b) => a - b);
  const overlaps: { start: number; end: number }[] = [];
  let runStart: number | null = null;
  for (let i = 0; i < points.length; i++) {
    const t = points[i] as number;
    const allDown = perProviderIntervals.every((ivs) => covers(ivs, t));
    if (allDown) {
      if (runStart === null) runStart = t;
      if (i === points.length - 1 && runStart !== null) {
        const end = Math.min(
          ...perProviderIntervals.map((ivs) =>
            Math.min(...ivs.filter((iv) => iv.start <= t && t < iv.end).map((iv) => iv.end)),
          ),
        );
        if (Number.isFinite(end) && end > runStart) overlaps.push({ start: runStart, end });
        runStart = null;
      }
    } else if (runStart !== null) {
      if (t > runStart) overlaps.push({ start: runStart, end: t });
      runStart = null;
    }
  }
  return overlaps.filter((o) => Number.isFinite(o.end) && o.end > o.start);
}

/** The human downtime-accuracy report: the §3 numbers with the method stated. */
export function renderDowntimeAccuracyReport(args: {
  scoring: DowntimeWindowScoring;
  baseline: DowntimeWindowBaseline;
}): string {
  const { scoring, baseline } = args;
  const lines: string[] = [];
  lines.push("# Downtime-window accuracy — C4.1 criterion 2 (#13257)");
  lines.push("");
  lines.push(
    "- Method: join predicted downtime_window records (kind downtime_window, source quota-prediction, #13295) to realized complete-downtime windows — maximal intervals where every routing pool's pool-state history shows walled (pct >= 100, the S2 realized-wall definition) from the same snapshot store the forecast product reads. Greedy best-overlap match in prediction order; honesty states (insufficient_history / no_basis) published, never scored.",
  );
  lines.push(
    `- Predictions: scored ${scoring.predictionsScored}, honesty-unscored ${scoring.predictionsUnscored}; hits ${scoring.hits}, false windows ${scoring.falseWindows}, misses ${scoring.misses}; precision ${baseline.precision ?? "—"}, recall ${baseline.recall ?? "—"}.`,
  );
  lines.push(
    `- Start error (predicted earliest − realized start): ${fmtErr(baseline.startErrorMin)}; end error (predicted latest − realized end): ${fmtErr(baseline.endErrorMin)}; mean overlap (intersection ÷ union) across hits: ${baseline.meanOverlapIoU ?? "—"}.`,
  );
  for (const reason of baseline.unscoreableReasons) lines.push(`- note: ${reason}`);
  lines.push("");
  return lines.join("\n");
}
