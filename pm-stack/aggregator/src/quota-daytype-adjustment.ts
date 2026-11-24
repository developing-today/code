// C4.3 — day-type and spike-day adjustments for quota forecasts,
// published as SHADOW records (#13259 criterion 2, component of
// the C4 quota-forecasting epic #9572). The models, the
// sufficiency gate, and the earn-in rule are the C4 design:
// doc/2026-11-13T15-12-00Z_design_reporting-architecture-event-driven/
// c4-forecasting-design.md §5. Criterion 1 (PR #13290, module
// quota-daytype-sufficiency.ts) re-measured the gate over the live
// snapshot store: 51 of 2,583 cells admitted — every one at the
// daypart or dayclass-weekday grain; weekday and weekday×daypart
// cells admit none yet, because per pool/window the gatherer-era
// series covers at most 19 distinct Chicago dates. This module is
// the model those admitted cells admit.
//
// THE MODELS (§5, as written):
//
// - Day-type. "An adjustment multiplies the v1 pace by the cell's
//   measured ratio vs the pool's overall pace, and the record
//   names the cell." Burn rates here are measured by ONE
//   estimator, shared by the cell rates, the overall rate, and
//   the spike windows: within each reset period (segmentPeriods,
//   the S2 harness's segmentation), every interval between
//   consecutive readings contributes its Δpct and its Δt; a rate
//   is ΣΔpct / ΣΔt over its interval set. An interval is
//   attributed to the Chicago cell of its END reading — the
//   reading that realized the burn. The cell applied at issuance
//   is the cell the issuance instant falls in, at the finest
//   grain the data supports: the design's cells are
//   {weekday × daypart} coarsened only as far as the gate
//   requires, so the precedence is weekday-daypart, then
//   daypart, then weekday, then dayclass — the first grain whose
//   current cell is admitted (≥ DAY_TYPE_SAMPLE_GATE independent
//   samples, counted exactly as criterion 1 counted them: one
//   America/Chicago calendar date with ≥1 reading in the cell,
//   over the same rename-joined series the S2 harness assembles)
//   AND whose measured ratio is finite and positive. A cell with
//   samples but no measurable positive burn does not adjust —
//   it falls through to the coarser grain, and the record's
//   reason says so.
// - Spike-day (S2.7). When the pool/window's trailing-7-day burn
//   runs materially under its trailing-30-day pace — the
//   design's shape is ~10% light — the model predicts a
//   compensating spike day at +20% pace inside the current
//   window and says so explicitly: a `spike_day` entry in
//   inputs.adjustments, the two measured rates and the light
//   fraction on the record, and the spike wall computed at the
//   (day-type-adjusted, when present) pace × 1.2. No silent
//   widening. The spike verdict needs a real 30-day basis:
//   intervals ending on ≥14 distinct Chicago dates inside the
//   trailing 30 days and ≥2 inside the trailing 7; below that
//   there is no verdict at all (the spike is an adjustment, not
//   a state — its absence is silent, the day-type honesty state
//   below covers the record).
//
// EARN-IN (§5): "An admitted adjustment must beat the unadjusted
// forecast in the S2 harness over the same history before it
// becomes the published forecast; until then it publishes as a
// shadow record and is scored alongside." These records are
// therefore SHADOW ONLY, in the Stage-3 S3 pattern: nothing in
// the live path reads them, the published forecast
// (quota-forecast.json) is never modified by this module, and
// the records accumulate in quota-daytype-shadow.jsonl beside
// the forecast product — deliberately NOT the C4 prediction log
// (quota-predictions.jsonl) and NOT the quota-prediction ledger
// source, because they are not the published forecast and must
// not be scored as it. Each record carries BOTH walls (the v1
// projection verbatim and the adjusted projection) so the
// criterion-3 harness comparison needs nothing else. A
// pool/window with no live v1 projection publishes no record —
// there is nothing to adjust; a pool/window whose current cell
// is admitted at no grain publishes the design's honesty state,
// adjustment_unavailable, with the per-grain sample counts that
// decided it.
//
// Methodology version c4-meth:5 (c4-methodology-log.jsonl beside
// this package's root, design §6): the entry lands with this
// module, so every shadow record is attributable to the method
// that produced it.
import {
  indexPercentRecords,
  pointsForWindow,
  segmentPeriods,
} from "./quota-forecast-accuracy.ts";
import {
  DAY_TYPE_GRAINS,
  DAY_TYPE_SAMPLE_GATE,
  cellsFor,
  dayTypeParts,
  type DayTypeGrain,
} from "./quota-daytype-sufficiency.ts";
import type { PctPoint } from "./quota-analytics.ts";
import type { QuotaForecastFile } from "./quota-forecast.ts";
import type { SnapshotRecord } from "./model.ts";

