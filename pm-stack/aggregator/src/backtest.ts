// Forecast backtest + calibration report (#3986; design #1256 §5).
//
// Forecasts (ETA bands from #3982, contract objects from #3984) are
// published with no measurement of whether they are useful or
// calibrated. This module is the measurement:
//
// - A weekly SNAPSHOT STORE: every captured forecast is frozen as of
//   its cutoff with its model version, source watermarks, and the full
//   contract objects — the minimal inputs needed to reproduce an
//   evaluation later. Nothing is ever scored from live state.
// - ROLLING-ORIGIN EVALUATION over snapshots only: an evaluation as of
//   instant T sees no snapshot captured after T and no outcome record
//   recorded after T, so a backtest can never leak the future into a
//   forecast's score.
// - A CALIBRATION REPORT: sample counts, median absolute ETA error,
//   and P80 interval coverage by horizon (2/4/8 weeks) and confidence
//   grade, per model version (versions are never mixed into one
//   aggregate), compared against the documented no-change baseline
//   (the subject's previous snapshot p50 — the forecast you would
//   have had by simply not updating).
//
// Honesty gates (from the issue):
// - The model is NOT labelled calibrated before at least 30 resolved
//   forecasts across at least 8 distinct weekly cutoffs.
// - Nominal P80 coverage outside 70–90% in two successive eligible
//   windows is flagged, as is a model persistently worse than the
//   no-change baseline: when `recommendUnavailable` is set, consumers
//   should revert the model's displayed confidence to "unavailable"
//   (the disable/revert-to-unavailable rule) until a later eligible
//   window clears it.
// - Quota-wall classification accuracy is NOT evaluated here: the
//   #3983 outcome history does not exist yet, and #3986's scope
//   excludes it until it does. The report says so explicitly.

import { appendFile, mkdir, readFile } from "node:fs/promises";
import { dirname } from "node:path";
import type { EpicCompletionForecast, ForecastContract } from "./forecast-contract.ts";
import type { ForecastConfidence } from "./forecast.ts";
import type { RefKey } from "./model.ts";

export const FORECAST_SNAPSHOT_VERSION = "forecast-snapshot/v1" as const;
export const BACKTEST_REPORT_VERSION = "forecast-backtest/v1" as const;

/** Minimum resolved forecasts before any calibration claim. */
export const CALIBRATION_MIN_RESOLVED = 30;
/** Minimum distinct weekly cutoffs before any calibration claim. */
export const CALIBRATION_MIN_CUTOFFS = 8;
/** Nominal P80 coverage band; outside it in successive eligible
 * windows is a miscalibration flag. */
export const P80_COVERAGE_BAND = { low: 0.7, high: 0.9 } as const;

const DAY_MS = 86_400_000;
const WEEK_MS = 7 * DAY_MS;

/** One frozen weekly forecast capture: the unit of the snapshot
 * store. `forecasts` are the full epic-completion contract objects —
 * they already embed scope, input window/coverage, remaining units,
 * and reasons, which is what makes an evaluation reproducible from
 * the snapshot alone. One snapshot carries exactly one model version
 * (separation starts at the store, not at report time). */
export interface ForecastSnapshot {
  snapshotVersion: typeof FORECAST_SNAPSHOT_VERSION;
  modelVersion: string;
  /** The forecast origin: forecasts in this snapshot are as of this
   * instant (their own `asOf` may be earlier, never later). */
  cutoff: string;
  /** When the snapshot was written to the store. An evaluation as of
   * T never sees a snapshot captured after T. */
  capturedAt: string;
  /** Source name -> watermark (cursor / last-seen id / file digest)
   * naming exactly what source state the forecasts were derived
   * from. */
  sourceWatermarks: Record<string, string>;
  forecasts: EpicCompletionForecast[];
}

export interface CreateSnapshotInput {
  forecasts: ForecastContract[];
  capturedAt: string;
  sourceWatermarks: Record<string, string>;
  /** Defaults to the latest `asOf` among the epic-completion
   * forecasts. */
  cutoff?: string;
}

/** Build one snapshot from contract objects. Only epic-completion
 * forecasts are scored by this module; other contract types are
 * refused rather than silently dropped, so a caller mixing types
 * learns about it at capture time. */
