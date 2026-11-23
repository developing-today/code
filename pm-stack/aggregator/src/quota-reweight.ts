// Re-weighting proposal generator for the Stage-3 quota plane
// (issue #12700, slice S6 of epic #12694; design
// doc/2026-11-22T22-35-00Z_design_quota-plane-stage-3 §5, decision
// D5).
//
// D5: the only configuration Stage 3 ever changes, it changes by
// pull request. This module examines the forecast accuracy record
// (S2's ledger, #12696) and, when the evidence supports a durable
// change — a pool weight, a caution margin — produces a proposal:
// the config edit as an exact find/replace pair, the full evidence
// pack (the accuracy rows behind it verbatim, the S1 forecast and
// S3 shadow summaries when supplied), and the expected effect
// stated as a measurable prediction the weekly review (S7) can
// score. This module has no write path at all: it returns plain
// values. The only component that can turn a proposal into a PR is
// quota-reweight-pr.ts, whose command surface is an allowlist with
// no merge on it; merge authority stays where it is (Felix/Drew).
//
// Evidence discipline (the S1/S2 honesty rules, applied to
// proposals):
// - A pool/window the accuracy ledger cannot score never grounds a
//   proposal. The generator emits a named gap instead, carrying
//   S2's unscoreable reasons verbatim — a gap is a report, not a
//   proposal with weaker numbers.
// - Volume gates: a window grounds a proposal only with at least
//   MIN_SCORED_FORECASTS scored forecasts and, where wall timing is
//   the signal, at least MIN_WALL_SAMPLES scored wall errors. Thin
//   per-workspace rows (single-digit scored forecasts today) are
//   named as gaps, not averaged into confidence.
// - The bare `opencode-go` pool is excluded from grounding in
//   every window: S2 showed its rows are assembled from all
//   workspaces' records (the monthly row is the named mixing
//   artifact — precision 0.054 over 290 reset clusters); the
//   per-workspace rows are the real ones. The exclusion is itself
//   reported as a gap so the register's coverage stays visible.
// - Undefined precision/recall (null in the ledger) is never read
//   as 0 or 1: a rule that needs the metric reports the gap with
//   S2's reason.
//
// The two v1 proposal classes:
// - pool-weight: inside one weighted-selection route, shift weight
//   from a member whose binding window walls systematically
//   earlier than the forecast projects (walls are real when called
//   — high precision — but systematically missed/late — low
//   recall, large positive median signed wall error) toward the
//   member with the strongest headroom evidence. Total route
//   weight is preserved. Round-robin routes are out of scope:
//   their "weights" are tier keys, not traffic shares (the config
//   says so itself), so re-weighting them is a different class.
// - caution-margin: when the weekly calibration shows walls
//   arriving systematically early across pools (the S2 headline:
//   linear pace under-calls accelerating weekly burn), propose
//   widening the caution margin NEAR_PCT (derive.ts) by one step.
//   D4 reserves margin changes to this mechanism. The margin is
//   global, so the rule demands breadth: at least two qualifying
//   pools, at least one whose wall calls are trustworthy
//   (precision anchor), and no strong counter-evidence.
//
// Pure functions over plain values — no I/O, no clock reads except
// through arguments. The CLI is scripts/quota-reweight-proposals.ts.

import type { ForecastAccuracyLedger, PoolBaseline, WindowBaseline } from "./quota-forecast-accuracy.ts";
import { FORECAST_POOLS } from "./quota-forecast-accuracy.ts";
import type { QuotaForecastFile } from "./quota-forecast.ts";
import type { ShadowPlacementRecord } from "./quota-shadow-placement.ts";

/** Minimum scored forecasts for a window to ground a proposal. */
export const MIN_SCORED_FORECASTS = 100;
/** Minimum scored wall errors for a wall-timing signal. */
export const MIN_WALL_SAMPLES = 20;
/** Median signed wall error (minutes) at/above which walls count
 * as arriving systematically early: +12h. Sign convention is S2's:
 * projectedWallAt − realizedWallAt, so positive means the wall
 * arrived BEFORE the projection — the forecast ran late. */
