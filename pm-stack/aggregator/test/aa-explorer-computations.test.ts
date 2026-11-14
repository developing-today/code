import { describe, expect, test } from "bun:test";
import {
  applyInequalityFilters,
  bandIndex,
  chartClickSuppressed,
  clampChartView,
  computeRecommendations,
  effectiveChartView,
  gapMetrics,
  logEdges,
  panChartView,
  paretoSet,
  parseChartViewBox,
  pctRank,
  projectTask,
  resetChartView,
  scoreEdges,
  zoomChartView,
} from "../src/assets/aa-explorer-computations.js";

describe("Cost Explorer browser computations", () => {
  test("percentile ranks split ties and respect the better direction", () => {
    expect(pctRank([10, 20, 20, 30, null], 20, true)).toBe(50);
    expect(pctRank([10, 20, 30], 30, true)).toBe(100);
    expect(pctRank([10, 20, 30], 30, false)).toBe(0);
    expect(pctRank([20], 20, true)).toBeNull();
  });

  test("Pareto membership handles high/low dimensions, ties and missing measures", () => {
    const rows = [
      { key: "balanced", score: 10, cost: 1 },
      { key: "dominated", score: 9, cost: 2 },
      { key: "cheap", score: 8, cost: 0.5 },
      { key: "missing", score: 12, cost: null },
      { key: "same", score: 10, cost: 1 },
    ];
    const frontier = paretoSet(rows, [
      { get: (p) => p.score, better: "high" },
      { get: (p) => p.cost, better: "low" },
    ]);
    expect(frontier.map((p) => p.key)).toEqual(["balanced", "cheap", "same"]);
  });

  test("gap metrics drive AND/OR and keep-only-rejected filters", () => {
    const rows = [
      { key: "cheap", score: 80, cost: 0.5, time: 130 },
      { key: "leader", score: 95, cost: 1, time: 100 },
      { key: "luna", score: 90, cost: 1.25, time: 150 },
      { key: "unknown", score: 70, cost: null, time: null },
    ];
    expect(gapMetrics(rows).luna).toMatchObject({ gapScore: 5, gapCost: 1.25, gapSpeed: 5 });
    const conditions = [
      { metric: "gapScore", op: "<=", value: "5" },
      { metric: "gapCost", op: "<=", value: "1.25" },
    ];
    expect(applyInequalityFilters(rows, "include", "and", conditions).map((p) => p.key)).toEqual([
      "cheap",
      "leader",
      "luna",
    ]);
    expect(applyInequalityFilters(rows, "include", "or", [conditions[0]]).map((p) => p.key)).toEqual([
      "cheap",
      "leader",
      "luna",
    ]);
    expect(applyInequalityFilters(rows, "reject", "and", conditions).map((p) => p.key)).toEqual(["unknown"]);
  });

  test("band edges keep exact boundaries in one deterministic band", () => {
    const rows = [{ score: 10 }, { score: 20 }, { score: 30 }];
    const linear = scoreEdges(rows, 2);
    const logarithmic = logEdges([1, 100], 2);
    expect(linear).toEqual([10, 20, 30]);
    expect(logarithmic[0]).toBeCloseTo(1);
    expect(logarithmic[1]).toBeCloseTo(10);
    expect(logarithmic[2]).toBeCloseTo(100);
    expect(bandIndex(20, linear)).toBe(1);
    expect(bandIndex(30, linear)).toBe(1);
    expect(bandIndex(31, linear)).toBe(-1);
  });

  test("recommendation ranking keeps the hand-checked Luna task example", () => {
    const points = [
      { key: "low", score: 70, cost: 0.5, time: 500, creator: "Other lab" },
      { key: "luna", score: 95, cost: 0.106, time: 353, creator: "OpenAI" },
      { key: "fast", score: 100, cost: 0.2, time: 280, creator: "Other lab" },
    ];
    const recommendations = computeRecommendations(
      points,
      {
        allow: 5,
        speedPriority: 85,
        priceAllow: 500,
        slowAllow: 2,
        altWindow: 2,
        diversity: "none",
        dominance: "none",
        perBand: 3,
        maxCost: "",
        maxSec: "",
      },
      2,
      "Intelligence",
    );
    const lunaBand = recommendations.bands.find((band) => band.members.some((point) => point.key === "luna"));
    expect(recommendations.empty).toBeNull();
    expect(lunaBand?.picks.some((pick) => pick.p.key === "luna")).toBe(true);
    expect(lunaBand?.picks.map((pick) => pick.p.key)).toEqual(["fast", "luna"]);
    expect(points[1]).toMatchObject({ cost: 0.106, time: 353 });
  });

  test("task projection matches the recorded Luna-rate example and missing prices stay unknown", () => {
    const settings = {
      turns: 3,
      inputTokens: 20_000,
      answerTokens: 2_000,
      thinkingTokens: 4_000,
      cacheReadPct: 50,
      writesPct: 0,
      overheadSec: 10,
      window: "peak",
    };
    const luna = {
      name: "GPT-6 Luna",
      rate: { bands: [{ window: "peak", input: 0.75, cacheRead: 0.075, output: 4.5 }] },
    };
    const point = { family: "GPT-6 Luna", tps: 145, ttft: 93.76 };
    const result = projectTask(luna, settings, [point], []);
    expect(result.cost).toBeCloseTo(0.10575, 8);
    expect(result.time).toBeCloseTo(352.66, 1);
    expect(result.band).toBe(luna.rate.bands[0]);

    const missingCacheRate = { name: "GPT-6 Luna", rate: { bands: [{ window: "peak", input: 0.75, output: 4.5 }] } };
    expect(projectTask(missingCacheRate, settings, [point], []).cost).toBeNull();
  });
});

