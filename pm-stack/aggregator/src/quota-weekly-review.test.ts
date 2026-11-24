// Tests for the Stage-3 S7 weekly review (#12701, design §7/D7).
// Every test names its oracle: the textbook definitions of
// precision/recall with S2's rounding and null rules
// (quota-forecast-accuracy.ts), S2's own errorDistribution for the
// wall-error quantiles, and the design's §7 report contract (per-pool
// projected-vs-realized, the §6 findings summary, a recorded rule
// consequence in every report).
import { describe, expect, test } from "bun:test";
import { buildWeeklyReview, renderWeeklyReview, scoreWeekFacts, summarizeWeekFindings } from "./quota-weekly-review.ts";
import type { ForecastAccuracyLedger, ScoredForecast } from "./quota-forecast-accuracy.ts";
import type { AnomalyFinding } from "./quota-anomaly.ts";
import type { ReweightProposal, ReweightRun } from "./quota-reweight.ts";

const WEEK_START = Date.parse("2026-11-16T00:00:00Z");
const WEEK_END = Date.parse("2026-11-23T00:00:00Z");
const iso = (ms: number) => new Date(ms).toISOString();

function fact(partial: Partial<ScoredForecast> & { outcome: ScoredForecast["outcome"] }): ScoredForecast {
  return {
    pool: "alpha",
    providerId: "alpha-provider",
    window: "5h",
    issuedAt: iso(WEEK_START + 3_600_000),
    resetsAt: iso(WEEK_START + 5 * 3_600_000),
    state: "projected",
    projectedWallAt: null,
    wallsBeforeReset: partial.outcome === "tp" || partial.outcome === "fp",
    realizedWallAt: null,
    errorMin: null,
    ...partial,
  };
}

const FACTS: ScoredForecast[] = [
  fact({ outcome: "tp", errorMin: 30 }),
  fact({ outcome: "tp", errorMin: -10 }),
  fact({ outcome: "fp" }),
  fact({ outcome: "fn" }),
  fact({ outcome: "tn" }),
  // Period reset before the week: excluded even though scored.
  fact({ outcome: "tp", errorMin: 999, resetsAt: iso(WEEK_START - 3_600_000) }),
  // Period reset exactly at the week's end: excluded (half-open week).
  fact({ outcome: "tp", errorMin: 999, resetsAt: iso(WEEK_END) }),
  // Period reset exactly at the week's start: included.
  fact({ outcome: "tn", resetsAt: iso(WEEK_START) }),
  // Another pool, outside the week only.
  fact({ pool: "beta", providerId: null, outcome: "tp", errorMin: 5, resetsAt: iso(WEEK_START - 7_200_000) }),
];

function finding(partial: Partial<AnomalyFinding> & { detector: AnomalyFinding["detector"] }): AnomalyFinding {
  return {
    key: "k",
    at: iso(WEEK_START + 86_400_000),
    serviceId: "alpha",
    providerId: null,
    window: null,
    severity: "warning",
    title: "a finding",
    baseline: "baseline",
    observed: "observed",
    detail: {},
    ...partial,
  };
}

const FINDINGS: AnomalyFinding[] = [
  finding({ detector: "burn-spike", key: "b1", title: "burn spike one" }),
  finding({ detector: "burn-spike", key: "b2", title: "burn spike two", at: iso(WEEK_START + 2 * 86_400_000) }),
  finding({ detector: "schema-drift", key: "s1", title: "schema drift one", at: iso(WEEK_START + 3 * 86_400_000) }),
  finding({ detector: "plan-change", key: "p0", title: "before the week", at: iso(WEEK_START - 86_400_000) }),
];

function ledger(): ForecastAccuracyLedger {
  const windowBaseline = {
    window: "5h",
    scoreable: true,
    unscoreableReasons: [],
    periodsTotal: 1,
    periodsScored: 1,
    periodsWalled: 1,
    periodsOpen: 0,
    periodsNoReset: 0,
    forecastsIssued: 8,
    forecastsScored: 8,
    forecastsInsufficient: 0,
    detectionsWhileWalled: 0,
    tp: 3,
    fp: 1,
    fn: 1,
    tn: 3,
    precision: 0.75,
    recall: 0.75,
    wallErrorMin: null,
    firstCallLeadMin: null,
  };
  const { window: _window, ...rollup } = windowBaseline;
  return {
    version: 1,
    kind: "quota-forecast-accuracy",
    generatedAt: iso(WEEK_END),
    model: "test model",
    store: { records: 100, pctRecords: 90, fromIso: iso(WEEK_START - 30 * 86_400_000), toIso: iso(WEEK_END) },
    pools: [
      {
        pool: "alpha",
        providerId: "alpha-provider",
        serviceIdsJoined: ["alpha"],
        windows: [windowBaseline],
        rollup,
      },
      {
        pool: "beta",
        providerId: null,
        serviceIdsJoined: ["beta"],
        windows: [],
        rollup: {
          scoreable: false,
          unscoreableReasons: ["no completed periods"],
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
        },
      },
    ],
  };
}

function proposal(): ReweightProposal {
  return {
    key: "caution-margin/NEAR_PCT/80-to-85",
    proposalClass: "caution-margin",
    title: "widen the caution margin NEAR_PCT 80 → 85",
    target: {
      description: "caution margin",
      file: "pm-stack/aggregator/src/derive.ts",
      find: "export const NEAR_PCT = 80;",
      replace: "export const NEAR_PCT = 85;",
      currentValue: "80",
      proposedValue: "85",
    },
    rationale: "walls arrive early",
    evidence: {
      ledgerGeneratedAt: iso(WEEK_END),
      ledgerStore: ledger().store,
      ledgerModel: "test model",
      rows: [],
      forecastSummary: "n/a",
      shadowSummary: "n/a",
    },
    expectedEffect: { statement: "walls arrive later", metric: "lead coverage", scoredBy: "the S7 weekly review" },
  };
}

