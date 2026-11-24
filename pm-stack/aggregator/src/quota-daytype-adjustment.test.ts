import { describe, expect, test } from "bun:test";
import {
  assessSpikeDay,
  buildDayTypeShadowRecords,
  dayTypeShadowSubjectKey,
  measureCellBurnRates,
  renderDayTypeShadowReport,
} from "./quota-daytype-adjustment.ts";
import { indexPercentRecords, pointsForWindow } from "./quota-forecast-accuracy.ts";
import type { PctPoint } from "./quota-analytics.ts";
import type { QuotaForecastFile, QuotaForecastPool, QuotaForecastWindow } from "./quota-forecast.ts";
import type { SnapshotRecord } from "./model.ts";

// Oracle: the C4 design (doc/2026-11-13T15-12-00Z_design_reporting-
// architecture-event-driven/c4-forecasting-design.md §5) — the
// day-type adjustment multiplies the v1 pace by the cell's
// measured ratio vs the pool's overall pace; the spike-day shape
// is trailing-7d ~10% light vs trailing-30d predicting a +20%
// spike day, stated explicitly; admission is the §5 gate (≥8
// independent samples, counted as criterion 1 counted them, PR
// #13290); until an adjustment beats the unadjusted forecast in
// the S2 harness it publishes as a shadow record only. Fixture
// arithmetic is hand-derived in each test's comment.

const H = 3_600_000;
/** Chicago wall-clock instant: day 0 = Monday 2026-11-09; Chicago
 * is CST (UTC−6) for all of November 2026 after Nov 1, so Chicago
 * midnight is 06:00Z. */
function chi(day: number, hour: number): number {
  return Date.UTC(2026, 10, 9, 6, 0, 0) + day * 24 * H + hour * H;
}
const RESET_ISO = "2026-12-05T06:00:00.000Z";
const READING_HOURS = [1, 7, 13, 19];

/** Readings at 01/07/13/19 Chicago each day; the reading at
 * (day, hour) realizes the interval ending there, sized by
 * deltaFor(day, hour). The first reading is the baseline. */
function series(days: number, deltaFor: (day: number, hour: number) => number): SnapshotRecord[] {
  const out: SnapshotRecord[] = [];
  let pct = 5;
  for (let d = 0; d < days; d++) {
    for (const h of READING_HOURS) {
      if (!(d === 0 && h === 1)) pct += deltaFor(d, h);
      out.push({
        ts: new Date(chi(d, h)).toISOString(),
        serviceId: "_claude",
        metricId: "weekly",
        value: pct,
        unit: "pct",
        limit: 100,
        resetsAt: RESET_ISO,
        source: "api",
        observedBy: "test",
      });
    }
  }
  return out;
}

function pointsOf(records: SnapshotRecord[]): PctPoint[] {
  const { index } = indexPercentRecords(records);
  return pointsForWindow(index, "_claude", "weekly");
}

function fwindow(over: Partial<QuotaForecastWindow> = {}): QuotaForecastWindow {
  return {
    name: "weekly",
    state: "projected",
    ratePtsPerHour: 0.25,
    samples: 30,
    projectedWallAt: null,
    durationUntilWallMs: null,
    wallsBeforeReset: true,
    pctNow: 50,
    measuredAt: new Date(chi(9, 8)).toISOString(),
    resetsAt: RESET_ISO,
    ...over,
  };
}

function fforecast(windows: QuotaForecastWindow[], generatedAtMs: number): QuotaForecastFile {
  const pool: QuotaForecastPool = { providerId: "claude", serviceId: "_claude", plan: null, windows };
  return { version: 1, generatedAt: new Date(generatedAtMs).toISOString(), source: "gather-quota", pools: { claude: [pool] } };
}

// The shared fixture: 10 days, the interval ending 07:00 Chicago
// (morning) burns Δ2 per 6h, every other interval Δ1 per 6h.
// Intervals: 39. Morning: 10 (ends 07:00, days 0–9). Overall rate
// = 49 pts / 234 h; morning rate = 20 pts / 60 h = 1/3; ratio
// = (1/3) / (49/234) = 234/147.
const MORNING_DOUBLE = (d: number, h: number) => (h === 7 ? 2 : 1);