export const LATE_WALL_MEDIAN_MIN = 720;
/** A late-wall signal needs wall calls that are real when made. */
export const LATE_WALL_PRECISION_MIN = 0.9;
/** …and walls that are systematically missed or late. */
export const LATE_WALL_RECALL_MAX = 0.7;
/** Headroom evidence: trustworthy calls, no late-wall pattern. */
export const HEADROOM_PRECISION_MIN = 0.9;
export const HEADROOM_RECALL_MIN = 0.85;
/** One margin step, in percentage points, and its ceiling. */
export const MARGIN_STEP_PTS = 5;
export const MARGIN_MAX_PCT = 95;
/** A late-wall member keeps at least this weight after a shift. */
export const WEIGHT_FLOOR = 0.5;
/** Fraction of a late-wall member's weight proposed to move. */
export const WEIGHT_SHIFT_FRACTION = 0.25;

/** Pools whose ledger rows mix several pools' records and so can
 * never ground a proposal (S2's mixing-artifact finding). */
export const AGGREGATE_POOLS_EXCLUDED: ReadonlySet<string> = new Set(["opencode-go"]);

/** One weighted-selection route, as the CLI extracted it from the
 * route config: the weights map plus the exact source line the
 * weights object sits on, so a proposal's edit is a verifiable
 * find/replace against the file as it stands. */
export interface RouteWeightTable {
  routeId: string;
  selection: string;
  weights: Record<string, number>;
  /** Exact text of the line carrying the weights object; null when
   * the object spans lines (the generator then refuses to edit it
   * rather than guess at the file's shape). */
  weightsLine: string | null;
}

export interface EvidenceGap {
  /** What was examined: a route member, a pool/window, a class. */
  scope: string;
  /** Stable machine-readable reason. */
  reason: string;
  /** The reason in prose, with the numbers (or S2's words). */
  detail: string;
}

export interface AccuracyEvidenceRow {
  pool: string;
  window: string;
  scoreable: boolean;
  forecastsScored: number;
  precision: number | null;
  recall: number | null;
  wallErrorMedianSignedMin: number | null;
  wallErrorCount: number | null;
  periodsWalled: number;
  periodsScored: number;
}

export interface ReweightProposal {
  /** Stable dedupe key: class + target + from → to. */
  key: string;
  proposalClass: "pool-weight" | "caution-margin";
  title: string;
  target: {
    description: string;
    /** Repo-relative path the PR edits. */
    file: string;
    /** Exact current text (must occur exactly once in the file). */
    find: string;
    /** The text it becomes. */
    replace: string;
    currentValue: string;
    proposedValue: string;
  };
  rationale: string;
  evidence: {
    ledgerGeneratedAt: string;
    ledgerStore: ForecastAccuracyLedger["store"];
    ledgerModel: string;
    /** The accuracy rows the proposal grounds on, verbatim. */
    rows: AccuracyEvidenceRow[];
    /** One-line summary of the current forecast for the grounding
     * pools, or the named reason no forecast input was supplied. */
    forecastSummary: string;
    /** The S3 shadow diff summary for the change class, or the
     * named reason no shadow record was supplied. */
    shadowSummary: string;
  };
  expectedEffect: {
    /** The prediction, stated so next week's review can score it. */
    statement: string;
    /** The measurable quantity that decides it. */
    metric: string;
    /** Where the measurement comes from. */
    scoredBy: string;
  };
}

export interface ReweightRun {
  proposals: ReweightProposal[];
  gaps: EvidenceGap[];
  /** Human-readable list of everything examined, in order. */
  examined: string[];
}

/** serviceId (the ledger's pool key) -> providerId (the route
 * config's member key), from S2's own pool register. */