export function createSnapshot(input: CreateSnapshotInput): ForecastSnapshot {
  const epics: EpicCompletionForecast[] = [];
  for (const f of input.forecasts) {
    if (f.type !== "epic_completion") {
      throw new Error(`forecast snapshot stores epic_completion forecasts only, got ${f.type}`);
    }
    epics.push(f);
  }
  if (epics.length === 0) throw new Error("forecast snapshot requires at least one forecast");
  const versions = new Set(epics.map((f) => f.modelVersion));
  if (versions.size !== 1) {
    throw new Error(`forecast snapshot mixes model versions: ${[...versions].join(", ")}`);
  }
  const cutoff = input.cutoff ?? epics.map((f) => f.asOf).sort().at(-1)!;
  for (const f of epics) {
    if (f.asOf > cutoff) throw new Error(`forecast for ${f.subjectRef} is as of ${f.asOf}, after snapshot cutoff ${cutoff}`);
  }
  return {
    snapshotVersion: FORECAST_SNAPSHOT_VERSION,
    modelVersion: epics[0].modelVersion,
    cutoff,
    capturedAt: input.capturedAt,
    sourceWatermarks: { ...input.sourceWatermarks },
    forecasts: epics,
  };
}

/** Append one snapshot to the JSONL store (creating directories). */
export async function appendSnapshot(path: string, snapshot: ForecastSnapshot): Promise<void> {
  await mkdir(dirname(path), { recursive: true });
  await appendFile(path, JSON.stringify(snapshot) + "\n", "utf8");
}

/** Load the whole snapshot store (JSONL). A missing store is an
 * empty store — a backtest over zero snapshots is a legitimate
 * not-yet-calibrated report, not an error. */
export async function loadSnapshots(path: string): Promise<ForecastSnapshot[]> {
  let text: string;
  try {
    text = await readFile(path, "utf8");
  } catch {
    return [];
  }
  const out: ForecastSnapshot[] = [];
  for (const line of text.split("\n")) {
    if (line.trim()) out.push(JSON.parse(line) as ForecastSnapshot);
  }
  return out;
}

/** One observed outcome for a forecast subject. `completedAt` is null
 * while the subject is still open; `recordedAt` is when this record
 * entered the outcome history — an evaluation as of T ignores
 * records recorded after T, even if they describe an earlier
 * completion (that is what cutoff isolation means). */
export interface ForecastOutcome {
  subjectRef: RefKey;
  completedAt: string | null;
  recordedAt: string;
}

export type HorizonBucket = "2w" | "4w" | "8w" | "over8w";

export interface BacktestGroup {
  horizon: HorizonBucket | "all";
  confidence: ForecastConfidence | "all";
  /** Resolved, scored forecasts in this group. */
  n: number;
  medianAbsEtaErrorDays: number | null;
  /** Share of resolved forecasts whose actual completion fell on or
   * before the forecast p80 date; null when no forecast in the group
   * carried a p80. */
  p80Coverage: number | null;
  p80N: number;
  /** Same median error for the no-change baseline (previous snapshot
   * p50 for the subject), over the forecasts that have one. */
  baselineMedianAbsEtaErrorDays: number | null;
  baselineN: number;
}

export interface CalibrationGate {
  state: "not_yet_calibrated" | "calibrated" | "flagged";
  resolved: number;
  distinctCutoffs: number;
  minResolved: number;
  minCutoffs: number;
  p80Coverage: number | null;
  /** P80 coverage outside the nominal band in two successive
   * eligible windows (gate already met at each window's end). */
  p80SuccessiveWindowsFlag: boolean;
  /** Model median error worse than the no-change baseline in each
   * of the two most recent scored windows. */
  persistentlyWorseThanBaseline: boolean;
  /** Disable/revert rule: when true, consumers revert this model's
   * displayed confidence to "unavailable" until a later eligible
   * window clears both flags. */
  recommendUnavailable: boolean;
}

export interface CalibrationReport {
  reportVersion: typeof BACKTEST_REPORT_VERSION;
  modelVersion: string;
  asOf: string;
  snapshotsUsed: number;
  cutoffs: string[];
  resolved: number;
  unresolved: number;
  /** Forecasts with no defensible date (p50 null): counted, never
   * scored. */
  unscoredNoDate: number;
  groups: BacktestGroup[];
  calibration: CalibrationGate;
  quotaWall: { state: "not_evaluated"; reason: string };
}