describe("measureCellBurnRates (oracle: design §5 ratio definition)", () => {
  test("cell ratio is the cell's interval rate over the overall interval rate", () => {
    const rates = measureCellBurnRates(pointsOf(series(10, MORNING_DOUBLE)));
    expect(rates.overallRatePtsPerHour).toBeCloseTo(49 / 234, 12);
    const morning = rates.cells.find((c) => c.grain === "daypart" && c.cell === "morning");
    expect(morning?.samples).toBe(10);
    expect(morning?.ratePtsPerHour).toBeCloseTo(1 / 3, 12);
    expect(morning?.ratio).toBeCloseTo(234 / 147, 12);
  });

  test("samples are distinct Chicago dates with a reading in the cell (criterion 1's unit)", () => {
    const rates = measureCellBurnRates(pointsOf(series(10, MORNING_DOUBLE)));
    const weekday = rates.cells.find((c) => c.grain === "dayclass" && c.cell === "weekday");
    // Nov 9–18: weekday dates are days 0–4 and 7–9 = 8 dates.
    expect(weekday?.samples).toBe(8);
    const weekend = rates.cells.find((c) => c.grain === "dayclass" && c.cell === "weekend");
    expect(weekend?.samples).toBe(2);
    // Dayclass-weekday ratio, hand-derived: intervals ending on a
    // weekday date carry Δ39 over 186 h; overall is 49 over 234 h.
    expect(weekday?.ratio).toBeCloseTo(9126 / 9114, 9);
  });
});

describe("buildDayTypeShadowRecords — day-type (oracle: design §5)", () => {
  test("an admitted current cell adjusts the v1 pace by its measured ratio", () => {
    const nowMs = chi(9, 8); // Wed Nov 18, 08:00 Chicago — morning.
    const forecast = fforecast([fwindow({ projectedWallAt: nowMs + 200 * H })], nowMs);
    const records = buildDayTypeShadowRecords(forecast, series(10, MORNING_DOUBLE), new Map());
    expect(records.length).toBe(1);
    const r = records[0] as NonNullable<(typeof records)[number]>;
    expect(r.state).toBe("shadow");
    expect(r.shadow).toBe(true);
    expect(r.methodology_version).toBe("c4-meth:5");
    expect(r.day_type?.grain).toBe("daypart");
    expect(r.day_type?.cell).toBe("morning");
    expect(r.day_type?.samples).toBe(10);
    expect(r.day_type?.ratio).toBeCloseTo(234 / 147, 12);
    expect(r.day_type?.ratePtsPerHour).toBeCloseTo(0.25 * (234 / 147), 12);
    // Adjusted wall: now + (100 − 50) / adjusted rate.
    expect(r.day_type?.wallAt).toBe(new Date(nowMs + (50 / (0.25 * (234 / 147))) * H).toISOString());
    expect(r.inputs.adjustments).toEqual(["day_type:daypart:morning"]);
    expect(r.unadjusted.ratePtsPerHour).toBe(0.25);
    expect(r.unadjusted.projectedWallAt).toBe(new Date(nowMs + 200 * H).toISOString());
    expect(r.spike_day).toBeNull();
    expect(r.supersedes).toBeNull();
  });

  test("the finest admitted grain wins over a coarser admitted grain", () => {
    // Dayclass-weekday is also admitted in this fixture (8 samples,
    // ratio ≈ 1.001) — the daypart cell (ratio 234/147) must win.
    const nowMs = chi(9, 8);
    const forecast = fforecast([fwindow({ projectedWallAt: nowMs + 200 * H })], nowMs);
    const [r] = buildDayTypeShadowRecords(forecast, series(10, MORNING_DOUBLE), new Map());
    expect(r?.day_type?.grain).toBe("daypart");
  });

  test("below the gate at every grain the record is adjustment_unavailable with the counts", () => {
    const nowMs = chi(6, 8); // Sun Nov 15, 08:00 Chicago — morning.
    const forecast = fforecast([fwindow({ projectedWallAt: nowMs + 200 * H })], nowMs);
    const [r] = buildDayTypeShadowRecords(forecast, series(7, MORNING_DOUBLE), new Map());
    expect(r?.state).toBe("adjustment_unavailable");
    expect(r?.day_type).toBeNull();
    expect(r?.inputs.adjustments).toEqual([]);
    expect(r?.state_reason).toContain("daypart 'morning' holds 7 samples");
    expect(r?.state_reason).toContain("≥8");
  });

  test("when only the dayclass grain is admitted, it supplies the adjustment", () => {
    // One reading a day at a rotating daypart over 16 days: every
    // daypart cell holds 4 dates, every weekday cell ≤3, but
    // dayclass-weekday holds 12 reading dates. Uniform Δ1 per
    // interval; intervals are 30h except the 19:00→01:00 ones
    // (6h). Weekday-ending intervals: 11 (Δ11) over 282 h;
    // overall: 15 over 378 h; ratio = (11/282) / (15/378)
    // = 4158/4230.
    const records: SnapshotRecord[] = [];
    let pct = 5;
    for (let d = 0; d < 16; d++) {
      if (d > 0) pct += 1;
      records.push({
        ts: new Date(chi(d, READING_HOURS[d % 4] as number)).toISOString(),
        serviceId: "_claude",
        metricId: "weekly",
        value: pct,
        unit: "pct",
        limit: 100,
        resetsAt: RESET_ISO,
        source: "api",
        observedBy: "test",
      });
    }
    const nowMs = chi(16, 8); // Wed Nov 25, 08:00 Chicago.
    const forecast = fforecast([fwindow({ projectedWallAt: nowMs + 200 * H })], nowMs);
    const [r] = buildDayTypeShadowRecords(forecast, records, new Map());
    expect(r?.state).toBe("shadow");
    expect(r?.day_type?.grain).toBe("dayclass");
    expect(r?.day_type?.cell).toBe("weekday");
    expect(r?.day_type?.samples).toBe(12);
    expect(r?.day_type?.ratio).toBeCloseTo(4158 / 4230, 12);
    expect(r?.day_type?.wallAt).toBe(new Date(nowMs + (50 / (0.25 * (4158 / 4230))) * H).toISOString());
  });

  test("no live v1 projection, no record — there is nothing to adjust", () => {
    const nowMs = chi(9, 8);
    const forecast = fforecast(
      [
        fwindow({ name: "weekly", state: "no-wall-before-reset", projectedWallAt: null }),
        fwindow({ name: "monthly", state: "insufficient-history", projectedWallAt: null, ratePtsPerHour: null }),
      ],
      nowMs,
    );
    expect(buildDayTypeShadowRecords(forecast, series(10, MORNING_DOUBLE), new Map())).toEqual([]);
  });

  test("the supersedes chain threads through the previous map", () => {
    const nowMs = chi(9, 8);
    const forecast = fforecast([fwindow({ projectedWallAt: nowMs + 200 * H })], nowMs);
    const previous = new Map([[dayTypeShadowSubjectKey("claude", "_claude", "weekly"), "c4.3:shadow:claude:weekly:1"]]);
    const [r] = buildDayTypeShadowRecords(forecast, series(10, MORNING_DOUBLE), previous);
    expect(r?.supersedes).toBe("c4.3:shadow:claude:weekly:1");
    expect(r?.id).toBe(`c4.3:shadow:claude:weekly:${nowMs}`);
  });
});

