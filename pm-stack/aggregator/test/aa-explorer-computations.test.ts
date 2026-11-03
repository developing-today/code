import { describe, expect, test } from "bun:test";
import {
  applyInequalityFilters,
  bandIndex,
  computeRecommendations,
  gapMetrics,
  logEdges,
  paretoSet,
  pctRank,
  projectTask,
  scoreEdges,
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
    expect(applyInequalityFilters(rows, "include", "and", conditions).map((p) => p.key)).toEqual(["cheap", "leader", "luna"]);
    expect(applyInequalityFilters(rows, "include", "or", [conditions[0]]).map((p) => p.key)).toEqual(["cheap", "leader", "luna"]);
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
      { allow: 5, speedPriority: 85, priceAllow: 500, slowAllow: 2, altWindow: 2, diversity: "none", dominance: "none", perBand: 3, maxCost: "", maxSec: "" },
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