export const DAYTYPE_METHODOLOGY_VERSION = "c4-meth:5" as const;

/** S2.7's shape, as constants: the trailing-7-day burn running at
 * least this fraction under the trailing-30-day pace is "materially
 * light" and predicts the spike day. */
export const SPIKE_LIGHT_FRACTION = 0.1;
/** The compensating spike day's pace, relative to the (adjusted)
 * pace: the design's ~+20%. */
export const SPIKE_PACE_MULTIPLIER = 1.2;
export const SPIKE_SHORT_WINDOW_DAYS = 7;
export const SPIKE_LONG_WINDOW_DAYS = 30;
/** Distinct Chicago dates with interval ends inside the trailing
 * 30 days before a "trailing-30-day pace" is a measured basis and
 * not a few days wearing a 30-day name. */
export const SPIKE_MIN_LONG_DATES = 14;
/** Distinct Chicago dates with interval ends inside the trailing
 * 7 days before the short window's rate is measurable. */
export const SPIKE_MIN_SHORT_DATES = 2;

/** Grain precedence at issuance: the design's {weekday × daypart}
 * cells, coarsened only as far as the gate requires (§5). */
export const DAY_TYPE_GRAIN_PRECEDENCE: readonly DayTypeGrain[] = [
  "weekday-daypart",
  "daypart",
  "weekday",
  "dayclass",
];

const HOUR_MS = 3_600_000;
const DAY_MS = 24 * HOUR_MS;

/** Forecast-product window name -> the S2 harness's canonical
 * window name (FORECAST_WINDOW_METRICS' keys). The forecast
 * product spells windows as quota-current does ("7d", "5-hour");
 * the harness's pointsForWindow keys its alias table by the
 * canonical names ("weekly", "5h") — assembling with the
 * product's spelling would silently drop the records filed
 * under the other spelling (the store writes both). */
const CANONICAL_WINDOW: Record<string, string> = {
  "5h": "5h",
  "5-hour": "5h",
  "7d": "weekly",
  "7-day": "weekly",
  weekly: "weekly",
  monthly: "monthly",
  api: "api",
  vibe: "vibe",
};

export interface BurnInterval {
  /** The interval's end reading instant (ms epoch) — the reading
   * that realized the burn, and the instant whose Chicago cell
   * the interval is attributed to. */
  endTs: number;
  dtMs: number;
  dPct: number;
}

/**
 * The shared burn estimator's intervals: within each reset
 * period, the consecutive-reading intervals. Intervals never
 * cross a reset (a period boundary is a new window, not burn).
 */
export function burnIntervals(points: readonly PctPoint[]): BurnInterval[] {
  const out: BurnInterval[] = [];
  for (const period of segmentPeriods(points)) {
    const pts = [...period.points].sort((a, b) => a.ts - b.ts);
    for (let i = 1; i < pts.length; i++) {
      const dtMs = (pts[i] as PctPoint).ts - (pts[i - 1] as PctPoint).ts;
      if (dtMs <= 0) continue;
      out.push({ endTs: (pts[i] as PctPoint).ts, dtMs, dPct: (pts[i] as PctPoint).pct - (pts[i - 1] as PctPoint).pct });
    }
  }
  return out;
}

function rateOver(intervals: readonly BurnInterval[]): number | null {
  let dt = 0;
  let dp = 0;
  for (const iv of intervals) {
    dt += iv.dtMs;
    dp += iv.dPct;
  }
  if (dt <= 0) return null;
  return (dp / dt) * HOUR_MS;
}

export interface CellBurnRate {
  grain: DayTypeGrain;
  cell: string;
  /** Independent samples behind the cell: distinct Chicago dates
   * with ≥1 reading in it (the criterion-1 gate unit). */
  samples: number;
  ratePtsPerHour: number | null;
  /** The cell's rate vs the pool/window's overall rate; null when
   * either rate is unmeasurable or the overall rate is not
   * positive. */
  ratio: number | null;
}

