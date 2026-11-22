import { describe, expect, test } from "bun:test";
import { computeSloBudget, SLO_DEFINITIONS, type SloDefinition, type SloSample } from "../src/slo.ts";

const MINUTE = 60_000;
const NOW = 30 * 24 * 60 * MINUTE;
const upFacet = SLO_DEFINITIONS.find((d) => d.id === "aggregator-up")!;
const ageFacet = SLO_DEFINITIONS.find((d) => d.id === "quota-age")!;

function sample(atMs: number, bad: boolean | null, value: number | null = null): SloSample {
  return { atMs, bad, value };
}

describe("SLO budget calculation", () => {
  test("charges missed up-facet samples as bad minutes", () => {
    const result = computeSloBudget(upFacet, [sample(NOW - MINUTE, false)], NOW);
    expect(result.badMinutes).toBe(30 * 24 * 60 - 1);
    expect(result.state).toBe("exhausted");
  });

  test("excludes and flags missing age samples instead of calling them good", () => {
    const result = computeSloBudget(ageFacet, [sample(NOW - MINUTE, false, 20 * MINUTE)], NOW);
    expect(result.badMinutes).toBe(0);
    expect(result.unknownSamples).toBeGreaterThan(0);
    expect(result.state).toBe("unknown");
    expect(result.budgetRemainingPercent).toBeNull();
  });

  test("double-weights feed samples beyond 30 minutes and reports current value", () => {
    const feed: SloDefinition = SLO_DEFINITIONS.find((d) => d.id === "feed-age")!;
    const result = computeSloBudget(feed, [sample(NOW - MINUTE, true, 31 * MINUTE)], NOW);
    expect(result.badMinutes).toBe(2);
    expect(result.currentValue).toBe(31 * MINUTE);
  });

  test("calculates one-hour burn against the window's per-hour budget", () => {
    const result = computeSloBudget(upFacet, [sample(NOW - MINUTE, true)], NOW);
    expect(result.burnRate1h).toBe(200); // the other 59 missing minutes in the hour are bad too
  });
});