const PROVIDER_BY_POOL: ReadonlyMap<string, string | null> = new Map(
  FORECAST_POOLS.map((p) => [p.serviceId, p.providerId]),
);
/** providerId -> ledger pool baseline, for exact member matches. */
function poolByProvider(ledger: ForecastAccuracyLedger): Map<string, PoolBaseline> {
  const out = new Map<string, PoolBaseline>();
  for (const pool of ledger.pools) {
    const providerId = PROVIDER_BY_POOL.get(pool.pool);
    if (providerId) out.set(providerId, pool);
  }
  return out;
}

function evidenceRow(pool: string, w: WindowBaseline): AccuracyEvidenceRow {
  return {
    pool,
    window: w.window,
    scoreable: w.scoreable,
    forecastsScored: w.forecastsScored,
    precision: w.precision,
    recall: w.recall,
    wallErrorMedianSignedMin: w.wallErrorMin?.medianSignedMin ?? null,
    wallErrorCount: w.wallErrorMin?.count ?? null,
    periodsWalled: w.periodsWalled,
    periodsScored: w.periodsScored,
  };
}

/** The window a pool's evidence is strongest on: among scoreable
 * windows, the one with the most scored forecasts. Null when no
 * window is scoreable. */
function bindingWindow(pool: PoolBaseline): WindowBaseline | null {
  let best: WindowBaseline | null = null;
  for (const w of pool.windows) {
    if (!w.scoreable) continue;
    if (!best || w.forecastsScored > best.forecastsScored) best = w;
  }
  return best;
}

interface GateResult {
  ok: boolean;
  gap?: EvidenceGap;
}

/** The volume/honesty gates every grounding window must pass.
 * Returns the named gap instead of a verdict when it cannot. */
function gateWindow(scope: string, pool: PoolBaseline, w: WindowBaseline | null): GateResult {
  if (AGGREGATE_POOLS_EXCLUDED.has(pool.pool)) {
    return {
      ok: false,
      gap: {
        scope,
        reason: "aggregate-pool-excluded",
        detail:
          `${pool.pool} is assembled from every workspace's records (S2's mixing-artifact ` +
          `finding); its rows never ground a proposal — the per-workspace rows are the real ones.`,
      },
    };
  }
  if (!w) {
    const reasons = pool.windows.flatMap((x) => x.unscoreableReasons);
    return {
      ok: false,
      gap: {
        scope,
        reason: "unscoreable-window",
        detail: `no scoreable window for ${pool.pool}: ${reasons.join("; ") || "no percent windows in the ledger"}`,
      },
    };
  }
  if (!w.scoreable) {
    return {
      ok: false,
      gap: {
        scope,
        reason: "unscoreable-window",
        detail: `${pool.pool}/${w.window} is unscoreable: ${w.unscoreableReasons.join("; ")}`,
      },
    };
  }
  if (w.forecastsScored < MIN_SCORED_FORECASTS) {
    return {
      ok: false,
      gap: {
        scope,
        reason: "insufficient-scored-forecasts",
        detail:
          `${pool.pool}/${w.window} has ${w.forecastsScored} scored forecasts, below the ` +
          `${MIN_SCORED_FORECASTS} a durable config change must ground on.`,
      },
    };
  }
  if (w.precision === null || w.recall === null) {
    return {
      ok: false,
      gap: {
        scope,
        reason: "undefined-accuracy-metric",
        detail: `${pool.pool}/${w.window}: ${w.unscoreableReasons.join("; ") || "precision/recall undefined"}`,
      },
    };
  }
  return { ok: true };
}

function isLateWall(w: WindowBaseline): boolean {
  return (
    w.precision !== null &&
    w.precision >= LATE_WALL_PRECISION_MIN &&
    w.recall !== null &&
    w.recall <= LATE_WALL_RECALL_MAX &&
    w.wallErrorMin !== null &&
    w.wallErrorMin.count >= MIN_WALL_SAMPLES &&
    w.wallErrorMin.medianSignedMin >= LATE_WALL_MEDIAN_MIN
  );
}

