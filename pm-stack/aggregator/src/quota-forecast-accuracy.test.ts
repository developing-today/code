// Tests for the Stage-3 S2 forecast accuracy ledger + backtest
// (#12696). Every test names its oracle: the design's rename list
// (§2), projectWindow's own semantics (the model under score is the
// oracle — the harness fits nothing), or the textbook definitions
// of precision/recall and the error quantiles.
import { describe, expect, test } from "bun:test";
import {
  FORECAST_POOLS,
  SERVICE_ID_RENAME_ALIASES,
  buildForecastAccuracyLedger,
  errorDistribution,
  forecastAccuracySeriesKey,
  indexPercentRecords,
  pointsForWindow,
  segmentPeriods,
  type ForecastAccuracyLedger,
} from "./quota-forecast-accuracy.ts";
import type { SnapshotRecord } from "./model.ts";

const H = 3_600_000;
const T0 = Date.parse("2026-11-20T00:00:00Z");
const iso = (ms: number) => new Date(ms).toISOString();

function rec(
  serviceId: string,
  metricId: string,
  tsMs: number,
  pct: number,
  resetsAt: string | null,
): SnapshotRecord {
  return {
    ts: iso(tsMs),
    serviceId,
    metricId,
    value: pct,
    unit: "pct",
    limit: 100,
    resetsAt,
    source: "api",
    observedBy: "test",
  };
}

function pool(ledger: ForecastAccuracyLedger, serviceId: string) {
  const p = ledger.pools.find((x) => x.pool === serviceId);
  if (!p) throw new Error(`pool ${serviceId} missing from ledger`);
  return p;
}

describe("rename join (oracle: design §2 rename list + quota-current pairs)", () => {
  test("the fold removes spelling, never structure — the S1 fold table", () => {
    expect(forecastAccuracySeriesKey("_antigravity")).toBe("antigravity");
    expect(forecastAccuracySeriesKey("command-code")).toBe("commandcode");
    expect(forecastAccuracySeriesKey("_commandcode")).toBe("commandcode");
    expect(forecastAccuracySeriesKey("meta:high-usage")).toBe("meta.highusage");
    expect(forecastAccuracySeriesKey("_openai.main")).toBe("openai.main");
    // Distinct pools stay distinct.
    expect(forecastAccuracySeriesKey("_openai.alt")).not.toBe(forecastAccuracySeriesKey("_openai.main"));
  });

  test("renamed spellings land in their pool's series", () => {
    const records: SnapshotRecord[] = [
      rec("command-code", "5h", T0, 10, iso(T0 + 5 * H)),
      rec("commandcode", "5h", T0 + H, 20, iso(T0 + 5 * H)),
      rec("_commandcode", "5h", T0 + 2 * H, 30, iso(T0 + 5 * H)),
      rec("openai.alternate", "5h", T0, 40, iso(T0 + 5 * H)),
      rec("meta:high-usage", "weekly", T0, 50, iso(T0 + 7 * 24 * H)),
      rec("antigravity:gemini", "5h", T0, 60, iso(T0 + 5 * H)),
      rec("opencode.drewrypope", "weekly", T0, 70, iso(T0 + 7 * 24 * H)),
    ];
    const { index, joinedIds } = indexPercentRecords(records);
    expect(pointsForWindow(index, "_commandcode", "5h")).toHaveLength(3);
    expect(pointsForWindow(index, "_openai.alt", "5h")).toHaveLength(1);
    expect(pointsForWindow(index, "meta", "weekly")).toHaveLength(1);
    expect(pointsForWindow(index, "_antigravity.gemini", "5h")).toHaveLength(1);
    expect(pointsForWindow(index, "opencode-go.drewrypope", "weekly")).toHaveLength(1);
    // Provenance: the joined ids are named, per pool.
    expect(joinedIds.get("commandcode")).toEqual(["_commandcode", "command-code", "commandcode"]);
    expect(SERVICE_ID_RENAME_ALIASES["openai.alternate"]).toBe("_openai.alt");
  });

  test("every register pool is a known aggregator pool shape", () => {
    // Oracle: FORECAST_POOLS mirrors POOL_SPECS + bare opencode-go.
    expect(FORECAST_POOLS.map((p) => p.serviceId)).toContain("_antigravity.claude-gpt");
    expect(FORECAST_POOLS.map((p) => p.serviceId)).toContain("opencode-go");
    expect(FORECAST_POOLS).toHaveLength(17);
  });
});

