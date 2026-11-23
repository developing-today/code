// Stage-3 S7 — the weekly forecast-accuracy review (issue #12701,
// slice S7 of epic #12694; design
// doc/2026-11-22T22-35-00Z_design_quota-plane-stage-3 §7, D7).
//
// The review is a report and proposals, never edits: this module is
// the pure core the scheduled job (scripts/quota-weekly-review.ts)
// runs. It scores the week's forecasts against realized outcomes
// from S2's accuracy ledger — the scored facts S2's backtest
// produces, cut to the week — appends the §6 detector findings of
// the week, and folds the S6 re-weighting run into the report as the
// week's rule consequence: the proposals it produced (with their PRs
// when the job opened them), or an explicit no-change finding with
// the accuracy numbers. The design's anti-decay rule (§7): the
// first two reports must each record at least one rule consequence,
// so a report with neither a proposal nor a no-change finding is not
// a shape this module can render — when the S6 fold was not
// evaluated at all, the report says so in those words instead.
//
// Week membership, stated once: a scored forecast belongs to the
// week its reset period completed in — the fact's resetsAt in the
// half-open interval [weekStart, weekEnd) — because that is when its
// outcome was realized. A detector finding belongs to the week of
// the instant it is about (its `at`). Precision/recall use the
// textbook definitions with S2's rounding and null rules (undefined
// stays null, never 0); wall error reuses S2's errorDistribution so
// the week's quantiles are the ledger's quantiles.
import {
  errorDistribution,
  type ErrorDistribution,
  type ForecastAccuracyLedger,
  type PoolBaseline,
  type ScoredForecast,
} from "./quota-forecast-accuracy.ts";
import type { AnomalyDetectorId, AnomalyFinding } from "./quota-anomaly.ts";
import type { EvidenceGap, ReweightRun } from "./quota-reweight.ts";

export interface WeekPoolScore {
  pool: string;
  providerId: string | null;
  forecastsScored: number;
  tp: number;
  fp: number;
  fn: number;
  tn: number;
  /** Null when no positive prediction was scored (S2's rule). */
  precision: number | null;
  /** Null when no realized wall was scored (S2's rule). */
  recall: number | null;
  wallErrorMin: ErrorDistribution | null;
}

/** Score the facts whose period completed in [weekStartMs, weekEndMs),
 * grouped per pool, in first-seen pool order. */
export function scoreWeekFacts(
  facts: readonly ScoredForecast[],
  weekStartMs: number,
  weekEndMs: number,
): WeekPoolScore[] {
  const byPool = new Map<string, { providerId: string | null; facts: ScoredForecast[] }>();
  for (const f of facts) {
    const resetMs = Date.parse(f.resetsAt);
    if (!Number.isFinite(resetMs) || resetMs < weekStartMs || resetMs >= weekEndMs) continue;
    let entry = byPool.get(f.pool);
    if (!entry) {
      entry = { providerId: f.providerId, facts: [] };
      byPool.set(f.pool, entry);
    }
    entry.facts.push(f);
  }
  const out: WeekPoolScore[] = [];
  for (const [pool, entry] of byPool) {
    let tp = 0;
    let fp = 0;
    let fn = 0;
    let tn = 0;
    const errors: number[] = [];
    for (const f of entry.facts) {
      if (f.outcome === "tp") tp++;
      else if (f.outcome === "fp") fp++;
      else if (f.outcome === "fn") fn++;
      else tn++;
      if (f.errorMin !== null) errors.push(f.errorMin);
    }
    out.push({
      pool,
      providerId: entry.providerId,
      forecastsScored: entry.facts.length,
      tp,
      fp,
      fn,
      tn,
      precision: tp + fp > 0 ? Math.round((tp / (tp + fp)) * 1000) / 1000 : null,
      recall: tp + fn > 0 ? Math.round((tp / (tp + fn)) * 1000) / 1000 : null,
      wallErrorMin: errorDistribution(errors),
    });
  }
  return out;
}

export interface WeekFindingsSummary {
  total: number;
  /** Every detector, in the §6 register's order, zeros included. */
  counts: { detector: AnomalyDetectorId; count: number }[];
  /** The week's findings, chronological. */
  findings: AnomalyFinding[];
}

const DETECTOR_ORDER: readonly AnomalyDetectorId[] = ["burn-spike", "schema-drift", "plan-change"];

export function summarizeWeekFindings(
  findings: readonly AnomalyFinding[],
  weekStartMs: number,
  weekEndMs: number,
): WeekFindingsSummary {
  const inWeek = findings
    .filter((f) => {
      const atMs = Date.parse(f.at);
      return Number.isFinite(atMs) && atMs >= weekStartMs && atMs < weekEndMs;
    })
    .sort((a, b) => Date.parse(a.at) - Date.parse(b.at));
  return {
    total: inWeek.length,
    counts: DETECTOR_ORDER.map((detector) => ({
      detector,
      count: inWeek.filter((f) => f.detector === detector).length,
    })),
    findings: inWeek,
  };
}

