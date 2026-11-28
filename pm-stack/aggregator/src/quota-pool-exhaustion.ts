// C4.5 — pool_exhaustion predictions (#13261, component of the C4
// quota-forecasting epic #9572). The record format, storage, and
// scoring contract are the C4 design:
// doc/2026-11-13T15-12-00Z_design_reporting-architecture-event-driven/
// c4-forecasting-design.md (§1 the prediction record, §4 range
// rendering, §2 the pool_exhaustion kind).
//
// Derivation (§2): the record wraps S1's product; it reimplements
// nothing (Stage-3 D2: the model is `projectWindow`, unchanged).
// Per tick, each pool/window whose S1 row is a live projection
// also gets a C4 record:
//
// - `basis`: `since_last_reset` (the same-period series S1
//   assembles is the current reset period's readings).
// - `prediction`: the projected wall instant becomes `likely`;
//   the range is `likely` widened by the S2 ledger's measured
//   signed error for that pool/window at that lead time. Until a
//   window cell has ≥20 scored forecasts, the range is the pool's
//   all-window error band; below that, `insufficient_history`
//   (never an invented range, never a silent absence).
// - Walled windows record the observed wall (`likely` = issuance):
//   the wall is measured, not estimated, and the alert the record
//   feeds must still fire for a workhorse pool that is already
//   walled. Scoring (§3) treats detections as detections — the S2
//   rules join outcomes to records in one place — so emitting the
//   record changes no score, it only keeps the alert honest.
//
// The calibration (the S2 ledger's per-cell signed-error
// quantiles) arrives as an argument: this module fits nothing
// itself. The gatherer wiring reads it from
// quota-exhaustion-calibration.json beside the forecast product;
// a missing or torn file is an empty map, so every subject
// honestly publishes insufficient_history until the S2
// live-scoring feed populates it (the follow-on slice). The
// degenerate-band floor (±1 minute) keeps earliest < latest per
// §1 without inventing precision the ledger does not have.
//
// Advisory only: no placement path reads these records — the C6
// pages and forecast alerts named by #9572 consume them, and the
// alerts module already rules that pool_exhaustion on a
// non-workhorse pool stays silent
// (quota-forecast-alerts.ts).
//
// Pure functions over plain values — no I/O, no clock reads
// except through arguments. The gatherer-tick wiring (records
// logged per live-projection pool on ticks, appended to
// quota-predictions.jsonl and mirrored into the ledger under
// source "quota-prediction") lives in scripts/gather-quota.ts.

import {
  C4_METHODOLOGY_VERSION,
  PREDICTION_LEDGER_SOURCE,
  renderPredictionRange,
} from "./quota-downtime.ts";
import { forecastSeriesKey, type ForecastLedgerRow, type QuotaForecastFile } from "./quota-forecast.ts";

export interface PoolExhaustionRecord {
  record_version: 1;
  id: string;
  kind: "pool_exhaustion";
  subject: { pool: string; window: string };
  issued_at: string;
  basis: { name: "since_last_reset"; detail: string };
  inputs: {
    series: string;
    samples: number;
    span: { from: string | null; to: string };
    adjustments: string[];
  };
  prediction: { earliest: string; latest: string; rendered: string } | null;
  methodology_version: typeof C4_METHODOLOGY_VERSION;
  state: "predicted" | "insufficient_history";
  state_reason: string | null;
  supersedes: string | null;
}

/** One S2 accuracy cell: signed-error quantiles in minutes for the
 * pool/window (window cell) or the pool's all-window rollup (pool
 * band), with the scored-forecast count behind them. */
export interface PoolExhaustionErrorBand {
  /** Scored forecasts behind this band; <20 is not a band (§2). */
  count: number;
  /** Signed-error quantiles in minutes (likely + lower/upper). */
  lowerMin: number;
  upperMin: number;
  /** The ledger cell this band quotes (window cell or pool rollup). */
  cell: string;
}

export interface PoolExhaustionCalibrationCell {
  window: PoolExhaustionErrorBand | null;
  pool: PoolExhaustionErrorBand | null;
}

/** The S2 ledger's measured error per subject, keyed by
 * poolExhaustionSubjectKey. Empty when the live-scoring feed has
 * published nothing yet — every subject then honestly reports
 * insufficient_history. */
export type PoolExhaustionCalibration = ReadonlyMap<string, PoolExhaustionCalibrationCell>;

/** Minimum scored forecasts before a band calibrates a range (§2). */
export const POOL_EXHAUSTION_MIN_BAND_COUNT = 20;

/** The subject key joining a forecast window to its calibration
 * cell and to its previous record (the supersedes map). */
export function poolExhaustionSubjectKey(providerId: string, windowName: string): string {
  return `${providerId}:${windowName}`;
}

/**
 * Derive one tick's pool_exhaustion records from the tick's
 * forecast product plus the S2 accuracy calibration the caller
 * assembled. One record per walled or near-wall (projected to
 * wall before its reset) pool/window; other windows publish
 * nothing. `previous` maps a subject key to the last record id
 * issued for it, for the supersedes link (§1); pass an empty map
 * when the caller has no history.
 */