describe("period segmentation (oracle: PROJECTION_PERIOD_TOLERANCE_MS semantics)", () => {
  test("jittered resets stay one period; a real reset starts a new one; null joins the open period", () => {
    const pts = [
      { ts: T0, pct: 10, resetsAt: iso(T0 + 5 * H) },
      { ts: T0 + H, pct: 20, resetsAt: iso(T0 + 5 * H + 30_000) }, // 30s jitter
      { ts: T0 + 2 * H, pct: 30, resetsAt: null },
      { ts: T0 + 6 * H, pct: 5, resetsAt: iso(T0 + 10 * H) },
    ];
    const periods = segmentPeriods(pts);
    expect(periods).toHaveLength(2);
    expect(periods[0]?.points).toHaveLength(3);
    expect(periods[1]?.points).toHaveLength(1);
  });
});

describe("scoring (oracle: projectWindow on a hand-computed linear series)", () => {
  // Linear burn 10..100 by 10 every 30 min: least squares is exact,
  // so every measurable forecast projects the wall at the realized
  // instant (4:30) with error 0.
  function linearWallRecords(): SnapshotRecord[] {
    const reset = iso(T0 + 5 * H);
    const out: SnapshotRecord[] = [];
    for (let i = 0; i < 10; i++) out.push(rec("cline-pass", "5h", T0 + i * (H / 2), 10 * (i + 1), reset));
    return out;
  }

  test("a linear wall is called exactly: TPs with zero error, precision/recall 1", () => {
    const { ledger, facts } = buildForecastAccuracyLedger(linearWallRecords(), { asOfMs: T0 + 24 * H });
    const w = pool(ledger, "cline-pass").windows.find((x) => x.window === "5h");
    if (!w) throw new Error("5h baseline missing");
    expect(w.scoreable).toBe(true);
    expect(w.periodsScored).toBe(1);
    expect(w.periodsWalled).toBe(1);
    expect(w.detectionsWhileWalled).toBe(1); // the 100% reading itself
    expect(w.forecastsInsufficient).toBe(1); // the first reading: <2 points
    expect(w.tp).toBe(8);
    expect(w.fp).toBe(0);
    expect(w.fn).toBe(0);
    expect(w.precision).toBe(1);
    expect(w.recall).toBe(1);
    expect(w.wallErrorMin?.count).toBe(8);
    expect(w.wallErrorMin?.medianSignedMin).toBe(0);
    expect(w.firstCallLeadMin).toEqual({ count: 1, medianMin: 240 });
    expect(facts.every((f) => f.errorMin === 0)).toBe(true);
  });

  test("a flat series scores TN only: precision/recall undefined, never zero (oracle: the definitions)", () => {
    const reset = iso(T0 + 5 * H);
    const records: SnapshotRecord[] = [];
    for (let i = 0; i < 10; i++) records.push(rec("cline-pass", "5h", T0 + i * (H / 2), 40, reset));
    const { ledger } = buildForecastAccuracyLedger(records, { asOfMs: T0 + 24 * H });
    const w = pool(ledger, "cline-pass").windows.find((x) => x.window === "5h");
    if (!w) throw new Error("5h baseline missing");
    expect(w.scoreable).toBe(true);
    expect(w.tn).toBeGreaterThan(0);
    expect(w.tp + w.fp + w.fn).toBe(0);
    expect(w.precision).toBeNull();
    expect(w.recall).toBeNull();
    expect(w.unscoreableReasons.join(" ")).toContain("precision undefined");
    expect(w.unscoreableReasons.join(" ")).toContain("recall undefined");
  });

  test("a wall with no warning scores FN: recall 0 (oracle: the definition)", () => {
    const reset = iso(T0 + 5 * H);
    const records: SnapshotRecord[] = [];
    for (let i = 0; i < 7; i++) records.push(rec("cline-pass", "5h", T0 + i * (H / 2), 40, reset));
    records.push(rec("cline-pass", "5h", T0 + 3.5 * H, 100, reset));
    const { ledger } = buildForecastAccuracyLedger(records, { asOfMs: T0 + 24 * H });
    const w = pool(ledger, "cline-pass").windows.find((x) => x.window === "5h");
    if (!w) throw new Error("5h baseline missing");
    expect(w.periodsWalled).toBe(1);
    expect(w.fn).toBeGreaterThan(0);
    expect(w.tp).toBe(0);
    expect(w.recall).toBe(0);
    expect(w.precision).toBeNull();
  });

  test("a burn that stops short scores FP: precision 0 with a realized no-wall (oracle: the definition)", () => {
    const reset = iso(T0 + 5 * H);
    const records: SnapshotRecord[] = [
      rec("cline-pass", "5h", T0, 10, reset),
      rec("cline-pass", "5h", T0 + H / 2, 30, reset),
      rec("cline-pass", "5h", T0 + H, 50, reset),
      rec("cline-pass", "5h", T0 + 2 * H, 50, reset),
      rec("cline-pass", "5h", T0 + 3 * H, 50, reset),
      rec("cline-pass", "5h", T0 + 4 * H, 50, reset),
    ];
    const { ledger } = buildForecastAccuracyLedger(records, { asOfMs: T0 + 24 * H });
    const w = pool(ledger, "cline-pass").windows.find((x) => x.window === "5h");
    if (!w) throw new Error("5h baseline missing");
    expect(w.periodsWalled).toBe(0);
    expect(w.fp).toBeGreaterThan(0);
    expect(w.tp).toBe(0);
    expect(w.precision).toBe(0);
    expect(w.recall).toBeNull();
  });
});

