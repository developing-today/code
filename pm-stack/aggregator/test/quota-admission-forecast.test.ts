import { describe, expect, test } from "bun:test";
import { type AdmissionInput, buildQuotaAdmission, type Forecast, type Observation } from "../src/quota-admission.ts";

// Oracle: Stage-3 design D4 (#12698, epic #12694) — a window whose
// forecast says projected with wallsBeforeReset and a projected wall at
// least the configured margin before the reset yields pace_hold — plus
// #12698 criterion 3: no decision is ever taken on a stale or
// honesty-flagged forecast (stale forecast = current behaviour). The
// forecast may only ever add a pace_hold to an otherwise eligible
// window; it never overrides a measured verdict and never manufactures
// evidence for an unknown window.
const now = 10_000_000;
const scope = { provider: "go", account: "a", bucket: "plan" };
const resetsAt = now + 86_400_000;
const marginMs = 3_600_000;

const read = (patch: Partial<Observation> = {}): Observation => ({
  ...scope,
  kind: "monthly",
  period: "p1",
  id: "new",
  source: "fixture",
  measuredAt: now,
  resetsAt,
  percent: 50,
  state: "ok",
  denominator: "100",
  ...patch,
});

const fc = (patch: Partial<Forecast> = {}): Forecast => ({
  ...scope,
  kind: "monthly",
  period: "p1",
  id: "fc-1",
  measuredAt: now,
  resetsAt,
  state: "projected",
  projectedWallAt: now + 3_600_000,
  wallsBeforeReset: true,
  ...patch,
});

function fixture(patch: Partial<AdmissionInput> = {}, windowPatch: Record<string, unknown> = {}): AdmissionInput {
  return {
    generation: "fixture-v1",
    bindings: [
      {
        ...scope,
        aliases: ["cred-a"],
        windows: [
          {
            kind: "monthly",
            period: "p1",
            resetsAt,
            freshnessMs: 1_200_000,
            collectionMs: 900_000,
            forecastMarginMs: marginMs,
            ...windowPatch,
          },
        ],
      },
    ],
    observations: [read()],
    stops: [],
    priorWalls: [],
    forecasts: [fc()],
    ...patch,
  };
}

const account = (f: AdmissionInput) => buildQuotaAdmission(f, now).accounts[0];
const window = (f: AdmissionInput) => account(f).windows[0];

describe("quota admission forecast input: oracle = design D4 + #12698 criterion 3", () => {
  test("projected wall at least the margin before reset yields pace_hold", () => {
    const f = fixture();
    expect(account(f).verdict).toBe("pace_hold");
    expect(window(f).reasons).toContain("forecast_projected_wall");
    expect(window(f).evidenceIds).toContain("fc-1");
  });

  test("wall exactly at the margin holds (at least the configured margin)", () => {
    const f = fixture({ forecasts: [fc({ projectedWallAt: resetsAt - marginMs })] });
    expect(account(f).verdict).toBe("pace_hold");
  });

  test("wall inside the margin does not hold", () => {
    const f = fixture({ forecasts: [fc({ projectedWallAt: resetsAt - marginMs + 1 })] });
    expect(account(f).verdict).toBe("eligible");
    expect(window(f).reasons).toEqual(["fresh_window"]);
  });

  test("no forecast input is byte-identical to the pre-S4 projection", () => {
    const without = fixture();
    delete (without as { forecasts?: unknown }).forecasts;
    (without.bindings[0].windows[0] as { forecastMarginMs?: number }).forecastMarginMs = undefined;
    const a = buildQuotaAdmission(without, now);
    const b = fixture({ forecasts: [] });
    delete (b.bindings[0].windows[0] as { forecastMarginMs?: number }).forecastMarginMs;
    expect(buildQuotaAdmission(b, now)).toEqual(a);
    expect(account(without).verdict).toBe("eligible");
  });

  test("window without forecastMarginMs ignores forecasts (per-window opt-in)", () => {
    const f = fixture();
    delete (f.bindings[0].windows[0] as { forecastMarginMs?: number }).forecastMarginMs;
    expect(account(f).verdict).toBe("eligible");
  });

  test("honesty states never hold: insufficient-history, no-wall-before-reset, walled forecast", () => {
    for (const patch of [
      { state: "insufficient-history" as const, projectedWallAt: null, wallsBeforeReset: null },
      { state: "no-wall-before-reset" as const, projectedWallAt: resetsAt + 3_600_000, wallsBeforeReset: false },
      { state: "walled" as const, projectedWallAt: now, wallsBeforeReset: true },
    ]) {
      const f = fixture({ forecasts: [fc(patch)] });
      expect(account(f).verdict).toBe("eligible");
      expect(window(f).reasons).toEqual(["fresh_window"]);
    }
  });

  test("stale forecast = current behaviour (criterion 3)", () => {
    const f = fixture({ forecasts: [fc({ measuredAt: now - 1_200_001 })] });
    expect(account(f).verdict).toBe("eligible");
    expect(window(f).reasons).toEqual(["fresh_window"]);
    expect(window(f).evidenceIds).not.toContain("fc-1");
  });

  test("forecast projected from a superseded reading does not hold", () => {
    const f = fixture({
      observations: [
        read({ id: "old", measuredAt: now - 900_000, percent: 40 }),
        read({ id: "new", measuredAt: now, percent: 50 }),
      ],
      forecasts: [fc({ measuredAt: now - 900_000 })],
    });
    expect(account(f).verdict).toBe("eligible");
  });

  test("forecast never overrides a measured wall", () => {
    const f = fixture({ observations: [read({ percent: 100 })] });
    expect(account(f).verdict).toBe("walled");
    expect(window(f).reasons).toEqual(["known_100"]);
  });

  test("forecast never manufactures evidence for an unknown window", () => {
    const f = fixture({ observations: [] });
    expect(account(f).verdict).toBe("unknown");
    expect(window(f).reasons).toEqual(["missing_observation"]);
  });

  test("forecast for a different period is ignored", () => {
    const f = fixture({ forecasts: [fc({ period: "p2" })] });
    expect(account(f).verdict).toBe("eligible");
  });

  test("conflicting duplicate forecast ids are ignored, byte-equivalent duplicates still hold", () => {
    const conflict = fixture({ forecasts: [fc(), fc({ projectedWallAt: now + 7_200_000 })] });
    expect(account(conflict).verdict).toBe("eligible");
    const dup = fixture({ forecasts: [fc(), fc()] });
    expect(account(dup).verdict).toBe("pace_hold");
  });

  test("forecast does not stack reasons on a measured pace_hold", () => {
    const f = fixture({}, { reservePct: 40 });
    expect(account(f).verdict).toBe("pace_hold");
    expect(window(f).reasons).toEqual(["reserve_threshold"]);
  });

  test("invalid forecastMarginMs is an invalid window policy", () => {
    const f = fixture({}, { forecastMarginMs: -1 });
    expect(account(f).verdict).toBe("unknown");
    expect(window(f).reasons).toEqual(["invalid_window_policy"]);
  });
});