export function derivePoolExhaustionRecords(
  forecast: QuotaForecastFile,
  calibration: PoolExhaustionCalibration,
  previous: ReadonlyMap<string, string> = new Map(),
): PoolExhaustionRecord[] {
  const issuedAt = forecast.generatedAt;
  const issuedMs = Date.parse(issuedAt);
  const records: PoolExhaustionRecord[] = [];
  const emitted = new Set<string>();

  for (const providerId of Object.keys(forecast.pools).sort()) {
    for (const pool of forecast.pools[providerId]) {
      for (const w of pool.windows) {
        const isSubject = w.state === "walled" || (w.state === "projected" && w.wallsBeforeReset === true);
        if (!isSubject) continue;
        const key = poolExhaustionSubjectKey(providerId, w.name);
        if (emitted.has(key)) continue;
        emitted.add(key);

        const series = forecastSeriesKey(pool.serviceId);
        const likelyMs = w.state === "walled" ? issuedMs : (w.projectedWallAt ?? issuedMs);
        const wallContext =
          w.state === "walled"
            ? "the window is already walled at issuance, so the likely wall is the observed wall"
            : `the S1 projected wall is ${new Date(likelyMs).toISOString()} at ${w.ratePtsPerHour ?? "?"} pts/h over ${w.samples} same-period samples`;
        const basisDetail =
          `Stage-3 pace projection ${series}:${w.name} (quota-forecast.json, #12695): ${wallContext}`;

        const record = (
          over: Partial<PoolExhaustionRecord> & Pick<PoolExhaustionRecord, "id" | "state">,
        ): PoolExhaustionRecord => ({
          record_version: 1,
          kind: "pool_exhaustion",
          subject: { pool: providerId, window: w.name },
          issued_at: issuedAt,
          basis: { name: "since_last_reset", detail: "" },
          inputs: {
            series: "quota-forecast.json",
            samples: w.samples,
            span: { from: w.measuredAt, to: issuedAt },
            adjustments: [],
          },
          prediction: null,
          methodology_version: C4_METHODOLOGY_VERSION,
          state_reason: null,
          supersedes: previous.get(key) ?? null,
          ...over,
        });

        const cell = calibration.get(key);
        const windowBand = cell?.window?.count !== undefined && (cell?.window?.count ?? 0) >= POOL_EXHAUSTION_MIN_BAND_COUNT
          ? cell?.window ?? null
          : null;
        const poolBand = cell?.pool?.count !== undefined && (cell?.pool?.count ?? 0) >= POOL_EXHAUSTION_MIN_BAND_COUNT
          ? cell?.pool ?? null
          : null;
        const band = windowBand ?? poolBand;

        if (band === null) {
          const windowCount = cell?.window?.count ?? 0;
          const poolCount = cell?.pool?.count ?? 0;
          records.push(
            record({
              id: `c4:pool_exhaustion:${key}@honesty:${issuedMs}`,
              state: "insufficient_history",
              state_reason:
                cell === undefined
                  ? `no accuracy cell for ${key}: the S2 ledger has scored no forecasts for this pool/window yet`
                  : `accuracy cells too thin for ${key} (window n=${windowCount}, pool n=${poolCount}; need ${POOL_EXHAUSTION_MIN_BAND_COUNT})`,
            }),
          );
          continue;
        }

        let earliestMs = Math.round(likelyMs + band.lowerMin * 60_000);
        let latestMs = Math.round(likelyMs + band.upperMin * 60_000);
        let floorNote = "";
        if (earliestMs >= latestMs) {
          // §1 refuses earliest == latest; the ±1-minute floor is
          // resolution, not precision the ledger does not have.
          earliestMs = likelyMs - 60_000;
          latestMs = likelyMs + 60_000;
          floorNote = " (degenerate ledger band widened to the ±1-minute issuance floor)";
        }
        const earliest = new Date(earliestMs).toISOString();
        const latest = new Date(latestMs).toISOString();
        records.push(
          record({
            id: `c4:pool_exhaustion:${key}:${issuedMs}`,
            state: "predicted",
            basis: {
              name: "since_last_reset",
              detail: `${basisDetail}; range ${windowBand !== null ? "window cell" : "pool all-window band"} ${band.cell} (n=${band.count})${floorNote}`,
            },
            prediction: { earliest, latest, rendered: renderPredictionRange(earliest, latest, issuedAt) },
          }),
        );
      }
    }
  }
  return records;
}

/**
 * The ledger mirror (design §1): one row per record under the
 * C4 source "quota-prediction", beside the S1 forecast rows in
 * agent_quota_records, payload = the record verbatim so the
 * accuracy scoring (§3) joins predictions to outcomes in one
 * place. resets carries the predicted wall range's latest
 * bound; null on honesty records, which publish no range.
 */
export function buildPoolExhaustionLedgerRows(
  records: readonly PoolExhaustionRecord[],
  tickAt: string,
): ForecastLedgerRow[] {
  return records.map((r) => ({
    id: r.id,
    source: PREDICTION_LEDGER_SOURCE,
    ts: tickAt,
    plan: null,
    weeklyPercent: null,
    resets: r.prediction?.latest ?? null,
    payload: { ...r },
  }));
}
