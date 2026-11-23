import { describe, expect, test } from "bun:test";
import type {
  ForecastAccuracyLedger,
  PoolBaseline,
  WindowBaseline,
} from "./quota-forecast-accuracy.ts";
import {
  buildReweightProposals,
  renderEvidencePack,
  type RouteWeightTable,
} from "./quota-reweight.ts";

// Oracle: the Stage-3 design D5 (doc/2026-11-22T22-35-00Z_design_
// quota-plane-stage-3, #12700): re-weighting proposals are PRs
// carrying the full evidence pack, grounded in S2's accuracy
// record — and a pool/window the record cannot score never
// grounds a proposal. S2's honesty rules bind here verbatim:
// unscoreable stays unscoreable with its reasons, undefined
// precision/recall stays undefined, and the bare opencode-go
// pool's rows (a cross-workspace mixing artifact) never ground
// anything — the per-workspace rows are the real ones.

function win(over: Partial<WindowBaseline> = {}): WindowBaseline {
  return {
    window: "weekly",
    scoreable: true,
    unscoreableReasons: [],
    periodsTotal: 4,
    periodsScored: 4,
    periodsWalled: 3,
    periodsOpen: 0,
    periodsNoReset: 0,
    forecastsIssued: 300,
    forecastsScored: 300,
    forecastsInsufficient: 0,
    detectionsWhileWalled: 0,
    tp: 100,
    fp: 0,
    fn: 50,
    tn: 150,
    precision: 1,
    recall: 0.667,
    wallErrorMin: { count: 100, meanSignedMin: 1000, medianSignedMin: 1000, medianAbsMin: 1000, p90AbsMin: 2000 },
    firstCallLeadMin: null,
    ...over,
  };
}

function pool(serviceId: string, windows: WindowBaseline[]): PoolBaseline {
  return {
    pool: serviceId,
    providerId: null,
    serviceIdsJoined: [serviceId],
    windows,
    rollup: win({ window: "rollup" }),
  };
}

function ledger(pools: PoolBaseline[]): ForecastAccuracyLedger {
  return {
    version: 1,
    kind: "quota-forecast-accuracy",
    generatedAt: "2026-11-23T00:00:00.000Z",
    model: "quota-analytics.projectWindow pace v1 (design D2, via S1 #12695)",
    store: { records: 20663, pctRecords: 19735, fromIso: "2026-10-20T12:00:00.000Z", toIso: "2026-11-23T00:00:00.000Z" },
    pools,
  };
}

const GO_ROUTE: RouteWeightTable = {
  routeId: "opencode-go-plans",
  selection: "weighted",
  weights: {
    "opencode-go.drewrypope": 2,
    "opencode-go.popesdrew": 1,
    "opencode-go.developing-today-0": 1,
    "opencode-go.0x111dev": 1,
  },
  weightsLine:
    '        "weights": { "opencode-go.drewrypope": 2, "opencode-go.popesdrew": 1, "opencode-go.developing-today-0": 1, "opencode-go.0x111dev": 1 }',
};

