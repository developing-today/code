// C4.2 — reset-duration predictions as ranges (#13258, component
// of the C4 quota-forecasting epic #9572). The record format,
// storage, and scoring contract are the C4 design:
// doc/2026-11-13T15-12-00Z_design_reporting-architecture-event-driven/
// c4-forecasting-design.md (§1 the prediction record, §4 range
// rendering, §7 the reset_duration kind), landed by PR #13268.
// The current-window input is the Stage-3 forecast product
// (quota-forecast.json, #12695); the cadence input is the
// snapshot store's resetsAt history for the pool/window,
// assembled by the caller into ResetObservation lists keyed by
// resetDurationSubjectKey (the gatherer wiring slice joins the
// store rename-folded via forecastSeriesKey, the S1 join).
//
// Derivation (§7): basis reset_cadence — the pool's observed
// reset intervals in the snapshot store (resetsAt deltas across
// periods), giving the reopening range for a wall. When the
// window's resetsAt is published, the prediction is the wait
// until it, ranged by the cadence's observed jitter (realized
// resets vs the resetsAt published for their period). When no
// reset is published for the current window, the next reset is
// projected one observed cadence interval past the last observed
// period's published reset, ranged by the cadence spread and
// the same jitter.
//
// The prediction range (§1, §3): earliest/latest bound the
// REOPENING instant — the duration is start-anchored (the wall
// instant is observed; for a near-wall window the projected wall
// rides in the basis detail), the reopening is predicted, and
// scoring is coverage + width on the reopening. earliest ==
// latest is refused: where the observed spread is degenerate
// the range widens to the observation resolution (half the
// median reading gap — the store cannot resolve a reset more
// finely than it samples), and a model that cannot bound its
// answer at all emits an honesty state instead (§1).
//
// Honesty (§1): insufficient_history (history exists but holds
// no completed reset period — no jitter is measured — or the
// ranged reopening already lies in the past, i.e. the inputs
// are stale relative to the tick) and no_basis (no snapshot
// history for the pool/window at all) are published states with
// named reasons — never an invented range, never a silent
// absence. Windows neither walled nor projected to wall before
// their reset are not this kind's subject and publish no
// record. A provider with several pool rows for the same
// window (split pools) yields one record per window — the
// first row in the product's order wins.
//
// Rendering (§4): the stored `rendered` string comes from the
// one shared formatter (renderPredictionRange in
// quota-downtime.ts) at issuance — what was shown is exactly
// what is scored.
//
// Basis (§1: mandatory, singular): reset_cadence for every
// record this module emits. Methodology version c4-meth:3 (the
// C4 methodology log, §6 — the version every C4 kind's current
// records name). Advisory only: no placement path reads these
// records — the C6 pages and forecast alerts named by #9572
// consume them, and the alerts module already rules that
// reset_duration records never alert standalone
// (quota-forecast-alerts.ts).
//
// Pure functions over plain values — no I/O, no clock reads
// except through arguments. The gatherer-tick wiring (records
// logged per walled/near-wall pool on ticks, appended to
// quota-predictions.jsonl and mirrored into the ledger under
// source "quota-prediction") is the follow-on slice this
// module's builders serve.

import { PROJECTION_PERIOD_TOLERANCE_MS } from "./quota-analytics.ts";
import { forecastSeriesKey, type ForecastLedgerRow, type QuotaForecastFile } from "./quota-forecast.ts";
import { C4_METHODOLOGY_VERSION, PREDICTION_LEDGER_SOURCE, renderPredictionRange } from "./quota-downtime.ts";

export interface ResetDurationRecord {
  record_version: 1;
  id: string;
  kind: "reset_duration";
  subject: { pool: string; window: string };
  issued_at: string;
  basis: { name: "reset_cadence"; detail: string };
  inputs: {
    series: string;
    samples: number;
    span: { from: string | null; to: string };
    adjustments: string[];
  };
  prediction: { earliest: string; latest: string; rendered: string } | null;
  methodology_version: typeof C4_METHODOLOGY_VERSION;
  state: "predicted" | "insufficient_history" | "no_basis";
  state_reason: string | null;
  supersedes: string | null;
}