export interface CellBurnRates {
  overallRatePtsPerHour: number | null;
  cells: CellBurnRate[];
}

/**
 * Burn-rate distribution per day-type cell over one pool/window
 * series, plus the independent-sample count per cell (the gate).
 * Samples are counted over the series' readings (as criterion 1
 * counted them); rates over the shared interval estimator.
 */
export function measureCellBurnRates(points: readonly PctPoint[]): CellBurnRates {
  const intervals = burnIntervals(points);
  const overall = rateOver(intervals);
  const intervalsByCell = new Map<string, BurnInterval[]>();
  const daysByCell = new Map<string, Set<string>>();
  for (const grain of DAY_TYPE_GRAINS) {
    for (const iv of intervals) {
      const key = `${grain}:${cellsFor(dayTypeParts(iv.endTs))[grain]}`;
      const list = intervalsByCell.get(key) ?? [];
      list.push(iv);
      intervalsByCell.set(key, list);
    }
  }
  for (const p of points) {
    const parts = dayTypeParts(p.ts);
    const cells = cellsFor(parts);
    for (const grain of DAY_TYPE_GRAINS) {
      const key = `${grain}:${cells[grain]}`;
      const days = daysByCell.get(key) ?? new Set<string>();
      days.add(parts.date);
      daysByCell.set(key, days);
    }
  }
  const cells: CellBurnRate[] = [];
  for (const grain of DAY_TYPE_GRAINS) {
    const names = new Set<string>();
    for (const key of intervalsByCell.keys()) if (key.startsWith(`${grain}:`)) names.add(key.slice(grain.length + 1));
    for (const key of daysByCell.keys()) if (key.startsWith(`${grain}:`)) names.add(key.slice(grain.length + 1));
    for (const cell of [...names].sort()) {
      const key = `${grain}:${cell}`;
      const rate = rateOver(intervalsByCell.get(key) ?? []);
      cells.push({
        grain,
        cell,
        samples: daysByCell.get(key)?.size ?? 0,
        ratePtsPerHour: rate,
        ratio: rate !== null && overall !== null && overall > 0 ? rate / overall : null,
      });
    }
  }
  return { overallRatePtsPerHour: overall, cells };
}

export interface SpikeDayAssessment {
  detected: boolean;
  rateShortPtsPerHour: number;
  rateLongPtsPerHour: number;
  /** 1 − short/long: how far under its 30-day pace the trailing
   * 7 days ran. Detection is lightFraction ≥ SPIKE_LIGHT_FRACTION
   * with a positive long rate. */
  lightFraction: number;
  shortDates: number;
  longDates: number;
}

/**
 * The S2.7 spike-day verdict over one pool/window series, or
 * null when the trailing windows cannot support one (the long
 * window's basis is thinner than SPIKE_MIN_LONG_DATES distinct
 * dates, or either rate is unmeasurable).
 */
export function assessSpikeDay(points: readonly PctPoint[], nowMs: number): SpikeDayAssessment | null {
  const intervals = burnIntervals(points);
  const inWindow = (days: number) => intervals.filter((iv) => iv.endTs > nowMs - days * DAY_MS && iv.endTs <= nowMs);
  const short = inWindow(SPIKE_SHORT_WINDOW_DAYS);
  const long = inWindow(SPIKE_LONG_WINDOW_DAYS);
  const dates = (ivs: readonly BurnInterval[]) => new Set(ivs.map((iv) => dayTypeParts(iv.endTs).date)).size;
  const shortDates = dates(short);
  const longDates = dates(long);
  if (longDates < SPIKE_MIN_LONG_DATES || shortDates < SPIKE_MIN_SHORT_DATES) return null;
  const rateShort = rateOver(short);
  const rateLong = rateOver(long);
  if (rateShort === null || rateLong === null) return null;
  const lightFraction = rateLong > 0 ? 1 - rateShort / rateLong : 0;
  return {
    detected: rateLong > 0 && lightFraction >= SPIKE_LIGHT_FRACTION,
    rateShortPtsPerHour: rateShort,
    rateLongPtsPerHour: rateLong,
    lightFraction,
    shortDates,
    longDates,
  };
}