describe("pool-weight class", () => {
  test("fires on a late-wall member with a headroom sibling, preserving total weight", () => {
    const run = buildReweightProposals({
      ledger: ledger([
        pool("opencode-go.drewrypope", [
          win({ forecastsScored: 300, precision: 1, recall: 0.5, wallErrorMin: { count: 40, meanSignedMin: 2000, medianSignedMin: 2000, medianAbsMin: 2000, p90AbsMin: 3000 } }),
        ]),
        pool("opencode-go.popesdrew", [
          win({ forecastsScored: 250, precision: 1, recall: 0.95, wallErrorMin: { count: 30, meanSignedMin: 100, medianSignedMin: 100, medianAbsMin: 100, p90AbsMin: 200 } }),
        ]),
        pool("opencode-go.developing-today-0", [win({ scoreable: false, forecastsScored: 0, unscoreableReasons: ["no percent readings for this pool/window in the store"] })]),
        pool("opencode-go.0x111dev", [win({ forecastsScored: 10 })]),
      ]),
      routes: [GO_ROUTE],
      nearPctCurrent: 85,
    });
    const p = run.proposals.find((x) => x.proposalClass === "pool-weight");
    expect(p).toBeDefined();
    expect(p?.key).toBe("pool-weight:opencode-go-plans:opencode-go.drewrypope:2->1.5");
    expect(p?.target.replace).toContain('"opencode-go.drewrypope": 1.5');
    expect(p?.target.replace).toContain('"opencode-go.popesdrew": 1.5');
    // gaps name the two members that could not ground
    const reasons = run.gaps.map((g) => g.reason);
    expect(reasons).toContain("unscoreable-window");
    expect(reasons).toContain("insufficient-scored-forecasts");
  });

  test("refuses when no sibling has headroom evidence", () => {
    const run = buildReweightProposals({
      ledger: ledger([
        pool("opencode-go.drewrypope", [win({ forecastsScored: 300, precision: 1, recall: 0.5 })]),
        pool("opencode-go.popesdrew", [win({ forecastsScored: 250, precision: 0.4, recall: 0.5 })]),
      ]),
      routes: [GO_ROUTE],
      nearPctCurrent: 85,
    });
    expect(run.proposals.filter((x) => x.proposalClass === "pool-weight")).toHaveLength(0);
    expect(run.examined.join("\n")).toContain("no weight proposal");
  });

  test("refuses an unscoreable binding window, carrying S2's reason verbatim", () => {
    const s2Reason = "every issued forecast was insufficient-history — the pace was never measurable in a completed period";
    const run = buildReweightProposals({
      ledger: ledger([
        pool("opencode-go.drewrypope", [win({ scoreable: false, forecastsScored: 0, unscoreableReasons: [s2Reason] })]),
        pool("opencode-go.popesdrew", [win({ forecastsScored: 250, precision: 1, recall: 0.95 })]),
      ]),
      routes: [GO_ROUTE],
      nearPctCurrent: 85,
    });
    expect(run.proposals.filter((x) => x.proposalClass === "pool-weight")).toHaveLength(0);
    const gap = run.gaps.find((g) => g.reason === "unscoreable-window");
    expect(gap?.detail).toContain(s2Reason);
  });

  test("refuses undefined precision/recall instead of reading them as numbers", () => {
    const run = buildReweightProposals({
      ledger: ledger([
        pool("opencode-go.drewrypope", [
          win({ forecastsScored: 300, precision: 0, recall: null, wallErrorMin: null, unscoreableReasons: ["recall undefined: no realized wall was scored"] }),
        ]),
        pool("opencode-go.popesdrew", [win({ forecastsScored: 250, precision: 1, recall: 0.95 })]),
      ]),
      routes: [GO_ROUTE],
      nearPctCurrent: 85,
    });
    expect(run.proposals.filter((x) => x.proposalClass === "pool-weight")).toHaveLength(0);
    const gap = run.gaps.find((g) => g.reason === "undefined-accuracy-metric");
    expect(gap?.detail).toContain("recall undefined");
  });

  test("a route member with no exact forecast pool is a gap, never a silent join", () => {
    const zen: RouteWeightTable = {
      routeId: "zen-plans",
      selection: "weighted",
      weights: { "opencode.drewrypope": 1, "opencode.popesdrew": 1 },
      weightsLine: '        "weights": { "opencode.drewrypope": 1, "opencode.popesdrew": 1 }',
    };
    const run = buildReweightProposals({
      ledger: ledger([pool("opencode-go.drewrypope", [win({ forecastsScored: 300 })])]),
      routes: [zen],
      nearPctCurrent: 85,
    });
    expect(run.proposals.filter((x) => x.proposalClass === "pool-weight")).toHaveLength(0);
    expect(run.gaps.filter((g) => g.reason === "no-forecast-pool-for-member")).toHaveLength(2);
  });
});