// Oracle: the viewBox arithmetic of PR #4154 as specified on issue
// #9581 — focal anchoring keeps the focal data point fixed, zoom is
// floored at 1/16 of the base width, zooming back out to the base
// width clears the zoom state, pan is clamped to the base viewBox,
// reset restores the base, and a click within 400 ms of a moved
// drag's pointerup stamp is swallowed while other clicks are not.
// These drive the same module functions the browser enhancer calls.
describe("aa chart zoom/pan viewBox behavior (#9581)", () => {
  const base = [0, 0, 1600, 900];
  const state = () => ({ base: base.slice(), cur: null as number[] | null });

  test("parseChartViewBox parses a viewBox and rejects malformed values", () => {
    expect(parseChartViewBox("0 0 1600 900")).toEqual([0, 0, 1600, 900]);
    expect(parseChartViewBox("10 20 30")).toBeNull();
    expect(parseChartViewBox("a b c d")).toBeNull();
    expect(parseChartViewBox("")).toBeNull();
    expect(parseChartViewBox(null)).toBeNull();
  });

  test("zoom anchors on the focal point and preserves aspect", () => {
    const st = state();
    zoomChartView(st, 0.5, 0.25, 0.5);
    // focal data point (400, 450) sits at the same fraction after zoom
    expect(st.cur).toEqual([200, 225, 800, 450]);
    expect(effectiveChartView(st)).toEqual([200, 225, 800, 450]);
    const centred = state();
    zoomChartView(centred, 0.5, null, null);
    expect(centred.cur).toEqual([400, 225, 800, 450]);
  });

  test("zoom is floored at 16x (base width / 16)", () => {
    const st = state();
    zoomChartView(st, 0.01, null, null);
    expect(st.cur).toEqual([750, 421.875, 100, 56.25]);
    zoomChartView(st, 0.01, null, null);
    expect(st.cur).toEqual([750, 421.875, 100, 56.25]);
  });

  test("zooming back out to the base width clears the zoom state", () => {
    const st = state();
    zoomChartView(st, 0.5, null, null);
    expect(st.cur).not.toBeNull();
    zoomChartView(st, 2, null, null);
    expect(st.cur).toBeNull();
    expect(effectiveChartView(st)).toEqual(base);
    const fresh = state();
    zoomChartView(fresh, 1.25, null, null);
    expect(fresh.cur).toBeNull();
  });

  test("pan scales pixels to viewBox units and clamps at the base edges", () => {
    const st = state();
    zoomChartView(st, 0.5, null, null); // cur [400, 225, 800, 450]
    expect(panChartView(st, 80, 45, 800, 450)).toBe(true);
    expect(st.cur).toEqual([320, 180, 800, 450]);
    // huge drags clamp to the base corners, never past them
    expect(panChartView(st, 100000, 100000, 800, 450)).toBe(true);
    expect(st.cur).toEqual([0, 0, 800, 450]);
    expect(panChartView(st, -100000, -100000, 800, 450)).toBe(true);
    expect(st.cur).toEqual([800, 450, 800, 450]);
  });

  test("pan is a no-op when not zoomed or the element has no size", () => {
    const st = state();
    expect(panChartView(st, 10, 10, 800, 450)).toBe(false);
    expect(st.cur).toBeNull();
    zoomChartView(st, 0.5, null, null);
    const before = (st.cur as number[]).slice();
    expect(panChartView(st, 10, 10, 0, 450)).toBe(false);
    expect(st.cur).toEqual(before);
  });

  test("clamp honours a non-zero base origin", () => {
    const st = { base: [100, 50, 1600, 900], cur: [0, 0, 800, 450] };
    clampChartView(st);
    expect(st.cur).toEqual([100, 50, 800, 450]);
    st.cur = [5000, 5000, 800, 450];
    clampChartView(st);
    expect(st.cur).toEqual([900, 500, 800, 450]);
  });

  test("reset restores the base viewBox after zoom + pan", () => {
    const st = state();
    zoomChartView(st, 0.5, 0.25, 0.5);
    panChartView(st, 40, 20, 800, 450);
    expect(effectiveChartView(st)).not.toEqual(base);
    resetChartView(st);
    expect(st.cur).toBeNull();
    expect(effectiveChartView(st)).toEqual(base);
  });

  test("click suppression: post-drag click swallowed, ordinary click passes", () => {
    const now = 1_700_000_000_000;
    // a moved drag stamps pointerup time; the click that follows it
    // inside the 400 ms window is the drag's tail — swallowed
    expect(chartClickSuppressed(String(now - 399), now)).toBe(true);
    expect(chartClickSuppressed(String(now), now)).toBe(true);
    // at/after the window the same stamp no longer suppresses
    expect(chartClickSuppressed(String(now - 400), now)).toBe(false);
    expect(chartClickSuppressed(String(now - 60_000), now)).toBe(false);
    // an ordinary click has no drag, hence no stamp — never swallowed
    expect(chartClickSuppressed(null, now)).toBe(false);
    expect(chartClickSuppressed("0", now)).toBe(false);
    expect(chartClickSuppressed("", now)).toBe(false);
  });
});