/**
 * One snapshot-store reading for a pool/window, reduced to what
 * the cadence basis reads: when it was observed, and the reset
 * instant it reported (null when the reading carried none).
 */
export interface ResetObservation {
  ts: string;
  resetsAt: string | null;
}

/** The subject key joining a forecast window to its snapshot
 * history (the observations map) and to its previous record
 * (the supersedes map). */
export function resetDurationSubjectKey(providerId: string, windowName: string): string {
  return `${providerId}:${windowName}`;
}

// --- Cadence basis -------------------------------------------------

interface ResetPeriod {
  /** The reset instant the period's readings published (ms). */
  reportedResetMs: number;
  firstTs: number;
  lastTs: number;
}

interface CadenceBasis {
  periods: ResetPeriod[];
  /** Realized - published per completed period (ms, signed).
   * The realized instant is estimated as the midpoint between
   * the last reading of the closing period and the first
   * reading of the next — the finest estimate the sampling
   * supports, and the reason the range widens by the
   * observation resolution when the spread is degenerate. */
  jitterMs: number[];
  /** Deltas between consecutive periods' published resets. */
  cadenceMs: number[];
  /** Usable readings behind the basis. */
  samples: number;
  /** Half the median gap between consecutive readings (ms),
   * floored at one minute: the store's resolution for a reset
   * instant. */
  resolutionMs: number;
  spanFrom: string | null;
  spanTo: string | null;
}

function median(xs: number[]): number {
  const s = [...xs].sort((a, b) => a - b);
  const mid = Math.floor(s.length / 2);
  return s.length % 2 === 1 ? s[mid] : (s[mid - 1] + s[mid]) / 2;
}

function computeCadence(observations: readonly ResetObservation[]): CadenceBasis {
  const usable = observations
    .map((o) => ({ ts: Date.parse(o.ts), resetsAt: o.resetsAt ? Date.parse(o.resetsAt) : NaN }))
    .filter((o) => Number.isFinite(o.ts))
    .sort((a, b) => a.ts - b.ts);

  const periods: ResetPeriod[] = [];
  const jitterMs: number[] = [];
  const cadenceMs: number[] = [];
  let current: ResetPeriod | null = null;
  for (const o of usable) {
    if (!Number.isFinite(o.resetsAt)) {
      // A reading that reports no reset still observes the open
      // period — but only before its published reset. At or past
      // it, the period may have rolled over unobserved, and
      // advancing lastTs would bill the publication gap to the
      // reset as positive jitter (the accuracy harness's
      // distinction: null readings at or beyond the prior reset
      // open an unknown-reset segment, quota-forecast-accuracy.ts).
      if (current !== null && o.ts < current.reportedResetMs) current.lastTs = o.ts;
      continue;
    }
    const r = o.resetsAt;
    if (current === null) {
      current = { reportedResetMs: r, firstTs: o.ts, lastTs: o.ts };
      periods.push(current);
      continue;
    }
    if (Math.abs(r - current.reportedResetMs) > PROJECTION_PERIOD_TOLERANCE_MS) {
      // A new period: the previous one is complete. Two reported
      // resets closer than the tolerance are the same period
      // (the accuracy harness's segmentation rule — one period's
      // reported reset jitters by seconds, adjacent periods
      // differ by at least the shortest window).
      const realized = Math.round((current.lastTs + o.ts) / 2);
      jitterMs.push(realized - current.reportedResetMs);
      cadenceMs.push(r - current.reportedResetMs);
      current = { reportedResetMs: r, firstTs: o.ts, lastTs: o.ts };
      periods.push(current);
      continue;
    }
    current.lastTs = o.ts;
  }

  const gaps: number[] = [];
  for (let i = 1; i < usable.length; i++) gaps.push(usable[i].ts - usable[i - 1].ts);
  const resolutionMs = Math.max(60_000, gaps.length > 0 ? median(gaps) / 2 : 0);
  return {
    periods,
    jitterMs,
    cadenceMs,
    samples: usable.length,
    resolutionMs,
    spanFrom: usable.length > 0 ? new Date(usable[0].ts).toISOString() : null,
    spanTo: usable.length > 0 ? new Date(usable[usable.length - 1].ts).toISOString() : null,
  };
}