describe("unscoreable honesty (oracle: an unmeasured window is never an accurate one)", () => {
  test("no resetsAt anywhere: unscoreable with the reason named", () => {
    const records: SnapshotRecord[] = [];
    for (let i = 0; i < 6; i++) records.push(rec("_commandcode", "monthly", T0 + i * 24 * H, 10 * i, null));
    const { ledger } = buildForecastAccuracyLedger(records, { asOfMs: T0 + 30 * 24 * H });
    const w = pool(ledger, "_commandcode").windows.find((x) => x.window === "monthly");
    if (!w) throw new Error("monthly baseline missing");
    expect(w.scoreable).toBe(false);
    expect(w.periodsNoReset).toBe(1);
    expect(w.precision).toBeNull();
    expect(w.recall).toBeNull();
    expect(w.unscoreableReasons.join(" ")).toContain("resetsAt");
  });

  test("a still-open period is excluded, not scored against an unknown outcome", () => {
    const reset = iso(T0 + 5 * H);
    const records: SnapshotRecord[] = [
      rec("cline-pass", "5h", T0, 10, reset),
      rec("cline-pass", "5h", T0 + H, 30, reset),
    ];
    const { ledger } = buildForecastAccuracyLedger(records, { asOfMs: T0 + 2 * H });
    const w = pool(ledger, "cline-pass").windows.find((x) => x.window === "5h");
    if (!w) throw new Error("5h baseline missing");
    expect(w.periodsOpen).toBe(1);
    expect(w.periodsScored).toBe(0);
    expect(w.scoreable).toBe(false);
    expect(w.forecastsScored).toBe(0);
  });

  test("a period that already walled is scored even with its reset still ahead (oracle: the outcome is realized)", () => {
    // The linear-wall fixture, but asOf is the wall reading itself —
    // before the reset. The wall is a realized outcome, so the
    // period scores exactly as in the completed case.
    const reset = iso(T0 + 5 * H);
    const records: SnapshotRecord[] = [];
    for (let i = 0; i < 10; i++) records.push(rec("cline-pass", "5h", T0 + i * (H / 2), 10 * (i + 1), reset));
    const { ledger } = buildForecastAccuracyLedger(records, { asOfMs: T0 + 4.5 * H });
    const w = pool(ledger, "cline-pass").windows.find((x) => x.window === "5h");
    if (!w) throw new Error("5h baseline missing");
    expect(w.periodsOpen).toBe(0);
    expect(w.periodsScored).toBe(1);
    expect(w.periodsWalled).toBe(1);
    expect(w.tp).toBe(8);
    expect(w.recall).toBe(1);
  });

  test("a pool with no percent readings is unscoreable at the rollup, not absent", () => {
    const { ledger } = buildForecastAccuracyLedger([], { asOfMs: T0 });
    const p = pool(ledger, "deepseek");
    expect(p.windows).toHaveLength(0);
    expect(p.rollup.scoreable).toBe(false);
    expect(p.rollup.unscoreableReasons.join(" ")).toContain("no percent windows");
  });
});

describe("error distribution (oracle: the quantile definitions)", () => {
  test("known array, known answer", () => {
    const d = errorDistribution([-30, 0, 30, 60]);
    expect(d?.count).toBe(4);
    expect(d?.meanSignedMin).toBe(15);
    expect(d?.medianSignedMin).toBe(15);
    expect(d?.medianAbsMin).toBe(30);
    expect(d?.p90AbsMin).toBe(51);
  });
  test("empty is null, not zero", () => {
    expect(errorDistribution([])).toBeNull();
  });
});
