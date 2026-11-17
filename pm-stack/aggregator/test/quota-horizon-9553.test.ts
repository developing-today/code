// lane-scratch: code-c /tmp/lane-code-c-20261114-1635 task 16104
import { describe, expect, test } from "bun:test";
import { analyticsSectionHtml, resetHorizonEvents } from "../src/quota-analytics-view.ts";
import { fmtCentral } from "../src/render.ts";
import { formatResetCompact } from "../src/notifications.ts";
import type { QuotaAnalytics } from "../src/analytics-store.ts";
import { emptyBurn } from "../src/analytics-store.ts";

// Sat 2026-11-14 18:00Z = 12:00 PM CST.
const NOW = Date.parse("2026-11-14T18:00:00Z");

function analytics(): QuotaAnalytics {
  const win = (name: string, percent: number, resetsAt: string | null) => ({
    name,
    percent,
    resetsAt,
    durationUntilResetMs: resetsAt ? Date.parse(resetsAt) - NOW : null,
    projection: {
      state: "no-wall-before-reset" as const,
      projectedWallAt: null,
      durationUntilWallMs: null,
      ratePtsPerHour: null,
      samples: 0,
    },
    wowDurationUntilWall: null,
  });
  return {
    generatedAt: new Date(NOW).toISOString(),
    now: NOW,
    dbAvailable: true,
    attemptsFrom: null,
    attemptsTo: null,
    credentialCoveragePct: null,
    totals: { lastHour: emptyBurn(), today: emptyBurn(), last7d: emptyBurn(), prev7d: emptyBurn() },
    totalsWowTokens: null,
    byModel7d: [],
    byEffort7d: [],
    byLeg7d: [],
    hourly7d: [],
    dowHistory: [],
    daily21d: [],
    activity7d: null,
    pools: [
      {
        poolId: "pool-w",
        provider: "ProvW",
        plan: "Max",
        state: "healthy",
        bindingWindow: "weekly",
        unlock: null,
        burnToday: null,
        burn7d: null,
        burnWowTokens: null,
        legBasis: null,
        legLabel: null,
        windows: [win("weekly", 73, "2026-11-20T14:00:00Z"), win("monthly", 41, "2026-12-01T06:00:00Z")],
      },
      {
        poolId: "pool-h",
        provider: "ProvH",
        plan: "Pro",
        state: "healthy",
        bindingWindow: "5-hour",
        unlock: null,
        burnToday: null,
        burn7d: null,
        burnWowTokens: null,
        legBasis: null,
        legLabel: null,
        windows: [win("5-hour", 12, "2026-11-14T20:00:00Z"), win("balance", 5, null)],
      },
    ],
  } as unknown as QuotaAnalytics;
}

describe("reset horizon (#9553)", () => {
  test("weekly window steps -3d..+35d from the recorded reset", () => {
    const ev = resetHorizonEvents(analytics()).filter((e) => e.window === "weekly");
    expect(ev.map((e) => new Date(e.at).toISOString())).toEqual([
      "2026-11-13T14:00:00.000Z",
      "2026-11-20T14:00:00.000Z",
      "2026-11-27T14:00:00.000Z",
      "2026-12-04T14:00:00.000Z",
      "2026-12-11T14:00:00.000Z",
      "2026-12-18T14:00:00.000Z",
    ]);
    expect(ev.find((e) => e.recorded)?.at).toBe(Date.parse("2026-11-20T14:00:00Z"));
  });

  test("monthly window reaches into next month, past step outside -3d excluded", () => {
    const ev = resetHorizonEvents(analytics()).filter((e) => e.window === "monthly");
    expect(ev.map((e) => new Date(e.at).toISOString())).toEqual(["2026-12-01T06:00:00.000Z"]);
  });

  test("sub-daily window is thinned to the neighbourhood of now; no-reset window contributes nothing", () => {
    const all = resetHorizonEvents(analytics());
    const five = all.filter((e) => e.window === "5-hour");
    expect(five.map((e) => new Date(e.at).toISOString())).toEqual([
      "2026-11-14T05:00:00.000Z",
      "2026-11-14T10:00:00.000Z",
      "2026-11-14T15:00:00.000Z",
      "2026-11-14T20:00:00.000Z",
    ]);
    expect(five[0].thinned).toBe(true);
    expect(all.some((e) => e.window === "balance")).toBe(false);
  });

  test("section renders the horizon in Central with no raw UTC ISO stamps", () => {
    const html = analyticsSectionHtml(analytics(), (n) => n);
    expect(html).toContain('id="analytics-horizon"');
    expect(html).toContain("past 3 days to next 35 days");
    expect(html).toContain("America/Chicago");
    expect(html).toContain("Fri, Nov 13"); // past weekly reset, Central rendering
    expect(html).toContain("Fri, Dec 18"); // +34d weekly reset
    expect(html).toContain("Tue, Dec 1"); // monthly reset into December
    expect(html).not.toContain("2026-11-20T14:00");
    expect(html).not.toContain("14:00:00Z");
  });
});

describe("Central stamps (#9553)", () => {
  test("fmtCentral renders America/Chicago, never UTC ISO", () => {
    expect(fmtCentral("2026-11-13T14:28:00Z")).toBe("Fri, Nov 13, 8:28 AM CST");
    expect(fmtCentral(null)).toBe("—");
  });

  test("push reset stamp is Central", () => {
    expect(formatResetCompact(Date.parse("2026-11-13T14:28:00Z"))).toBe("Fri 8:28 AM CST");
    expect(formatResetCompact(Date.parse("2026-07-13T14:28:00Z"))).toBe("Mon 9:28 AM CDT");
  });
});