export interface DayTypeAdjustment {
  grain: DayTypeGrain;
  cell: string;
  samples: number;
  ratio: number;
  ratePtsPerHour: number;
  /** now + (100 − pctNow) / adjusted rate; null when the adjusted
   * rate cannot reach the wall (never, at a non-positive rate —
   * excluded by the ratio > 0 admission rule, kept for shape). */
  wallAt: string | null;
}

export interface DayTypeShadowRecord {
  record_version: 1;
  id: string;
  kind: "daytype_shadow";
  /** Earn-in (§5): a shadow record is scored alongside the
   * published forecast; it is never the published forecast. */
  shadow: true;
  subject: { pool: string; serviceId: string; window: string };
  issued_at: string;
  basis: { name: "since_last_reset"; detail: string };
  inputs: {
    series: string;
    /** The v1 forecast's same-period samples (the S1 row's). */
    samples: number;
    span: { from: string | null; to: string };
    adjustments: string[];
  };
  methodology_version: typeof DAYTYPE_METHODOLOGY_VERSION;
  state: "shadow" | "adjustment_unavailable";
  state_reason: string | null;
  /** The v1 pace projection, verbatim from the forecast product. */
  unadjusted: { ratePtsPerHour: number; projectedWallAt: string };
  day_type: DayTypeAdjustment | null;
  spike_day: {
    rateShortPtsPerHour: number;
    rateLongPtsPerHour: number;
    lightFraction: number;
    paceMultiplier: number;
    /** The wall if the predicted spike day's pace held over the
     * whole horizon — the widened range's near bound, stated
     * explicitly per §5 (no silent widening). */
    wallAt: string | null;
  } | null;
  supersedes: string | null;
}

export function dayTypeShadowSubjectKey(pool: string, serviceId: string, window: string): string {
  return `${pool}|${serviceId}|${window}`;
}

function wallAt(nowMs: number, pctNow: number, ratePtsPerHour: number): string | null {
  if (!(ratePtsPerHour > 0)) return null;
  return new Date(nowMs + ((100 - pctNow) / ratePtsPerHour) * HOUR_MS).toISOString();
}

/**
 * Build one tick's day-type shadow records from the tick's
 * forecast product and the snapshot history. Pure: no I/O, no
 * clock reads except through arguments. Only pool/windows with a
 * live v1 projection (state "projected", a positive measured
 * rate, a current reading) get a record — anything else has
 * nothing to adjust. `previous` maps dayTypeShadowSubjectKey to
 * the last shadow record id per subject, for the supersedes
 * chain.
 */