export interface WeeklyReviewPool {
  pool: string;
  providerId: string | null;
  week: WeekPoolScore;
  /** The pool's all-history S2 rollup — the calibration the week's
   * numbers are read against. */
  rollup: PoolBaseline["rollup"];
}

export interface WeeklyReview {
  version: 1;
  kind: "quota-weekly-review";
  generatedAt: string;
  weekStartIso: string;
  weekEndIso: string;
  model: string;
  store: ForecastAccuracyLedger["store"];
  pools: WeeklyReviewPool[];
  findings: WeekFindingsSummary;
  paceHoldNote: string;
  /** The S6 fold's proposals for this week (empty when the fold
   * produced none or was not evaluated). */
  proposals: { key: string; title: string; prUrl: string | null }[];
  gaps: EvidenceGap[];
  examined: string[];
  /** True when the S6 fold ran (a route config was supplied). */
  reweightEvaluated: boolean;
  /** The explicit no-change finding, composed when the fold ran and
   * produced no proposal; null otherwise (design §7 anti-decay). */
  noChangeFinding: string | null;
}

/** The standing pace_hold note while S4 (#12698) is unlanded: there
 * are no live forecast-driven admission verdicts to score, and the
 * report names that instead of printing an empty section. The job
 * overrides this the week S4 produces records. */
export const PACE_HOLD_NOT_LIVE_NOTE =
  "No live pace_hold outcomes to score: S4 (#12698, live forecast-driven admission) has not landed, " +
  "so admission issued no forecast-driven pace_hold verdicts this week.";

function emptyWeekScore(pool: string, providerId: string | null): WeekPoolScore {
  return {
    pool,
    providerId,
    forecastsScored: 0,
    tp: 0,
    fp: 0,
    fn: 0,
    tn: 0,
    precision: null,
    recall: null,
    wallErrorMin: null,
  };
}

function emptyRollup(): PoolBaseline["rollup"] {
  return {
    scoreable: false,
    unscoreableReasons: ["pool absent from the accuracy ledger"],
    periodsTotal: 0,
    periodsScored: 0,
    periodsWalled: 0,
    periodsOpen: 0,
    periodsNoReset: 0,
    forecastsIssued: 0,
    forecastsScored: 0,
    forecastsInsufficient: 0,
    detectionsWhileWalled: 0,
    tp: 0,
    fp: 0,
    fn: 0,
    tn: 0,
    precision: null,
    recall: null,
    wallErrorMin: null,
    firstCallLeadMin: null,
  };
}

export function buildWeeklyReview(args: {
  ledger: ForecastAccuracyLedger;
  facts: readonly ScoredForecast[];
  findings: readonly AnomalyFinding[];
  weekStartMs: number;
  weekEndMs: number;
  generatedAt: string;
  paceHoldNote?: string;
  reweight?: ReweightRun | null;
  openedPrs?: readonly { key: string; url: string }[];
}): WeeklyReview {
  const weekScores = scoreWeekFacts(args.facts, args.weekStartMs, args.weekEndMs);
  const scoreByPool = new Map(weekScores.map((s) => [s.pool, s]));
  const pools: WeeklyReviewPool[] = args.ledger.pools.map((p) => ({
    pool: p.pool,
    providerId: p.providerId,
    week: scoreByPool.get(p.pool) ?? emptyWeekScore(p.pool, p.providerId),
    rollup: p.rollup,
  }));
  // A pool present in facts but missing from the ledger cannot
  // happen (facts are built from the ledger's pools) — but if the
  // inputs ever disagree, the week's facts win a row rather than
  // being dropped silently.
  for (const s of weekScores) {
    if (!pools.some((p) => p.pool === s.pool)) {
      pools.push({
        pool: s.pool,
        providerId: s.providerId,
        week: s,
        rollup: emptyRollup(),
      });
    }
  }

  const reweight = args.reweight ?? null;
  const prByKey = new Map((args.openedPrs ?? []).map((p) => [p.key, p.url]));
  const proposals = (reweight?.proposals ?? []).map((p) => ({
    key: p.key,
    title: p.title,
    prUrl: prByKey.get(p.key) ?? null,
  }));

  let noChangeFinding: string | null = null;
  if (reweight && proposals.length === 0) {
    const totalScored = pools.reduce((sum, p) => sum + p.week.forecastsScored, 0);
    const poolsScored = pools.filter((p) => p.week.forecastsScored > 0).length;
    noChangeFinding =
      `no rule change is proposed this week: the S6 generator examined ${reweight.examined.length} ` +
      `class(es) (${reweight.examined.join("; ") || "none recorded"}) and produced no proposal. ` +
      `The week's accuracy record behind that finding: ${totalScored} forecasts scored across ` +
      `${poolsScored} pool(s); ${reweight.gaps.length} evidence gap(s) recorded below.`;
  }

  return {
    version: 1,
    kind: "quota-weekly-review",
    generatedAt: args.generatedAt,
    weekStartIso: new Date(args.weekStartMs).toISOString(),
    weekEndIso: new Date(args.weekEndMs).toISOString(),
    model: args.ledger.model,
    store: args.ledger.store,
    pools,
    findings: summarizeWeekFindings(args.findings, args.weekStartMs, args.weekEndMs),
    paceHoldNote: args.paceHoldNote ?? PACE_HOLD_NOT_LIVE_NOTE,
    proposals,
    gaps: reweight?.gaps ?? [],
    examined: reweight?.examined ?? [],
    reweightEvaluated: reweight !== null,
    noChangeFinding,
  };
}

