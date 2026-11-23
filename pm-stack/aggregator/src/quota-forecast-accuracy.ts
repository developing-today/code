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