interface Scored {
  cutoff: string;
  confidence: ForecastConfidence;
  horizon: HorizonBucket;
  etaErrorDays: number;
  p80Covered: boolean | null;
  baselineErrorDays: number | null;
}

function median(values: number[]): number | null {
  if (values.length === 0) return null;
  const sorted = [...values].sort((a, b) => a - b);
  const mid = Math.floor(sorted.length / 2);
  return sorted.length % 2 === 1 ? sorted[mid] : (sorted[mid - 1] + sorted[mid]) / 2;
}

function horizonBucket(errSpanMs: number): HorizonBucket {
  if (errSpanMs <= 2 * WEEK_MS) return "2w";
  if (errSpanMs <= 4 * WEEK_MS) return "4w";
  if (errSpanMs <= 8 * WEEK_MS) return "8w";
  return "over8w";
}

function groupMetrics(rows: Scored[], horizon: BacktestGroup["horizon"], confidence: BacktestGroup["confidence"]): BacktestGroup {
  const p80Rows = rows.filter((r) => r.p80Covered !== null);
  const baseRows = rows.filter((r) => r.baselineErrorDays !== null);
  return {
    horizon,
    confidence,
    n: rows.length,
    medianAbsEtaErrorDays: median(rows.map((r) => r.etaErrorDays)),
    p80Coverage: p80Rows.length === 0 ? null : p80Rows.filter((r) => r.p80Covered).length / p80Rows.length,
    p80N: p80Rows.length,
    baselineMedianAbsEtaErrorDays: median(baseRows.map((r) => r.baselineErrorDays!)),
    baselineN: baseRows.length,
  };
}

/** Rolling-origin evaluation for ONE model version (callers get one
 * report per version present — see evaluateBacktest). */