function isHeadroom(w: WindowBaseline): boolean {
  if (w.precision === null || w.precision < HEADROOM_PRECISION_MIN) return false;
  if (w.recall !== null && w.recall < HEADROOM_RECALL_MIN) return false;
  if (w.wallErrorMin !== null && w.wallErrorMin.medianSignedMin >= LATE_WALL_MEDIAN_MIN) return false;
  return true;
}

function forecastSummaryFor(
  forecast: QuotaForecastFile | null,
  serviceIds: readonly string[],
): string {
  if (!forecast) {
    return "no forecast file supplied to this run (the accuracy ledger alone grounds the proposal; the live forecast's current projections join the pack when the generator runs against the aggregator state dir)";
  }
  const parts: string[] = [];
  for (const rows of Object.values(forecast.pools)) {
    for (const pool of rows) {
      if (!serviceIds.includes(pool.serviceId)) continue;
      for (const w of pool.windows) {
        parts.push(
          `${pool.serviceId}/${w.name}: state ${w.state}, wallsBeforeReset ${w.wallsBeforeReset ?? "unknown"}` +
            (w.pctNow !== null ? `, at ${w.pctNow}%` : ""),
        );
      }
    }
  }
  return parts.length
    ? `forecast ${forecast.generatedAt}: ${parts.join("; ")}`
    : `forecast ${forecast.generatedAt}: no rows for the grounding pools`;
}

function shadowSummaryFor(
  shadow: ShadowPlacementRecord | null,
  changeClass: string,
  nearPctCurrent: number,
): string {
  if (!shadow) {
    return (
      `no shadow record supplied to this run (the ${changeClass} class is the one S3 shadows; ` +
      `its diff summary joins the pack when the generator runs against the aggregator state dir, ` +
      `and the post-change shadow record is what S7 scores the expected effect against)`
    );
  }
  const demoted = shadow.pools.filter((p) => p.moved === "demoted").map((p) => p.providerId);
  return (
    `shadow record ${shadow.generatedAt} (computed at NEAR_PCT=${nearPctCurrent}): ` +
    `${shadow.summary.pools} pools, ${shadow.summary.moved} moved, ${shadow.summary.demotedByForecast} demoted by forecast` +
    (demoted.length ? ` (${demoted.join(", ")})` : "") +
    `; live and shadow orderings ${shadow.summary.identical ? "identical" : "differ"}`
  );
}

/** Class pool-weight: one proposal per weighted route at most —
 * the strongest late-wall member against the strongest headroom
 * member. Every member that cannot ground is a named gap. */
