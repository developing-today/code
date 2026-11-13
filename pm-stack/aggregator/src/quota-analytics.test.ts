import { describe, expect, test } from "bun:test";
import {
  fmtDuration,
  projectWindow,
  tokenRatios,
  unlockPicture,
  usFederalHoliday,
  wowDelta,
  type PctPoint,
} from "./quota-analytics.ts";

const H = 3_600_000;
const NOW = Date.parse("2026-11-13T12:00:00Z");
const RESET = "2026-11-17T21:22:46.000Z"; // _openai.main weekly reset

function series(pcts: number[], startAgoH: number, resetsAt: string | null = RESET): PctPoint[] {
  return pcts.map((pct, i) => ({
    ts: NOW - startAgoH * H + (i * (startAgoH * H)) / Math.max(1, pcts.length - 1),
    pct,
    resetsAt,
  }));
}

describe("projectWindow", () => {
  test("already walled: state walled, zero duration, walls before reset", () => {
    const p = projectWindow({ points: [], pctNow: 100, resetsAt: RESET, now: NOW });
    expect(p.state).toBe("walled");
    expect(p.durationUntilWallMs).toBe(0);
    expect(p.wallsBeforeReset).toBe(true);
  });

  test("steady measured burn projects the wall before the reset", () => {
    // 50% -> 75% over the last 10 hours = 2.5 pts/h; 25 points remain
    // => wall in 10h, well before the reset ~4.4 days out.
    const p = projectWindow({ points: series([50, 62.5, 75], 10), pctNow: 75, resetsAt: RESET, now: NOW });
    expect(p.state).toBe("projected");
    expect(p.ratePtsPerHour).toBeCloseTo(2.5, 5);
    expect(p.durationUntilWallMs).toBeCloseTo(10 * H, -4);
    expect(p.projectedWallAt).toBeCloseTo(NOW + 10 * H, -4);
    expect(p.wallsBeforeReset).toBe(true);
    expect(p.samples).toBe(3);
  });

  test("slow burn that would wall only after the reset is no-wall-before-reset", () => {
    // 0.1 pts/h from 90%: wall in 100h, reset in ~105h... use a reset
    // 24h out so the wall clearly lands after it.
    const reset = new Date(NOW + 24 * H).toISOString();
    const pts = series([88, 89, 90], 20, reset);
    const p = projectWindow({ points: pts, pctNow: 90, resetsAt: reset, now: NOW });
    expect(p.ratePtsPerHour).toBeCloseTo(0.1, 5);
    expect(p.state).toBe("no-wall-before-reset");
    expect(p.wallsBeforeReset).toBe(false);
    expect(p.projectedWallAt).toBeGreaterThan(NOW + 24 * H);
  });

  test("flat burn never walls", () => {
    const p = projectWindow({ points: series([40, 40, 40], 6), pctNow: 40, resetsAt: RESET, now: NOW });
    expect(p.state).toBe("no-wall-before-reset");
    expect(p.projectedWallAt).toBeNull();
    expect(p.reason).toContain("flat");
  });

  test("one reading is insufficient history, never a guess", () => {
    const p = projectWindow({ points: series([80], 1), pctNow: 80, resetsAt: RESET, now: NOW });
    expect(p.state).toBe("insufficient-history");
    expect(p.projectedWallAt).toBeNull();
    expect(p.samples).toBe(1);
  });

  test("readings from a previous reset period do not join the fit", () => {
    const old = series([10, 90], 30, "2026-11-10T21:22:46.000Z");
    const p = projectWindow({ points: old, pctNow: 75, resetsAt: RESET, now: NOW });
    expect(p.state).toBe("insufficient-history");
    expect(p.samples).toBe(0);
  });

  test("reset instants jittered by seconds still count as one period", () => {
    // The Claude gatherer reports the same reset as ...59.304Z,
    // ...00.255Z etc. on consecutive reads; the fit must not split.
    const pts: PctPoint[] = [
      { ts: NOW - 10 * H, pct: 50, resetsAt: "2026-11-17T21:22:45.900Z" },
      { ts: NOW - 5 * H, pct: 62.5, resetsAt: "2026-11-17T21:22:46.400Z" },
      { ts: NOW, pct: 75, resetsAt: "2026-11-17T21:22:46.000Z" },
    ];
    const p = projectWindow({ points: pts, pctNow: 75, resetsAt: RESET, now: NOW });
    expect(p.state).toBe("projected");
    expect(p.samples).toBe(3);
  });

  test("readings packed inside 10 minutes are insufficient", () => {
    const pts: PctPoint[] = [
      { ts: NOW - 5 * 60_000, pct: 70, resetsAt: RESET },
      { ts: NOW - 60_000, pct: 71, resetsAt: RESET },
    ];
    const p = projectWindow({ points: pts, pctNow: 71, resetsAt: RESET, now: NOW });
    expect(p.state).toBe("insufficient-history");
  });
});