export function evaluateVersion(input: {
  snapshots: ForecastSnapshot[];
  outcomes: ForecastOutcome[];
  asOf: string;
  modelVersion: string;
}): CalibrationReport {
  const { asOf, modelVersion } = input;
  const asOfMs = Date.parse(asOf);

  // Visibility: nothing created after the evaluation instant exists.
  const snapshots = input.snapshots
    .filter((s) => s.modelVersion === modelVersion && Date.parse(s.cutoff) <= asOfMs && Date.parse(s.capturedAt) <= asOfMs)
    .sort((a, b) => a.cutoff.localeCompare(b.cutoff));
  const outcomeBySubject = new Map<RefKey, ForecastOutcome>();
  for (const o of input.outcomes) {
    if (Date.parse(o.recordedAt) > asOfMs) continue;
    const prev = outcomeBySubject.get(o.subjectRef);
    if (!prev || o.recordedAt > prev.recordedAt) outcomeBySubject.set(o.subjectRef, o);
  }

  // Previous p50 per (subject, cutoff) for the no-change baseline.
  const previousP50 = new Map<string, string | null>();
  {
    const lastBySubject = new Map<RefKey, string | null>();
    for (const s of snapshots) {
      for (const f of s.forecasts) {
        previousP50.set(`${s.cutoff}\0${f.subjectRef}`, lastBySubject.get(f.subjectRef) ?? null);
        lastBySubject.set(f.subjectRef, f.p50);
      }
    }
  }

  const scored: Scored[] = [];
  let unresolved = 0;
  let unscoredNoDate = 0;
  for (const s of snapshots) {
    for (const f of s.forecasts) {
      if (f.p50 === null) {
        unscoredNoDate++;
        continue;
      }
      const outcome = outcomeBySubject.get(f.subjectRef);
      const completedMs = outcome?.completedAt ? Date.parse(outcome.completedAt) : null;
      if (completedMs === null || completedMs > asOfMs) {
        unresolved++;
        continue;
      }
      const base = previousP50.get(`${s.cutoff}\0${f.subjectRef}`);
      scored.push({
        cutoff: s.cutoff,
        confidence: f.confidence,
        horizon: horizonBucket(completedMs - Date.parse(s.cutoff)),
        etaErrorDays: Math.abs(completedMs - Date.parse(f.p50)) / DAY_MS,
        p80Covered: f.p80 === null ? null : completedMs <= Date.parse(f.p80),
        baselineErrorDays: base ? Math.abs(completedMs - Date.parse(base)) / DAY_MS : null,
      });
    }
  }

  const groups: BacktestGroup[] = [];
  const horizons: BacktestGroup["horizon"][] = ["2w", "4w", "8w", "over8w", "all"];
  const confidences: BacktestGroup["confidence"][] = ["high", "medium", "low", "unavailable", "all"];
  for (const h of horizons) {
    for (const c of confidences) {
      const rows = scored.filter((r) => (h === "all" || r.horizon === h) && (c === "all" || r.confidence === c));
      if (rows.length > 0) groups.push(groupMetrics(rows, h, c));
    }
  }

  // Windows: one per distinct cutoff, cumulative for gate/flag maths.
  const cutoffs = [...new Set(snapshots.map((s) => s.cutoff))].sort();
  let cumResolved = 0;
  let cumP80Covered = 0;
  let cumP80N = 0;
  const windowP80Flags: boolean[] = [];
  const windowWorse: boolean[] = [];
  for (const cutoff of cutoffs) {
    const rows = scored.filter((r) => r.cutoff === cutoff);
    cumResolved += rows.length;
    const p80Rows = rows.filter((r) => r.p80Covered !== null);
    cumP80Covered += p80Rows.filter((r) => r.p80Covered).length;
    cumP80N += p80Rows.length;
    const eligible = cumResolved >= CALIBRATION_MIN_RESOLVED && cutoffs.indexOf(cutoff) + 1 >= CALIBRATION_MIN_CUTOFFS;
    if (eligible && cumP80N > 0) {
      const cov = cumP80Covered / cumP80N;
      windowP80Flags.push(cov < P80_COVERAGE_BAND.low || cov > P80_COVERAGE_BAND.high);
    }
    const baseRows = rows.filter((r) => r.baselineErrorDays !== null);
    if (baseRows.length > 0) {
      const modelMed = median(rows.map((r) => r.etaErrorDays));
      const baseMed = median(baseRows.map((r) => r.baselineErrorDays!));
      if (modelMed !== null && baseMed !== null) windowWorse.push(modelMed > baseMed);
    }
  }
  const successive = (flags: boolean[]) => flags.length >= 2 && flags.at(-1)! && flags.at(-2)!;
  const p80Flag = successive(windowP80Flags);
  const worse = successive(windowWorse);

  const allGroup = groupMetrics(scored, "all", "all");
  const gateMet = scored.length >= CALIBRATION_MIN_RESOLVED && cutoffs.length >= CALIBRATION_MIN_CUTOFFS;
  return {
    reportVersion: BACKTEST_REPORT_VERSION,
    modelVersion,
    asOf,
    snapshotsUsed: snapshots.length,
    cutoffs,
    resolved: scored.length,
    unresolved,
    unscoredNoDate,
    groups,
    calibration: {
      state: !gateMet ? "not_yet_calibrated" : p80Flag || worse ? "flagged" : "calibrated",
      resolved: scored.length,
      distinctCutoffs: cutoffs.length,
      minResolved: CALIBRATION_MIN_RESOLVED,
      minCutoffs: CALIBRATION_MIN_CUTOFFS,
      p80Coverage: allGroup.p80Coverage,
      p80SuccessiveWindowsFlag: p80Flag,
      persistentlyWorseThanBaseline: worse,
      recommendUnavailable: gateMet && (p80Flag || worse),
    },
    quotaWall: {
      state: "not_evaluated",
      reason: "quota-wall classification accuracy requires the #3983 outcome history, which does not exist yet; excluded from #3986 evaluation until it does",
    },
  };
}

/** Evaluate every model version present in the visible snapshots,
 * one report per version — aggregates never mix versions. */
export function evaluateBacktest(input: {
  snapshots: ForecastSnapshot[];
  outcomes: ForecastOutcome[];
  asOf: string;
}): CalibrationReport[] {
  const versions = [...new Set(input.snapshots.map((s) => s.modelVersion))].sort();
  return versions.map((modelVersion) => evaluateVersion({ ...input, modelVersion }));
}