function poolWeightProposals(args: {
  ledger: ForecastAccuracyLedger;
  routes: readonly RouteWeightTable[];
  forecast: QuotaForecastFile | null;
  shadow: ShadowPlacementRecord | null;
  nearPctCurrent: number;
  gaps: EvidenceGap[];
  examined: string[];
}): ReweightProposal[] {
  const byProvider = poolByProvider(args.ledger);
  const proposals: ReweightProposal[] = [];
  for (const route of args.routes) {
    args.examined.push(`route ${route.routeId} (selection ${route.selection})`);
    if (route.selection !== "weighted") continue;
    const members = Object.keys(route.weights);
    const evaluated: { member: string; pool: PoolBaseline; window: WindowBaseline }[] = [];
    for (const member of members) {
      const scope = `route ${route.routeId} member ${member}`;
      const pool = byProvider.get(member);
      if (!pool) {
        args.gaps.push({
          scope,
          reason: "no-forecast-pool-for-member",
          detail:
            `${member} matches no forecast pool's provider id exactly, so no accuracy ` +
            `evidence exists for it (near-miss spellings are not joined silently).`,
        });
        continue;
      }
      const w = bindingWindow(pool);
      const gate = gateWindow(scope, pool, w);
      if (!gate.ok || !w) {
        if (gate.gap) args.gaps.push(gate.gap);
        continue;
      }
      evaluated.push({ member, pool, window: w });
    }
    const late = evaluated
      .filter((e) => isLateWall(e.window))
      .sort((a, b) => (b.window.wallErrorMin?.medianSignedMin ?? 0) - (a.window.wallErrorMin?.medianSignedMin ?? 0));
    const head = evaluated
      .filter((e) => isHeadroom(e.window) && !late.includes(e))
      .sort((a, b) => b.window.forecastsScored - a.window.forecastsScored);
    if (!late.length || !head.length) {
      args.examined.push(
        `route ${route.routeId}: ${evaluated.length} members evaluated, ` +
          `${late.length} late-wall, ${head.length} headroom — no weight proposal`,
      );
      continue;
    }
    const from = late[0] as (typeof evaluated)[number];
    const to = head[0] as (typeof evaluated)[number];
    const fromOld = route.weights[from.member] as number;
    const toOld = route.weights[to.member] as number;
    const shift = Math.round(fromOld * WEIGHT_SHIFT_FRACTION * 100) / 100;
    const fromNew = Math.max(WEIGHT_FLOOR, Math.round((fromOld - shift) * 100) / 100);
    const moved = Math.round((fromOld - fromNew) * 100) / 100;
    if (moved <= 0) continue;
    const toNew = Math.round((toOld + moved) * 100) / 100;
    if (route.weightsLine === null) {
      args.gaps.push({
        scope: `route ${route.routeId}`,
        reason: "weights-not-single-line",
        detail:
          `the weights object for ${route.routeId} spans lines in the route config; ` +
          `the generator refuses to edit a shape it cannot anchor exactly.`,
      });
      continue;
    }
    const line = route.weightsLine;
    const fromEntry = `"${from.member}": ${fromOld}`;
    const toEntry = `"${to.member}": ${toOld}`;
    if (line.split(fromEntry).length !== 2 || line.split(toEntry).length !== 2) {
      args.gaps.push({
        scope: `route ${route.routeId}`,
        reason: "weights-line-anchor-mismatch",
        detail: `the weights line for ${route.routeId} does not contain ${fromEntry} / ${toEntry} exactly once each; refusing to edit.`,
      });
      continue;
    }
    const newLine = line.replace(fromEntry, `"${from.member}": ${fromNew}`).replace(toEntry, `"${to.member}": ${toNew}`);
    const rows = [evidenceRow(from.pool.pool, from.window), evidenceRow(to.pool.pool, to.window)];
    proposals.push({
      key: `pool-weight:${route.routeId}:${from.member}:${fromOld}->${fromNew}`,
      proposalClass: "pool-weight",
      title: `proposal(quota-plane): shift ${route.routeId} weight from ${from.member} (${fromOld} → ${fromNew}) to ${to.member} (${toOld} → ${toNew})`,
      target: {
        description: `route ${route.routeId} weights in the route config`,
        file: ".opencode/opencode.jsonc",
        find: line,
        replace: newLine,
        currentValue: `${from.member}=${fromOld}, ${to.member}=${toOld}`,
        proposedValue: `${from.member}=${fromNew}, ${to.member}=${toNew}`,
      },
      rationale:
        `${from.member}'s binding window (${from.pool.pool}/${from.window.window}) walls systematically earlier than ` +
        `the forecast projects: precision ${from.window.precision} (wall calls are real), recall ${from.window.recall} ` +
        `(walls are systematically missed), median signed wall error +${from.window.wallErrorMin?.medianSignedMin} min ` +
        `over ${from.window.forecastsScored} scored forecasts. ${to.member} shows headroom on the same evidence standard ` +
        `(precision ${to.window.precision}, recall ${to.window.recall}, ${to.window.forecastsScored} scored). Shifting ` +
        `${moved} of weight moves burn toward the pool the evidence says holds, and preserves the route's total weight.`,
      evidence: {
        ledgerGeneratedAt: args.ledger.generatedAt,
        ledgerStore: args.ledger.store,
        ledgerModel: args.ledger.model,
        rows,
        forecastSummary: forecastSummaryFor(args.forecast, [from.pool.pool, to.pool.pool]),
        shadowSummary: shadowSummaryFor(args.shadow, "pool-weight", args.nearPctCurrent),
      },
      expectedEffect: {
        statement:
          `In the four weeks after this lands, ${from.pool.pool}'s share of the route's realized walls falls and its ` +
          `weekly wall-before-reset recall (S2 scoring) rises toward ${HEADROOM_RECALL_MIN}, with no scoreable window ` +
          `of ${to.pool.pool} losing precision below ${LATE_WALL_PRECISION_MIN}.`,
        metric: "per-pool wall-before-reset precision/recall and realized wall counts, S2-scored",
        scoredBy: "the S7 weekly review, from the accuracy ledger and the snapshot store",
      },
    });
  }
  return proposals;
}

