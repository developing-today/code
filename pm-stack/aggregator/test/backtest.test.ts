import { describe, expect, it } from "bun:test";
import { mkdtemp, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import {
  appendSnapshot,
  createSnapshot,
  evaluateBacktest,
  evaluateVersion,
  loadSnapshots,
  type ForecastOutcome,
  type ForecastSnapshot,
} from "../src/backtest.ts";
import type { EpicCompletionForecast } from "../src/forecast-contract.ts";
import type { ForecastConfidence } from "../src/forecast.ts";

const CUTOFF = "2026-01-05T00:00:00.000Z";

function forecast(
  subjectRef: string,
  asOf: string,
  p50: string | null,
  p80: string | null,
  confidence: ForecastConfidence = "high",
  modelVersion = "forecast/v3",
): EpicCompletionForecast {
  return {
    contractVersion: "forecast-contract/v1",
    type: "epic_completion",
    subjectRef,
    scope: "test scope",
    asOf,
    modelVersion,
    confidence,
    reasons: [],
    limitations: [],
    status: "forecast",
    inputWindow: { primaryWeeks: 4, comparisonWeeks: 4, completeWeeks: 4, from: null, to: asOf },
    inputCoverage: { completeWeeks: 4, totalCutoffs: 4, unavailableCutoffs: 0 },
    remainingUnits: 3,
    completedUnits: 7,
    p50,
    p80,
    baselineSource: "subject",
    remainingEffort: { byEffort: {}, unsized: 0, openEnded: 0 },
  };
}

function snapshot(forecasts: EpicCompletionForecast[], cutoff = CUTOFF, capturedAt = cutoff): ForecastSnapshot {
  return createSnapshot({ forecasts, capturedAt, sourceWatermarks: { "item-history": "fixture" }, cutoff });
}

function outcome(
  subjectRef: string,
  completedAt: string | null,
  recordedAt = "2026-03-01T00:00:00.000Z",
): ForecastOutcome {
  return { subjectRef, completedAt, recordedAt };
}

describe("forecast snapshot store (#3986)", () => {
  it("round-trips snapshots as JSONL with the issue's fields", async () => {
    const dir = await mkdtemp(join(tmpdir(), "backtest-store-"));
    try {
      const path = join(dir, "snapshots.jsonl");
      const snap = snapshot([forecast("gh:o/r#1", CUTOFF, "2026-01-12T00:00:00.000Z", "2026-01-19T00:00:00.000Z")]);
      expect(snap.modelVersion).toBe("forecast/v3");
      expect(snap.cutoff).toBe(CUTOFF);
      expect(snap.sourceWatermarks["item-history"]).toBe("fixture");
      await appendSnapshot(path, snap);
      await appendSnapshot(path, snap);
      const loaded = await loadSnapshots(path);
      expect(loaded).toHaveLength(2);
      expect(loaded[0]).toEqual(snap);
      expect(await loadSnapshots(join(dir, "missing.jsonl"))).toEqual([]);
    } finally {
      await rm(dir, { recursive: true, force: true });
    }
  });

  it("refuses mixed model versions and non-epic forecasts at capture", () => {
    expect(() =>
      createSnapshot({
        forecasts: [
          forecast("gh:o/r#1", CUTOFF, null, null, "high", "forecast/v3"),
          forecast("gh:o/r#2", CUTOFF, null, null, "high", "forecast/v4"),
        ],
        capturedAt: CUTOFF,
        sourceWatermarks: {},
      }),
    ).toThrow("mixes model versions");
  });
});

describe("rolling-origin evaluation (#3986)", () => {
  const asOf = "2026-03-01T00:00:00.000Z";

  it("computes median ETA error and P80 coverage by horizon and confidence", () => {
    const snap = snapshot([
      forecast("gh:o/r#a", CUTOFF, "2026-01-12T00:00:00.000Z", "2026-01-19T00:00:00.000Z"),
      forecast("gh:o/r#b", CUTOFF, "2026-01-12T00:00:00.000Z", "2026-01-13T00:00:00.000Z", "low"),
      forecast("gh:o/r#c", CUTOFF, null, null),
      forecast("gh:o/r#d", CUTOFF, "2026-01-12T00:00:00.000Z", "2026-01-19T00:00:00.000Z"),
    ]);
    const report = evaluateVersion({
      snapshots: [snap],
      outcomes: [
        outcome("gh:o/r#a", "2026-01-14T00:00:00.000Z"), // err 2d, 2w horizon, covered
        outcome("gh:o/r#b", "2026-01-20T00:00:00.000Z"), // err 8d, 4w horizon (15d), not covered
        outcome("gh:o/r#c", "2026-01-14T00:00:00.000Z"),
        outcome("gh:o/r#d", null), // unresolved
      ],
      asOf,
      modelVersion: "forecast/v3",
    });
    expect(report.resolved).toBe(2);
    expect(report.unresolved).toBe(1);
    expect(report.unscoredNoDate).toBe(1);
    const all = report.groups.find((g) => g.horizon === "all" && g.confidence === "all")!;
    expect(all.n).toBe(2);
    expect(all.medianAbsEtaErrorDays).toBe(5);
    expect(all.p80Coverage).toBe(0.5);
    const twoW = report.groups.find((g) => g.horizon === "2w" && g.confidence === "high")!;
    expect(twoW.medianAbsEtaErrorDays).toBe(2);
    const fourW = report.groups.find((g) => g.horizon === "4w" && g.confidence === "low")!;
    expect(fourW.p80Coverage).toBe(0);
    expect(report.calibration.state).toBe("not_yet_calibrated");
    expect(report.quotaWall.state).toBe("not_evaluated");
  });

  it("cutoff isolation: nothing created after the evaluation instant is visible", () => {
    const visible = snapshot([forecast("gh:o/r#a", CUTOFF, "2026-01-12T00:00:00.000Z", "2026-01-19T00:00:00.000Z")]);
    // Same cutoff, but captured after asOf: a record created after T.
    const futureCapture = { ...visible, capturedAt: "2026-03-02T00:00:00.000Z" };
    const report = evaluateVersion({
      snapshots: [futureCapture],
      outcomes: [outcome("gh:o/r#a", "2026-01-14T00:00:00.000Z")],
      asOf,
      modelVersion: "forecast/v3",
    });
    expect(report.snapshotsUsed).toBe(0);
    expect(report.resolved).toBe(0);
    // An outcome recorded after asOf does not resolve the forecast,
    // even though the completion itself predates asOf.
    const lateRecord = outcome("gh:o/r#a", "2026-01-14T00:00:00.000Z", "2026-03-02T00:00:00.000Z");
    const report2 = evaluateVersion({
      snapshots: [visible],
      outcomes: [lateRecord],
      asOf,
      modelVersion: "forecast/v3",
    });
    expect(report2.resolved).toBe(0);
    expect(report2.unresolved).toBe(1);
  });

  it("separates model versions into separate reports", () => {
    const v3 = snapshot([forecast("gh:o/r#a", CUTOFF, "2026-01-12T00:00:00.000Z", null, "high", "forecast/v3")]);
    const v4 = snapshot([forecast("gh:o/r#a", CUTOFF, "2026-01-19T00:00:00.000Z", null, "high", "forecast/v4")]);
    const reports = evaluateBacktest({
      snapshots: [v3, v4],
      outcomes: [outcome("gh:o/r#a", "2026-01-13T00:00:00.000Z")],
      asOf,
    });
    expect(reports.map((r) => r.modelVersion)).toEqual(["forecast/v3", "forecast/v4"]);
    expect(reports[0].groups.find((g) => g.horizon === "all" && g.confidence === "all")!.medianAbsEtaErrorDays).toBe(1);
    expect(reports[1].groups.find((g) => g.horizon === "all" && g.confidence === "all")!.medianAbsEtaErrorDays).toBe(6);
  });

  it("scores the no-change baseline from the subject's previous snapshot", () => {
    const first = snapshot(
      [forecast("gh:o/r#a", "2026-01-05T00:00:00.000Z", "2026-02-01T00:00:00.000Z", null)],
      "2026-01-05T00:00:00.000Z",
    );
    const second = snapshot(
      [forecast("gh:o/r#a", "2026-01-12T00:00:00.000Z", "2026-01-20T00:00:00.000Z", null)],
      "2026-01-12T00:00:00.000Z",
    );
    const report = evaluateVersion({
      snapshots: [first, second],
      outcomes: [outcome("gh:o/r#a", "2026-01-22T00:00:00.000Z")],
      asOf,
      modelVersion: "forecast/v3",
    });
    const all = report.groups.find((g) => g.horizon === "all" && g.confidence === "all")!;
    expect(all.n).toBe(2);
    expect(all.baselineN).toBe(1); // only the second forecast has a predecessor
    expect(all.baselineMedianAbsEtaErrorDays).toBe(10); // |Jan 22 - Feb 1|
  });
});

describe("calibration gate (#3986)", () => {
  const asOf = "2026-06-01T00:00:00.000Z";
  const DAY = 86_400_000;

  function gateFixture(cutoffs: number, perCutoff: number, coveredShare: number) {
    const snapshots: ForecastSnapshot[] = [];
    const outcomes: ForecastOutcome[] = [];
    const base = Date.parse("2026-01-05T00:00:00.000Z");
    let idx = 0;
    for (let c = 0; c < cutoffs; c++) {
      const cutoffMs = base + c * 7 * DAY;
      const cutoff = new Date(cutoffMs).toISOString();
      const forecasts: EpicCompletionForecast[] = [];
      for (let k = 0; k < perCutoff; k++, idx++) {
        const subject = `gh:o/r#s${idx}`;
        const p50 = new Date(cutoffMs + 10 * DAY).toISOString();
        const p80 = new Date(cutoffMs + 20 * DAY).toISOString();
        forecasts.push(forecast(subject, cutoff, p50, p80));
        // Deterministic coverage pattern: share 0.8 covers idx % 5 != 4
        // (actual lands 12d after cutoff, inside the p80 at +20d); an
        // uncovered forecast lands at +25d, past its p80.
        const isCovered = coveredShare >= 1 ? true : coveredShare <= 0 ? false : idx % 5 !== 4;
        const actual = new Date(cutoffMs + (isCovered ? 12 : 25) * DAY).toISOString();
        outcomes.push(outcome(subject, actual, "2026-05-01T00:00:00.000Z"));
      }
      snapshots.push(createSnapshot({ forecasts, capturedAt: cutoff, sourceWatermarks: {}, cutoff }));
    }
    return { snapshots, outcomes };
  }

  it("labels the model calibrated only at 30 resolved across 8 cutoffs with in-band coverage", () => {
    const { snapshots, outcomes } = gateFixture(10, 4, 0.8); // 40 resolved, 80% coverage
    const report = evaluateVersion({ snapshots, outcomes, asOf, modelVersion: "forecast/v3" });
    expect(report.resolved).toBe(40);
    expect(report.calibration.distinctCutoffs).toBe(10);
    expect(report.calibration.p80Coverage).toBeCloseTo(0.8, 5);
    expect(report.calibration.state).toBe("calibrated");
    expect(report.calibration.recommendUnavailable).toBe(false);
  });

  it("flags P80 coverage outside 70-90% in successive eligible windows", () => {
    const { snapshots, outcomes } = gateFixture(10, 4, 0); // nothing covered
    const report = evaluateVersion({ snapshots, outcomes, asOf, modelVersion: "forecast/v3" });
    expect(report.calibration.p80Coverage).toBe(0);
    expect(report.calibration.p80SuccessiveWindowsFlag).toBe(true);
    expect(report.calibration.state).toBe("flagged");
    expect(report.calibration.recommendUnavailable).toBe(true);
  });

  it("keeps the not-yet-calibrated gate below the thresholds", () => {
    const { snapshots, outcomes } = gateFixture(3, 4, 0.8); // 12 resolved, 3 cutoffs
    const report = evaluateVersion({ snapshots, outcomes, asOf, modelVersion: "forecast/v3" });
    expect(report.calibration.state).toBe("not_yet_calibrated");
    expect(report.calibration.recommendUnavailable).toBe(false);
  });
});

describe("Codex P2 regressions (PR #8436)", () => {
  const asOf = "2026-06-01T00:00:00.000Z";
  const DAY = 86_400_000;

  it("excludes subjects already complete at the forecast origin", () => {
    const snap = snapshot([
      // As deriveEpicEta emits for a completed subject: p50 = p80 = asOf.
      forecast("gh:o/r#done", CUTOFF, CUTOFF, CUTOFF),
      forecast("gh:o/r#live", CUTOFF, "2026-01-12T00:00:00.000Z", "2026-01-19T00:00:00.000Z"),
    ]);
    const report = evaluateVersion({
      snapshots: [snap],
      outcomes: [
        outcome("gh:o/r#done", "2026-01-03T00:00:00.000Z"), // completed BEFORE the Jan 5 cutoff
        outcome("gh:o/r#live", "2026-01-14T00:00:00.000Z"),
      ],
      asOf,
      modelVersion: "forecast/v3",
    });
    expect(report.resolved).toBe(1);
    expect(report.excludedAlreadyComplete).toBe(1);
    const all = report.groups.find((g) => g.horizon === "all" && g.confidence === "all")!;
    expect(all.n).toBe(1);
    expect(all.medianAbsEtaErrorDays).toBe(2);
  });

  it("calibration gate counts only cutoffs contributing scored forecasts", () => {
    const snapshots: ForecastSnapshot[] = [];
    const outcomes: ForecastOutcome[] = [];
    const base = Date.parse(CUTOFF);
    // Seven weekly snapshots whose forecasts are all unavailable.
    for (let c = 0; c < 7; c++) {
      const cutoff = new Date(base + c * 7 * DAY).toISOString();
      snapshots.push(snapshot([forecast(`gh:o/r#u${c}`, cutoff, null, null)], cutoff));
    }
    // The eighth carries 30 forecasts that resolve at 80% P80 coverage.
    const c8ms = base + 7 * 7 * DAY;
    const c8 = new Date(c8ms).toISOString();
    const fs: EpicCompletionForecast[] = [];
    for (let k = 0; k < 30; k++) {
      const subject = `gh:o/r#g${k}`;
      fs.push(forecast(subject, c8, new Date(c8ms + 10 * DAY).toISOString(), new Date(c8ms + 20 * DAY).toISOString()));
      outcomes.push(
        outcome(subject, new Date(c8ms + (k % 5 !== 4 ? 12 : 25) * DAY).toISOString(), "2026-05-01T00:00:00.000Z"),
      );
    }
    snapshots.push(snapshot(fs, c8));
    const report = evaluateVersion({ snapshots, outcomes, asOf, modelVersion: "forecast/v3" });
    expect(report.resolved).toBe(30);
    expect(report.calibration.distinctCutoffs).toBe(1);
    expect(report.calibration.state).toBe("not_yet_calibrated");
    expect(report.calibration.recommendUnavailable).toBe(false);
  });

  it("compares model and baseline errors over the same subjects", () => {
    const c1 = "2026-01-05T00:00:00.000Z";
    const c2 = "2026-01-12T00:00:00.000Z";
    const c3 = "2026-01-19T00:00:00.000Z";
    const actual = "2026-03-01T00:00:00.000Z";
    const s1 = snapshot(
      [
        forecast("gh:o/r#a", c1, "2026-01-20T00:00:00.000Z", null),
        forecast("gh:o/r#b", c1, "2026-01-20T00:00:00.000Z", null),
      ],
      c1,
    );
    // Continuing subjects improve (model err 5 vs baseline err 40);
    // first-seen subjects carry 100d errors and no baseline.
    const s2 = snapshot(
      [
        forecast("gh:o/r#a", c2, "2026-02-24T00:00:00.000Z", null),
        forecast("gh:o/r#b", c2, "2026-02-24T00:00:00.000Z", null),
        forecast("gh:o/r#n1", c2, "2026-06-09T00:00:00.000Z", null),
        forecast("gh:o/r#n2", c2, "2026-06-09T00:00:00.000Z", null),
      ],
      c2,
    );
    // Continuing subjects improve again (model err 2 vs baseline err 5).
    const s3 = snapshot(
      [
        forecast("gh:o/r#a", c3, "2026-02-27T00:00:00.000Z", null),
        forecast("gh:o/r#b", c3, "2026-02-27T00:00:00.000Z", null),
        forecast("gh:o/r#n3", c3, "2026-06-09T00:00:00.000Z", null),
        forecast("gh:o/r#n4", c3, "2026-06-09T00:00:00.000Z", null),
      ],
      c3,
    );
    const outcomes = ["a", "b", "n1", "n2", "n3", "n4"].map((x) => outcome(`gh:o/r#${x}`, actual));
    const report = evaluateVersion({ snapshots: [s1, s2, s3], outcomes, asOf, modelVersion: "forecast/v3" });
    // Unpaired medians would call both windows worse (medians 52.5 > 40
    // and 51 > 5); paired, the model beats the baseline in both.
    expect(report.calibration.persistentlyWorseThanBaseline).toBe(false);
  });

  it("forecast-snapshot CLI captures the /api/forecasts.json contracts shape", async () => {
    const dir = await mkdtemp(join(tmpdir(), "backtest-cli-"));
    try {
      const api = {
        contractVersion: "forecast-contract/v1",
        generatedAt: CUTOFF,
        contracts: [
          forecast("gh:o/r#1", CUTOFF, "2026-01-12T00:00:00.000Z", "2026-01-19T00:00:00.000Z"),
          { type: "quota_exhaustion" },
          { type: "stall_risk" },
        ],
      };
      const inputPath = join(dir, "forecasts.json");
      await writeFile(inputPath, JSON.stringify(api));
      const outPath = join(dir, "snapshots.jsonl");
      const script = join(import.meta.dir, "../scripts/forecast-snapshot.ts");
      const proc = Bun.spawn(
        [
          process.execPath,
          "run",
          script,
          "--forecasts",
          inputPath,
          "--out",
          outPath,
          "--watermark",
          "item-history=fixture",
        ],
        { stdout: "pipe", stderr: "pipe" },
      );
      expect(await proc.exited).toBe(0);
      const loaded = await loadSnapshots(outPath);
      expect(loaded).toHaveLength(1);
      expect(loaded[0].forecasts).toHaveLength(1);
      expect(loaded[0].forecasts[0].subjectRef).toBe("gh:o/r#1");
      expect(loaded[0].sourceWatermarks["item-history"]).toBe("fixture");
    } finally {
      await rm(dir, { recursive: true, force: true });
    }
  });
});