describe("scoreWeekFacts (oracle: textbook precision/recall, S2 rounding + null rules)", () => {
  test("the week is the half-open reset interval [start, end)", () => {
    const rows = scoreWeekFacts(FACTS, WEEK_START, WEEK_END);
    const alpha = rows.find((r) => r.pool === "alpha");
    if (!alpha) throw new Error("alpha row missing");
    expect(alpha.forecastsScored).toBe(6);
    expect(alpha.tp).toBe(2);
    expect(alpha.fp).toBe(1);
    expect(alpha.fn).toBe(1);
    expect(alpha.tn).toBe(2);
    expect(alpha.precision).toBe(0.667);
    expect(alpha.recall).toBe(0.667);
    // Wall error over the week's TP errors [30, -10] only — the two
    // out-of-week 999s must not leak in (S2 errorDistribution).
    expect(alpha.wallErrorMin?.count).toBe(2);
    expect(alpha.wallErrorMin?.meanSignedMin).toBe(10);
    expect(alpha.wallErrorMin?.medianAbsMin).toBe(20);
    expect(alpha.wallErrorMin?.p90AbsMin).toBe(28);
  });

  test("a pool with no in-week facts gets no row from scoring alone", () => {
    // scoreWeekFacts scores facts, nothing else: beta's only fact is
    // out-of-week, so beta is absent here. The zero row with
    // undefined precision/recall is buildWeeklyReview's job (it owns
    // the ledger's pool list) and is asserted there.
    const rows = scoreWeekFacts(FACTS, WEEK_START, WEEK_END);
    expect(rows.map((r) => r.pool)).toEqual(["alpha"]);
  });
});

describe("summarizeWeekFindings (oracle: design §7 step 2 — the §6 findings of the week)", () => {
  test("only findings inside the week count, grouped by detector", () => {
    const summary = summarizeWeekFindings(FINDINGS, WEEK_START, WEEK_END);
    expect(summary.total).toBe(3);
    expect(summary.counts).toEqual([
      { detector: "burn-spike", count: 2 },
      { detector: "schema-drift", count: 1 },
      { detector: "plan-change", count: 0 },
    ]);
    expect(summary.findings.map((f) => f.key)).toEqual(["b1", "b2", "s1"]);
  });
});

describe("buildWeeklyReview + renderWeeklyReview (oracle: design §7 report contract)", () => {
  test("a proposal from the S6 fold is the recorded rule consequence, with its PR when opened", () => {
    const run: ReweightRun = { proposals: [proposal()], gaps: [], examined: ["caution-margin class"] };
    const review = buildWeeklyReview({
      ledger: ledger(),
      facts: FACTS,
      findings: FINDINGS,
      weekStartMs: WEEK_START,
      weekEndMs: WEEK_END,
      generatedAt: iso(WEEK_END),
      reweight: run,
      openedPrs: [{ key: proposal().key, url: "https://example.invalid/pr/1" }],
    });
    expect(review.noChangeFinding).toBeNull();
    const md = renderWeeklyReview(review);
    expect(md).toContain("# Quota forecast weekly review");
    expect(md).toContain("caution-margin/NEAR_PCT/80-to-85");
    expect(md).toContain("https://example.invalid/pr/1");
    expect(md).toContain("burn spike one");
    // S4 is not live: the pace_hold section names the absence.
    expect(md).toContain("#12698");
  });

  test("no proposal yields an explicit no-change finding carrying the week's numbers", () => {
    const run: ReweightRun = {
      proposals: [],
      gaps: [{ scope: "pool-weight class", reason: "insufficient-history", detail: "too few scored forecasts" }],
      examined: ["pool-weight class", "caution-margin class"],
    };
    const review = buildWeeklyReview({
      ledger: ledger(),
      facts: FACTS,
      findings: FINDINGS,
      weekStartMs: WEEK_START,
      weekEndMs: WEEK_END,
      generatedAt: iso(WEEK_END),
      reweight: run,
    });
    if (!review.noChangeFinding) throw new Error("no-change finding missing");
    expect(review.noChangeFinding).toContain("no rule change");
    expect(review.noChangeFinding).toContain("6 forecasts scored");
    const md = renderWeeklyReview(review);
    expect(md).toContain("No-change finding");
    expect(md).toContain("insufficient-history");
  });

  test("without a reweight run the fold is reported as not evaluated, never as a no-change", () => {
    const review = buildWeeklyReview({
      ledger: ledger(),
      facts: FACTS,
      findings: FINDINGS,
      weekStartMs: WEEK_START,
      weekEndMs: WEEK_END,
      generatedAt: iso(WEEK_END),
      reweight: null,
    });
    expect(review.noChangeFinding).toBeNull();
    const md = renderWeeklyReview(review);
    expect(md).toContain("not evaluated");
  });

  test("every ledger pool appears in the review, even with no in-week facts", () => {
    const review = buildWeeklyReview({
      ledger: ledger(),
      facts: FACTS,
      findings: FINDINGS,
      weekStartMs: WEEK_START,
      weekEndMs: WEEK_END,
      generatedAt: iso(WEEK_END),
      reweight: null,
    });
    expect(review.pools.map((p) => p.pool)).toEqual(["alpha", "beta"]);
    expect(review.pools[1]?.week.forecastsScored).toBe(0);
    expect(review.pools[1]?.week.precision).toBeNull();
    expect(review.pools[1]?.week.recall).toBeNull();
    expect(review.pools[1]?.week.wallErrorMin).toBeNull();
  });
});