/** Class caution-margin: walls arriving systematically early on
 * weekly windows across pools → widen NEAR_PCT by one step. */
function cautionMarginProposal(args: {
  ledger: ForecastAccuracyLedger;
  forecast: QuotaForecastFile | null;
  shadow: ShadowPlacementRecord | null;
  nearPctCurrent: number;
  gaps: EvidenceGap[];
  examined: string[];
}): ReweightProposal | null {
  const weekly: { pool: PoolBaseline; window: WindowBaseline }[] = [];
  for (const pool of args.ledger.pools) {
    const w = pool.windows.find((x) => x.window === "weekly");
    if (w) weekly.push({ pool, window: w });
  }
  args.examined.push(`weekly baselines: ${weekly.length} pools carry one`);
  const considered = weekly.filter(
    ({ window: w }) => w.scoreable && w.forecastsScored >= MIN_SCORED_FORECASTS,
  );
  const qualifying = considered.filter(
    ({ pool, window: w }) =>
      !AGGREGATE_POOLS_EXCLUDED.has(pool.pool) &&
      w.wallErrorMin !== null &&
      w.wallErrorMin.count >= MIN_WALL_SAMPLES &&
      w.wallErrorMin.medianSignedMin >= LATE_WALL_MEDIAN_MIN,
  );
  const anchors = qualifying.filter(({ window: w }) => w.precision !== null && w.precision >= LATE_WALL_PRECISION_MIN);
  const counter = considered.filter(
    ({ pool, window: w }) =>
      !AGGREGATE_POOLS_EXCLUDED.has(pool.pool) &&
      w.wallErrorMin !== null &&
      w.wallErrorMin.medianSignedMin <= -LATE_WALL_MEDIAN_MIN,
  );
  const excluded = considered.filter(({ pool }) => AGGREGATE_POOLS_EXCLUDED.has(pool.pool));
  for (const { pool, window: w } of excluded) {
    args.gaps.push({
      scope: `caution-margin evidence ${pool.pool}/weekly`,
      reason: "aggregate-pool-excluded",
      detail:
        `${pool.pool}/weekly shows median signed wall error +${w.wallErrorMin?.medianSignedMin} min over ` +
        `${w.forecastsScored} scored forecasts, but the bare pool's rows mix every workspace's records ` +
        `(S2) and never ground a proposal; it is reported here, not counted.`,
    });
  }
  args.examined.push(
    `caution-margin: ${considered.length} weekly baselines over the volume gate, ` +
      `${qualifying.length} qualifying late-wall, ${anchors.length} precision anchors, ${counter.length} counter-evidence`,
  );
  if (qualifying.length < 2 || anchors.length < 1 || counter.length > 0) {
    if (qualifying.length > 0 && counter.length > 0) {
      args.gaps.push({
        scope: "caution-margin",
        reason: "counter-evidence-present",
        detail:
          `weekly walls arrive early in ${qualifying.map((q) => q.pool.pool).join(", ")} but late in ` +
          `${counter.map((c) => c.pool.pool).join(", ")}; a global margin change is not grounded while the ` +
          `calibration disagrees with itself.`,
      });
    }
    return null;
  }
  const from = args.nearPctCurrent;
  const to = from + MARGIN_STEP_PTS;
  if (to > MARGIN_MAX_PCT) {
    args.gaps.push({
      scope: "caution-margin",
      reason: "margin-at-ceiling",
      detail: `NEAR_PCT is already ${from}; the generator does not propose past ${MARGIN_MAX_PCT}.`,
    });
    return null;
  }
  const rows = considered
    .filter(({ pool }) => !AGGREGATE_POOLS_EXCLUDED.has(pool.pool))
    .map(({ pool, window: w }) => evidenceRow(pool.pool, w));
  const names = qualifying.map((q) => q.pool.pool).join(", ");
  return {
    key: `caution-margin:NEAR_PCT:${from}->${to}`,
    proposalClass: "caution-margin",
    title: `proposal(quota-plane): widen the caution margin NEAR_PCT ${from} → ${to} (weekly walls arrive systematically early)`,
    target: {
      description: "the caution margin NEAR_PCT (derive.ts), the threshold at which a pool enters the caution/near tier",
      file: "pm-stack/aggregator/src/derive.ts",
      find: `export const NEAR_PCT = ${from};`,
      replace: `export const NEAR_PCT = ${to};`,
      currentValue: `${from}`,
      proposedValue: `${to}`,
    },
    rationale:
      `Weekly walls arrive systematically earlier than the linear pace projects: ` +
      `${qualifying
        .map(
          ({ pool, window: w }) =>
            `${pool.pool} (median signed wall error +${w.wallErrorMin?.medianSignedMin} min over ${w.forecastsScored} scored forecasts, precision ${w.precision}, recall ${w.recall})`,
        )
        .join("; ")}. ` +
      `The caution tier is entered on measured percent at NEAR_PCT; widening it by one step (${from} → ${to}) puts ` +
      `pools under caution discipline earlier in exactly the windows whose walls the forecast under-calls. ` +
      `D4 reserves margin changes to this proposal mechanism; the #1342 controlled A/B gate applies before the ` +
      `margin class becomes routine.`,
    evidence: {
      ledgerGeneratedAt: args.ledger.generatedAt,
      ledgerStore: args.ledger.store,
      ledgerModel: args.ledger.model,
      rows,
      forecastSummary: forecastSummaryFor(
        args.forecast,
        qualifying.map((q) => q.pool.pool),
      ),
      shadowSummary: shadowSummaryFor(args.shadow, "caution-margin", args.nearPctCurrent),
    },
    expectedEffect: {
      statement:
        `In the four weeks after this lands, the fraction of realized weekly walls in the grounding pools ` +
        `(${names}) that were preceded by at least one gatherer tick with the pool in the caution/near state ` +
        `rises versus the pre-change baseline computed the same way, and no scoreable weekly baseline's ` +
        `wall-before-reset precision falls below 0.5.`,
      metric: "caution-tier lead coverage of realized weekly walls (shadow history × realized wall instants)",
      scoredBy: "the S7 weekly review, from quota-shadow-placement-history.jsonl and the accuracy ledger",
    },
  };
}