/** Widen a degenerate range to the observation resolution:
 * earliest == latest is refused (§1), and a spread narrower
 * than the sampling can resolve claims precision the store
 * does not have. */
function rangedByResolution(lo: number, hi: number, resolutionMs: number): [number, number] {
  if (hi - lo >= 2 * resolutionMs) return [lo, hi];
  const mid = (lo + hi) / 2;
  return [Math.floor(mid - resolutionMs), Math.ceil(mid + resolutionMs)];
}

const fmtSignedDur = (ms: number): string => {
  const sign = ms < 0 ? "-" : "+";
  const abs = Math.abs(ms);
  if (abs >= 2 * 3_600_000) return `${sign}${(abs / 3_600_000).toFixed(1)}h`;
  return `${sign}${Math.round(abs / 60_000)}m`;
};

// --- Derivation ----------------------------------------------------

/**
 * Derive one tick's reset_duration records from the tick's
 * forecast product plus the snapshot history the caller
 * assembled. One record per walled or near-wall (projected to
 * wall before its reset) pool/window; other windows publish
 * nothing. `previous` maps a subject key
 * (resetDurationSubjectKey) to the last record id issued for
 * it, for the supersedes link (§1); pass an empty map when the
 * caller has no history.
 */
export function deriveResetDurationRecords(
  forecast: QuotaForecastFile,
  observations: ReadonlyMap<string, readonly ResetObservation[]>,
  previous: ReadonlyMap<string, string> = new Map(),
): ResetDurationRecord[] {
  const issuedAt = forecast.generatedAt;
  const issuedMs = Date.parse(issuedAt);
  const records: ResetDurationRecord[] = [];
  const emitted = new Set<string>();

  for (const providerId of Object.keys(forecast.pools).sort()) {
    for (const pool of forecast.pools[providerId]) {
      for (const w of pool.windows) {
        const isSubject = w.state === "walled" || (w.state === "projected" && w.wallsBeforeReset === true);
        if (!isSubject) continue;
        const key = resetDurationSubjectKey(providerId, w.name);
        if (emitted.has(key)) continue;
        emitted.add(key);

        const series = forecastSeriesKey(pool.serviceId);
        const basis = computeCadence(observations.get(key) ?? []);
        const wallContext =
          w.state === "walled"
            ? "the window is already walled at issuance, so the duration runs from the observed wall"
            : `the projected wall is ${w.projectedWallAt != null ? new Date(w.projectedWallAt).toISOString() : "unmeasured"}, so the duration runs from the projected wall`;

        const record = (
          over: Partial<ResetDurationRecord> & Pick<ResetDurationRecord, "id" | "state">,
        ): ResetDurationRecord => ({
          record_version: 1,
          kind: "reset_duration",
          subject: { pool: providerId, window: w.name },
          issued_at: issuedAt,
          basis: { name: "reset_cadence", detail: "" },
          inputs: {
            series,
            samples: basis.samples,
            span: { from: basis.spanFrom, to: basis.spanTo ?? issuedAt },
            adjustments: [],
          },
          prediction: null,
          methodology_version: C4_METHODOLOGY_VERSION,
          state_reason: null,
          supersedes: previous.get(key) ?? null,
          ...over,
        });
        const honesty = (state: "insufficient_history" | "no_basis", reason: string): ResetDurationRecord =>
          record({ id: `c4:reset_duration:${key}@honesty:${issuedMs}`, state, state_reason: reason });

        if (basis.samples === 0) {
          records.push(
            honesty(
              "no_basis",
              "no snapshot history for this pool/window — no reset cadence is observed, and an unobserved cadence is not a basis",
            ),
          );
          continue;
        }

        const forecastResetMs = w.resetsAt ? Date.parse(w.resetsAt) : NaN;
        const lastPeriod = basis.periods[basis.periods.length - 1] ?? null;
        let anchorMs: number | null = null;
        let anchorWording = "";
        if (Number.isFinite(forecastResetMs)) {
          anchorMs = forecastResetMs;
          anchorWording = `the published resetsAt ${new Date(forecastResetMs).toISOString()}`;
        } else if (lastPeriod !== null && lastPeriod.reportedResetMs > issuedMs) {
          anchorMs = lastPeriod.reportedResetMs;
          anchorWording = `the snapshot store's current-period resetsAt ${new Date(lastPeriod.reportedResetMs).toISOString()} (the forecast row publishes none)`;
        }

        if (anchorMs !== null) {
          if (basis.jitterMs.length === 0) {
            records.push(
              honesty(
                "insufficient_history",
                `the window's reset is published but no completed reset period is observed for this pool/window (${basis.periods.length} period(s) in the store) — no reset jitter is measured, so the published reset cannot be ranged`,
              ),
            );
            continue;
          }
          const lo = anchorMs + Math.min(...basis.jitterMs);
          const hi = anchorMs + Math.max(...basis.jitterMs);
          const [earliestMs, latestMs] = rangedByResolution(lo, hi, basis.resolutionMs);
          if (latestMs <= issuedMs) {
            records.push(
              honesty(
                "insufficient_history",
                "the published reset, ranged by the observed jitter, already lies in the past at issuance — the forecast row is stale relative to the tick and no future reopening can be bounded from it",
              ),
            );
            continue;
          }
          const earliest = new Date(earliestMs).toISOString();
          const latest = new Date(latestMs).toISOString();
          records.push(
            record({
              id: `c4:reset_duration:${key}:${issuedMs}`,
              state: "predicted",
              basis: {
                name: "reset_cadence",
                detail:
                  `reopening anchored on ${anchorWording}, ranged by the reset jitter observed over ${basis.jitterMs.length} completed reset period(s) ` +
                  `(realized minus published: ${fmtSignedDur(Math.min(...basis.jitterMs))} to ${fmtSignedDur(Math.max(...basis.jitterMs))}), ` +
                  `series ${series} rename-joined via forecastSeriesKey; ${wallContext}`,
              },
              prediction: { earliest, latest, rendered: renderPredictionRange(earliest, latest, issuedAt) },
            }),
          );
          continue;
        }

        // No published reset anywhere: project one cadence
        // interval past the last observed period's reset.
        if (lastPeriod === null || basis.cadenceMs.length === 0 || basis.jitterMs.length === 0) {
          records.push(
            honesty(
              "insufficient_history",
              `no reset is published for the current window and the observed history holds ${basis.periods.length} period(s) with ${basis.cadenceMs.length} completed interval(s) — a cadence projection needs at least one completed interval and its jitter`,
            ),
          );
          continue;
        }
        const candidates = basis.cadenceMs.map((c) => lastPeriod.reportedResetMs + c);
        const lo = Math.min(...candidates) + Math.min(...basis.jitterMs);
        const hi = Math.max(...candidates) + Math.max(...basis.jitterMs);
        const [earliestMs, latestMs] = rangedByResolution(lo, hi, basis.resolutionMs);
        if (latestMs <= issuedMs) {
          records.push(
            honesty(
              "insufficient_history",
              "no reset is published for the current window and the observed cadence places the next reset in the past — the snapshot history is stale relative to the tick",
            ),
          );
          continue;
        }
        const earliest = new Date(earliestMs).toISOString();
        const latest = new Date(latestMs).toISOString();
        records.push(
          record({
            id: `c4:reset_duration:${key}:${issuedMs}`,
            state: "predicted",
            basis: {
              name: "reset_cadence",
              detail:
                `no reset is published for the current window; the next reset is projected one observed cadence interval ` +
                `(${basis.cadenceMs.map((c) => fmtSignedDur(c)).join(", ")}) past the last published reset ${new Date(lastPeriod.reportedResetMs).toISOString()}, ` +
                `ranged by the cadence spread and the reset jitter observed over ${basis.jitterMs.length} completed period(s) ` +
                `(realized minus published: ${fmtSignedDur(Math.min(...basis.jitterMs))} to ${fmtSignedDur(Math.max(...basis.jitterMs))}), ` +
                `series ${series} rename-joined via forecastSeriesKey; ${wallContext}`,
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
 * place. resets carries the predicted reopening range's latest
 * bound, mirroring the forecast rows' resets convention; null
 * on honesty records, which publish no range.
 */
export function buildResetDurationLedgerRows(
  records: readonly ResetDurationRecord[],
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