describe("spike day (oracle: design §5, S2.7 shape)", () => {
  // 16 days: days 0–8 burn Δ1 per interval (4/day), days 9–15
  // burn Δ0.5 (2/day). At day 15 20:00 Chicago the trailing 7
  // days are all slow: rate 14 pts / 168 h = 1/12. The trailing
  // 30 days carry every interval: 49 pts / 378 h. Light fraction
  // = 1 − (1/12) / (49/378) = 5/14 ≈ 0.357 ≥ 0.1 → spike.
  const SPIKE = (d: number) => (h: number) => (d <= 8 ? 1 : 0.5);
  const spikeSeries = () => series(16, (d, h) => SPIKE(d)(h));

  test("assessSpikeDay detects a ≥10%-light trailing week and reports both rates", () => {
    const a = assessSpikeDay(pointsOf(spikeSeries()), chi(15, 20));
    expect(a).not.toBeNull();
    expect(a?.detected).toBe(true);
    expect(a?.rateShortPtsPerHour).toBeCloseTo(1 / 12, 12);
    expect(a?.rateLongPtsPerHour).toBeCloseTo(49 / 378, 12);
    expect(a?.lightFraction).toBeCloseTo(5 / 14, 9);
    expect(a?.longDates).toBe(16);
  });

  test("a uniform burn is not light — no spike", () => {
    const a = assessSpikeDay(pointsOf(series(16, () => 1)), chi(15, 20));
    expect(a?.detected).toBe(false);
    expect(a?.lightFraction).toBeCloseTo(0, 9);
  });

  test("a thin long basis yields no verdict at all", () => {
    expect(assessSpikeDay(pointsOf(series(10, MORNING_DOUBLE)), chi(9, 20))).toBeNull();
  });

  test("the shadow record states the spike explicitly and widens by ×1.2 on the adjusted pace", () => {
    const nowMs = chi(15, 20); // Tue Nov 24, 20:00 Chicago — evening.
    const forecast = fforecast([fwindow({ projectedWallAt: nowMs + 200 * H })], nowMs);
    const [r] = buildDayTypeShadowRecords(forecast, spikeSeries(), new Map());
    expect(r?.inputs.adjustments).toEqual(["day_type:daypart:evening", "spike_day"]);
    expect(r?.spike_day?.paceMultiplier).toBe(1.2);
    expect(r?.spike_day?.lightFraction).toBeCloseTo(5 / 14, 9);
    const adjustedRate = (r?.day_type?.ratePtsPerHour as number) ?? 0.25;
    expect(r?.spike_day?.wallAt).toBe(new Date(nowMs + (50 / (adjustedRate * 1.2)) * H).toISOString());
  });
});

describe("renderDayTypeShadowReport", () => {
  test("the report names the methodology and both walls", () => {
    const nowMs = chi(9, 8);
    const forecast = fforecast([fwindow({ projectedWallAt: nowMs + 200 * H })], nowMs);
    const report = renderDayTypeShadowReport(buildDayTypeShadowRecords(forecast, series(10, MORNING_DOUBLE), new Map()));
    expect(report).toContain("c4-meth:5");
    expect(report).toContain("claude");
    expect(report).toContain("daypart:morning");
  });
});