function fmtMetric(n: number | null): string {
  return n === null ? "undefined" : `${n}`;
}

function fmtErr(e: ErrorDistribution | null): string {
  if (!e) return "—";
  return `${e.count} TP — median signed ${e.medianSignedMin}m, median abs ${e.medianAbsMin}m, p90 abs ${e.p90AbsMin}m`;
}

/** The report the job posts on epic #12694 (design §7 step 3). */
export function renderWeeklyReview(review: WeeklyReview): string {
  const lines: string[] = [];
  lines.push(`# Quota forecast weekly review — ${review.weekStartIso.slice(0, 10)} → ${review.weekEndIso.slice(0, 10)}`);
  lines.push("");
  lines.push("Stage-3 S7 weekly review (#12701; design §7, D7), posted by the quota-weekly-review job.");
  lines.push("");
  lines.push(`- Week: ${review.weekStartIso} → ${review.weekEndIso} (a scored forecast belongs to the week its reset period completed in)`);
  lines.push(`- Model under score: ${review.model}`);
  lines.push(
    `- Store: ${review.store.records} records (${review.store.pctRecords} percent readings), ${review.store.fromIso ?? "?"} → ${review.store.toIso ?? "?"}`,
  );
  lines.push("");
  lines.push("## Per-pool accuracy — this week (projected vs realized)");
  lines.push("");
  lines.push("| pool | forecasts scored | TP / FP / FN / TN | precision | recall | wall error (projected − realized) |");
  lines.push("|---|---|---|---|---|---|");
  for (const p of review.pools) {
    const w = p.week;
    lines.push(
      `| ${p.pool} | ${w.forecastsScored} | ${w.tp} / ${w.fp} / ${w.fn} / ${w.tn} | ${fmtMetric(w.precision)} | ${fmtMetric(w.recall)} | ${fmtErr(w.wallErrorMin)} |`,
    );
  }
  lines.push("");
  lines.push("## Per-pool calibration — all history (S2 ledger rollup)");
  lines.push("");
  lines.push("| pool | scoreable | periods (scored/walled) | forecasts scored | precision | recall |");
  lines.push("|---|---|---|---|---|---|");
  for (const p of review.pools) {
    const r = p.rollup;
    lines.push(
      `| ${p.pool} | ${r.scoreable ? "yes" : "NO"} | ${r.periodsScored}/${r.periodsWalled} | ${r.forecastsScored} | ${fmtMetric(r.precision)} | ${fmtMetric(r.recall)} |`,
    );
  }
  lines.push("");
  lines.push("## Detector findings (§6) — this week");
  lines.push("");
  if (review.findings.total === 0) {
    lines.push("No detector findings this week.");
  } else {
    lines.push(
      `${review.findings.total} finding(s): ${review.findings.counts
        .filter((c) => c.count > 0)
        .map((c) => `${c.detector} ${c.count}`)
        .join(", ")}.`,
    );
    lines.push("");
    for (const f of review.findings.findings) {
      lines.push(`- \`${f.at}\` **${f.detector}** (${f.serviceId}${f.window ? `, ${f.window}` : ""}) — ${f.title}`);
      lines.push(`  - baseline: ${f.baseline}`);
      lines.push(`  - observed: ${f.observed}`);
    }
  }
  lines.push("");
  lines.push("## pace_hold outcomes");
  lines.push("");
  lines.push(review.paceHoldNote);
  lines.push("");
  lines.push("## Rule consequences (§5 fold)");
  lines.push("");
  if (!review.reweightEvaluated) {
    lines.push(
      "The S6 fold was not evaluated this week: no route config was supplied to the review job, so no proposal " +
        "could be generated and no no-change finding is claimed. Supplying the route config is the job's " +
        "standing configuration (see the schedule-registry entry), and its absence is recorded here rather " +
        "than papered over.",
    );
  } else if (review.proposals.length > 0) {
    lines.push("Proposed through the S6 PR mechanism (never edited in place):");
    lines.push("");
    for (const p of review.proposals) {
      lines.push(`- \`${p.key}\` — ${p.title}${p.prUrl ? ` — ${p.prUrl}` : " (PR not opened by this run)"}`);
    }
  } else {
    lines.push("### No-change finding");
    lines.push("");
    lines.push(review.noChangeFinding ?? "");
  }
  if (review.gaps.length) {
    lines.push("");
    lines.push("Evidence gaps recorded by the generator:");
    lines.push("");
    for (const g of review.gaps) lines.push(`- \`${g.scope}\` — ${g.reason}: ${g.detail}`);
  }
  lines.push("");
  lines.push(
    "_The weekly review produces this report and proposals only; it never edits quota config in place (design §7, D7)._",
  );
  lines.push("");
  return lines.join("\n");
}