describe("wowDelta", () => {
  test("absolute and percent", () => {
    expect(wowDelta(150, 100)).toEqual({ abs: 50, pct: 50 });
    expect(wowDelta(80, 100)).toEqual({ abs: -20, pct: -20 });
  });
  test("zero previous yields null pct, not Infinity", () => {
    expect(wowDelta(10, 0)).toEqual({ abs: 10, pct: null });
    expect(wowDelta(0, 0)).toEqual({ abs: 0, pct: null });
  });
});

describe("tokenRatios", () => {
  test("in-per-out and cache share", () => {
    const r = tokenRatios({ input: 1000, output: 250, reasoning: 0, cacheRead: 3000, cacheWrite: 0 });
    expect(r.inPerOut).toBe(4);
    expect(r.cacheSharePct).toBe(75);
    expect(r.total).toBe(4250);
  });
  test("no output / no input side yields nulls", () => {
    const r = tokenRatios({ input: 0, output: 0, reasoning: 0, cacheRead: 0, cacheWrite: 0 });
    expect(r.inPerOut).toBeNull();
    expect(r.cacheSharePct).toBeNull();
    expect(r.total).toBe(0);
  });
});

describe("unlockPicture", () => {
  const wins = [
    { name: "weekly", percent: 100, resetsAt: "2026-11-17T21:22:46.000Z" },
    { name: "5-hour", percent: 0, resetsAt: "2026-11-13T12:30:00.000Z" },
    { name: "monthly", percent: 100, resetsAt: "2026-12-03T00:00:00.000Z" },
  ];
  test("what frees at the binding reset and what stays walled", () => {
    const u = unlockPicture(wins, wins[0]);
    expect(u.bindingResetsAt).toBe("2026-11-17T21:22:46.000Z");
    expect(u.freesAtReset).toEqual(["weekly"]);
    expect(u.stillWalledAfter).toEqual([{ name: "monthly", resetsAt: "2026-12-03T00:00:00.000Z" }]);
  });
  test("a walled window with no reset stays walled", () => {
    const ws = [...wins, { name: "api", percent: 100, resetsAt: null }];
    const u = unlockPicture(ws, wins[0]);
    expect(u.stillWalledAfter.map((w) => w.name)).toContain("api");
  });
});

describe("fmtDuration + holidays", () => {
  test("durations", () => {
    expect(fmtDuration(null)).toBe("—");
    expect(fmtDuration(0)).toBe("now");
    expect(fmtDuration(42 * 60_000)).toBe("42m");
    expect(fmtDuration(5 * H + 12 * 60_000)).toBe("5h 12m");
    expect(fmtDuration(4 * 24 * H + 8 * H)).toBe("4d 8h");
  });
  test("holidays are labelled, ordinary days are not", () => {
    expect(usFederalHoliday("2026-11-26")).toBe("Thanksgiving");
    expect(usFederalHoliday("2026-11-11")).toBe("Veterans Day");
    expect(usFederalHoliday("2026-11-13")).toBeNull();
  });
});