describe("caution-margin class", () => {
  const anthropic = () =>
    pool("anthropic:claude-pro", [
      win({ window: "5h", forecastsScored: 305, precision: 0.593, recall: 0.44 }),
      win({ forecastsScored: 273, precision: 1, recall: 0.582, wallErrorMin: { count: 159, meanSignedMin: 900, medianSignedMin: 795, medianAbsMin: 2204, p90AbsMin: 4000 } }),
    ]);
  const meta = () =>
    pool("meta", [
      win({ forecastsScored: 269, precision: 0.529, recall: 0.647, wallErrorMin: { count: 110, meanSignedMin: 3194, medianSignedMin: 4741, medianAbsMin: 4741, p90AbsMin: 6000 } }),
    ]);

  test("fires when weekly walls arrive early across pools with a precision anchor", () => {
    const run = buildReweightProposals({
      ledger: ledger([
        anthropic(),
        meta(),
        pool("_openai.alt", [win({ forecastsScored: 169, precision: 1, recall: 0.958, wallErrorMin: { count: 60, meanSignedMin: -100, medianSignedMin: -155, medianAbsMin: 370, p90AbsMin: 900 } })]),
      ]),
      routes: [],
      nearPctCurrent: 85,
    });
    const p = run.proposals.find((x) => x.proposalClass === "caution-margin");
    expect(p).toBeDefined();
    expect(p?.key).toBe("caution-margin:NEAR_PCT:85->90");
    expect(p?.target.file).toBe("pm-stack/aggregator/src/derive.ts");
    expect(p?.target.find).toBe("export const NEAR_PCT = 85;");
    expect(p?.target.replace).toBe("export const NEAR_PCT = 90;");
    // the pack carries the grounding rows' numbers verbatim
    const pack = renderEvidencePack(p as NonNullable<typeof p>);
    expect(pack).toContain("anthropic:claude-pro");
    expect(pack).toContain("+795");
    expect(pack).toContain("4741");
  });

  test("does not fire on a single qualifying pool", () => {
    const run = buildReweightProposals({ ledger: ledger([anthropic()]), routes: [], nearPctCurrent: 85 });
    expect(run.proposals.filter((x) => x.proposalClass === "caution-margin")).toHaveLength(0);
  });

  test("does not fire when strong counter-evidence exists", () => {
    const run = buildReweightProposals({
      ledger: ledger([
        anthropic(),
        meta(),
        pool("opendesign", [win({ forecastsScored: 200, precision: 1, recall: 0.9, wallErrorMin: { count: 50, meanSignedMin: -2000, medianSignedMin: -2000, medianAbsMin: 2000, p90AbsMin: 3000 } })]),
      ]),
      routes: [],
      nearPctCurrent: 85,
    });
    expect(run.proposals.filter((x) => x.proposalClass === "caution-margin")).toHaveLength(0);
    expect(run.gaps.some((g) => g.reason === "counter-evidence-present")).toBe(true);
  });

  test("the bare opencode-go pool never grounds the margin, even with the strongest numbers", () => {
    const run = buildReweightProposals({
      ledger: ledger([
        anthropic(),
        pool("opencode-go", [
          win({ forecastsScored: 711, precision: 1, recall: 0.454, wallErrorMin: { count: 300, meanSignedMin: 1903, medianSignedMin: 1903, medianAbsMin: 2961, p90AbsMin: 5000 } }),
          win({ window: "monthly", forecastsScored: 513, precision: 0.054, recall: 0.929 }),
        ]),
      ]),
      routes: [],
      nearPctCurrent: 85,
    });
    // anthropic alone cannot fire the rule; the bare pool is excluded, not counted
    expect(run.proposals.filter((x) => x.proposalClass === "caution-margin")).toHaveLength(0);
    expect(run.gaps.some((g) => g.reason === "aggregate-pool-excluded")).toBe(true);
  });

  test("refuses an unscoreable weekly baseline as grounding (named gap via the volume gate)", () => {
    const run = buildReweightProposals({
      ledger: ledger([
        anthropic(),
        pool("_antigravity.claude-gpt", [
          win({ scoreable: false, forecastsScored: 0, precision: null, recall: null, wallErrorMin: null, unscoreableReasons: ["every issued forecast was insufficient-history — the pace was never measurable in a completed period"] }),
        ]),
      ]),
      routes: [],
      nearPctCurrent: 85,
    });
    expect(run.proposals.filter((x) => x.proposalClass === "caution-margin")).toHaveLength(0);
  });
});

describe("evidence pack honesty", () => {
  test("absent forecast/shadow inputs are named in the pack, never papered over", () => {
    const run = buildReweightProposals({
      ledger: ledger([
        pool("anthropic:claude-pro", [win({ forecastsScored: 273, precision: 1, recall: 0.582, wallErrorMin: { count: 159, meanSignedMin: 900, medianSignedMin: 795, medianAbsMin: 2204, p90AbsMin: 4000 } })]),
        pool("meta", [win({ forecastsScored: 269, precision: 0.529, recall: 0.647, wallErrorMin: { count: 110, meanSignedMin: 3194, medianSignedMin: 4741, medianAbsMin: 4741, p90AbsMin: 6000 } })]),
      ]),
      routes: [],
      nearPctCurrent: 85,
    });
    const p = run.proposals[0];
    expect(p).toBeDefined();
    const pack = renderEvidencePack(p as NonNullable<typeof p>);
    expect(pack).toContain("no forecast file supplied");
    expect(pack).toContain("no shadow record supplied");
    expect(pack).toContain("does not\n> merge");
  });
});