/** Run every v1 proposal class over the evidence. */
export function buildReweightProposals(args: {
  ledger: ForecastAccuracyLedger;
  routes: readonly RouteWeightTable[];
  forecast?: QuotaForecastFile | null;
  shadow?: ShadowPlacementRecord | null;
  nearPctCurrent: number;
}): ReweightRun {
  const gaps: EvidenceGap[] = [];
  const examined: string[] = [];
  const forecast = args.forecast ?? null;
  const shadow = args.shadow ?? null;
  const proposals = poolWeightProposals({
    ledger: args.ledger,
    routes: args.routes,
    forecast,
    shadow,
    nearPctCurrent: args.nearPctCurrent,
    gaps,
    examined,
  });
  const margin = cautionMarginProposal({
    ledger: args.ledger,
    forecast,
    shadow,
    nearPctCurrent: args.nearPctCurrent,
    gaps,
    examined,
  });
  if (margin) proposals.push(margin);
  return { proposals, gaps, examined };
}

function fmtMetric(n: number | null): string {
  return n === null ? "undefined" : `${n}`;
}

function fmtSigned(n: number | null): string {
  if (n === null) return "n/a";
  return n > 0 ? `+${n}` : `${n}`;
}

/** The evidence pack: the PR body a proposal carries (design D5:
 * the forecasts behind the proposal, their accuracy record, the
 * shadow diff summary when the class was shadow-tested, and the
 * expected effect). Also written verbatim as proposal.md in draft
 * mode, so the pack reviewed in a draft is the pack the PR gets. */