export function buildDayTypeShadowRecords(
  forecast: QuotaForecastFile,
  records: readonly SnapshotRecord[],
  previous: ReadonlyMap<string, string>,
  now = new Date(forecast.generatedAt),
): DayTypeShadowRecord[] {
  const nowMs = now.getTime();
  const { index } = indexPercentRecords(records);
  const nowCells = cellsFor(dayTypeParts(nowMs));
  const out: DayTypeShadowRecord[] = [];

  for (const providerId of Object.keys(forecast.pools).sort()) {
    for (const pool of forecast.pools[providerId] as NonNullable<QuotaForecastFile["pools"][string]>) {
      for (const window of pool.windows) {
        if (window.state !== "projected") continue;
        if (window.ratePtsPerHour === null || !(window.ratePtsPerHour > 0)) continue;
        if (window.pctNow === null || window.projectedWallAt === null) continue;

        const points = pointsForWindow(index, pool.serviceId, CANONICAL_WINDOW[window.name] ?? window.name);
        const rates = measureCellBurnRates(points);
        const byKey = new Map(rates.cells.map((c) => [`${c.grain}:${c.cell}`, c]));

        let dayType: DayTypeAdjustment | null = null;
        const diagnostics: string[] = [];
        for (const grain of DAY_TYPE_GRAIN_PRECEDENCE) {
          const cell = nowCells[grain];
          const measured = byKey.get(`${grain}:${cell}`);
          const samples = measured?.samples ?? 0;
          if (samples < DAY_TYPE_SAMPLE_GATE) {
            diagnostics.push(`${grain} '${cell}' holds ${samples} samples vs the ≥${DAY_TYPE_SAMPLE_GATE} gate`);
            continue;
          }
          if (measured?.ratio === null || measured === undefined || !(measured.ratio > 0)) {
            diagnostics.push(`${grain} '${cell}' is admitted (${samples} samples) but its burn ratio is unmeasurable`);
            continue;
          }
          const rate = window.ratePtsPerHour * measured.ratio;
          dayType = {
            grain,
            cell,
            samples,
            ratio: measured.ratio,
            ratePtsPerHour: rate,
            wallAt: wallAt(nowMs, window.pctNow, rate),
          };
          break;
        }

        const spike = assessSpikeDay(points, nowMs);
        const spikeDay =
          spike !== null && spike.detected
            ? {
                rateShortPtsPerHour: spike.rateShortPtsPerHour,
                rateLongPtsPerHour: spike.rateLongPtsPerHour,
                lightFraction: spike.lightFraction,
                paceMultiplier: SPIKE_PACE_MULTIPLIER,
                wallAt: wallAt(nowMs, window.pctNow, (dayType?.ratePtsPerHour ?? window.ratePtsPerHour) * SPIKE_PACE_MULTIPLIER),
              }
            : null;

        const adjustments: string[] = [];
        if (dayType) adjustments.push(`day_type:${dayType.grain}:${dayType.cell}`);
        if (spikeDay) adjustments.push("spike_day");

        const key = dayTypeShadowSubjectKey(providerId, pool.serviceId, window.name);
        out.push({
          record_version: 1,
          id: `c4.3:shadow:${providerId}:${window.name}:${nowMs}`,
          kind: "daytype_shadow",
          shadow: true,
          subject: { pool: providerId, serviceId: pool.serviceId, window: window.name },
          issued_at: now.toISOString(),
          basis: {
            name: "since_last_reset",
            detail:
              `v1 pace is the S1 product's projectWindow projection over the current reset period; ` +
              `day-type ratios and spike windows are measured over the full rename-joined percent series ` +
              `for this pool/window (${points.length} readings, all reset periods) with the shared ` +
              `interval estimator; cells are America/Chicago calendar cells.`,
          },
          inputs: {
            series: pool.serviceId,
            samples: window.samples,
            span: {
              from: points.length ? new Date((points[0] as PctPoint).ts).toISOString() : null,
              to: now.toISOString(),
            },
            adjustments,
          },
          methodology_version: DAYTYPE_METHODOLOGY_VERSION,
          state: adjustments.length ? "shadow" : "adjustment_unavailable",
          state_reason: adjustments.length
            ? null
            : `no admitted day-type cell for the issuance instant and no spike day predicted: ${diagnostics.join("; ")}`,
          unadjusted: {
            ratePtsPerHour: window.ratePtsPerHour,
            projectedWallAt: new Date(window.projectedWallAt).toISOString(),
          },
          day_type: dayType,
          spike_day: spikeDay,
          supersedes: previous.get(key) ?? null,
        });
      }
    }
  }
  return out;
}

/** The human report for the shadow command: one row per record —
 * the two walls, the cell and ratio behind the adjusted one, and
 * the spike verdict — with the honesty states named. */
export function renderDayTypeShadowReport(records: readonly DayTypeShadowRecord[]): string {
  const lines: string[] = [];
  lines.push("# Day-type shadow adjustments — C4.3 (#13259 criterion 2)");
  lines.push("");
  lines.push(
    `Shadow records only (C4 design §5 earn-in): the adjusted forecast must beat the ` +
      `unadjusted forecast in the S2 harness before it publishes. Methodology ` +
      `${DAYTYPE_METHODOLOGY_VERSION}; gate ≥${DAY_TYPE_SAMPLE_GATE} independent samples per cell.`,
  );
  lines.push("");
  lines.push("| pool | window | state | unadjusted wall | day-type cell | ratio | adjusted wall | spike |");
  lines.push("| --- | --- | --- | --- | --- | ---: | --- | --- |");
  for (const r of records) {
    lines.push(
      `| ${r.subject.pool} | ${r.subject.window} | ${r.state} | ${r.unadjusted.projectedWallAt} | ` +
        `${r.day_type ? `${r.day_type.grain}:${r.day_type.cell}` : "—"} | ` +
        `${r.day_type ? r.day_type.ratio.toFixed(3) : "—"} | ${r.day_type?.wallAt ?? "—"} | ` +
        `${r.spike_day ? `light ${(r.spike_day.lightFraction * 100).toFixed(1)}% → wall ${r.spike_day.wallAt ?? "—"}` : "—"} |`,
    );
  }
  lines.push("");
  return lines.join("\n");
}