export function renderEvidencePack(p: ReweightProposal): string {
  const s = p.evidence.ledgerStore;
  const lines: string[] = [
    `# ${p.title}`,
    "",
    `> Generated by the Stage-3 re-weighting proposal generator (S6, #12700; design D5).`,
    `> This PR was opened by the generator, not hand-written. It proposes; it does not`,
    `> merge — merge authority is unchanged (Felix/Drew), and the #1342 controlled A/B`,
    `> gate applies before this proposal class becomes routine.`,
    "",
    `**Proposal key:** \`${p.key}\``,
    `**Class:** ${p.proposalClass}`,
    `**Target:** ${p.target.description} — \`${p.target.file}\`: \`${p.target.currentValue}\` → \`${p.target.proposedValue}\``,
    "",
    "## Rationale",
    "",
    p.rationale,
    "",
    "## Evidence — accuracy record (S2)",
    "",
    `Accuracy ledger generated ${p.evidence.ledgerGeneratedAt}; model: ${p.evidence.ledgerModel}.`,
    `Store span: ${s.records} records (${s.pctRecords} percent readings), ${s.fromIso ?? "?"} → ${s.toIso ?? "?"}.`,
    "",
    "| pool | window | scored | precision | recall | median signed wall error (min) | wall samples | walled periods |",
    "|---|---|---|---|---|---|---|---|",
    ...p.evidence.rows.map(
      (r) =>
        `| ${r.pool} | ${r.window} | ${r.forecastsScored} | ${fmtMetric(r.precision)} | ${fmtMetric(r.recall)} | ${fmtSigned(r.wallErrorMedianSignedMin)} | ${r.wallErrorCount ?? "n/a"} | ${r.periodsWalled}/${r.periodsScored} |`,
    ),
    "",
    "## Evidence — forecast (S1)",
    "",
    p.evidence.forecastSummary,
    "",
    "## Evidence — shadow diff (S3)",
    "",
    p.evidence.shadowSummary,
    "",
    "## Expected effect (scored by the weekly review)",
    "",
    p.expectedEffect.statement,
    "",
    `- Metric: ${p.expectedEffect.metric}`,
    `- Scored by: ${p.expectedEffect.scoredBy}`,
    "",
  ];
  return lines.join("\n");
}

/** The run report: proposals and gaps together — the gaps are
 * half the product (they are the evidence register's coverage,
 * stated instead of silently skipped). */
export function renderRunReport(run: ReweightRun, generatedAt: string): string {
  const lines: string[] = [
    `# Re-weighting proposal run — ${generatedAt}`,
    "",
    "## Examined",
    "",
    ...run.examined.map((e) => `- ${e}`),
    "",
    `## Proposals (${run.proposals.length})`,
    "",
    ...(run.proposals.length
      ? run.proposals.map((p) => `- \`${p.key}\` — ${p.title}`)
      : ["- none: the evidence grounded no durable change this run"]),
    "",
    `## Evidence gaps (${run.gaps.length})`,
    "",
    ...(run.gaps.length
      ? run.gaps.map((g) => `- **${g.scope}** [\`${g.reason}\`]: ${g.detail}`)
      : ["- none"]),
    "",
  ];
  return lines.join("\n");
}
