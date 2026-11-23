// Day-type sample-sufficiency re-measure for the C4 forecast
// adjustments (#13259, slice C4.3 of #9572; C4 design
// doc/2026-11-13T15-12-00Z_design_reporting-architecture-event-driven/
// c4-forecasting-design.md §5).
//
// The Stage-3 design deferred day-type modelling on evidence: at
// 33 days of snapshot-store history each weekday cell held ~4–5
// samples per window — too thin to tune on (Stage-3 design §2, §8
// alternative B). The C4 design defines the models and the gate
// that admits them, and does not waive the gate:
//
//   "An adjustment ships for a cell only when the cell holds ≥8
//    independent samples (Stage-3 measured ~4–5 per weekday at
//    33 days — under gate ...) Below the gate:
//    adjustment_unavailable, unadjusted forecast stands. Admission
//    is measured, not scheduled (C4.3 re-measures first)."
//
// This module is that re-measure. It reads the same percent series
// the forecast is fit on — indexPercentRecords' rename join and
// FORECAST_WINDOW_METRICS, so the measurement and the S2 accuracy
// harness agree on the inputs — and counts, per pool/window, the
// independent samples in every day-type cell on the design's grain
// ladder (§5: cells are {weekday × daypart} "at the coarsest grain
// the data supports"):
//
//   weekday          Mon..Sun            ("other Fridays", S2.6)
//   dayclass         weekday | weekend   ("other weekends", S2.6)
//   daypart          night | morning | afternoon | evening
//                                        ("same time-of-day on
//                                         other days", S2.6)
//   weekday-daypart  weekday × daypart   (the finest cell, §5)
//
// An independent sample is one America/Chicago calendar date
// contributing at least one reading to the cell. Readings inside a
// single occurrence of a cell are one burn episode, not independent
// evidence — at the gatherer's ~15-minute cadence, counting
// readings would call a single Friday morning ~24 samples and the
// gate would admit cells on one day's data. Dayparts are the
// design §4 names on quarter-day Chicago wall-clock blocks: night
// 00–06, morning 06–12, afternoon 12–18, evening 18–24. Day-type is
// Drew-local, so cells are Chicago calendar cells (the design §4
// wall clock), never UTC cells.
//
// The measurement changes nothing: a cell at or over the gate is
// reported "admitted" — eligible for the adjustment slice, which
// must still earn in against the unadjusted forecast in the S2
// harness before it publishes (§5) — and a cell under the gate is
// reported with the design's honesty state,
// adjustment_unavailable. Pools are the store's folded series keys
// (the rename join's output); each row names the raw service ids
// folded into it, so the join stays auditable.
import { FORECAST_WINDOW_METRICS, FORECAST_WINDOWS, indexPercentRecords } from "./quota-forecast-accuracy.ts";
import type { SnapshotRecord } from "./model.ts";

/** The C4 design §5 sufficiency gate: the independent samples a
 * cell must hold before an adjustment may ship for it. */
export const DAY_TYPE_SAMPLE_GATE = 8;

/** Day-type cells are America/Chicago calendar cells (design §4). */
export const DAY_TYPE_TIME_ZONE = "America/Chicago";

export const DAY_TYPE_WEEKDAYS = ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"] as const;
export const DAY_TYPE_DAYCLASSES = ["weekday", "weekend"] as const;
export const DAY_TYPE_DAYPARTS = ["night", "morning", "afternoon", "evening"] as const;
export const DAY_TYPE_GRAINS = ["weekday", "dayclass", "daypart", "weekday-daypart"] as const;
export type DayTypeGrain = (typeof DAY_TYPE_GRAINS)[number];

/** The design §5 verdict for one cell: admitted at/over the gate,
 * the honesty state below it. */
export type DayTypeCellState = "admitted" | "adjustment_unavailable";

export interface DayTypeCellMeasurement {
  grain: DayTypeGrain;
  cell: string;
  /** Independent samples: distinct Chicago dates in the cell. */
  samples: number;
  /** Raw readings behind the samples — context, never the gate unit. */
  readings: number;
  state: DayTypeCellState;
}

export interface DayTypePoolWindowMeasurement {
  /** Folded series key (the rename join's output). */
  pool: string;
  /** Raw store service ids folded into `pool`. */
  serviceIds: string[];
  /** Canonical window name (FORECAST_WINDOW_METRICS key). */
  window: string;
  readings: number;
  /** Distinct Chicago dates with any reading in this pool/window. */
  days: number;
  /** The full grain ladder, every cell present (0 samples is a
   * measurement, not an omission). */
  cells: DayTypeCellMeasurement[];
}

export interface DayTypeSufficiencyMeasurement {
  gate: number;
  timeZone: string;
  store: {
    records: number;
    /** Percent readings with a parseable ts (the measured universe). */
    pctRecords: number;
    fromIso: string | null;
    toIso: string | null;
  };
  poolWindows: DayTypePoolWindowMeasurement[];
}

const partsFormatter = new Intl.DateTimeFormat("en-US", {
  timeZone: DAY_TYPE_TIME_ZONE,
  weekday: "short",
  year: "numeric",
  month: "2-digit",
  day: "2-digit",
  hour: "2-digit",
  hourCycle: "h23",
});

/** The Chicago calendar cell a reading falls in: its local date,
 * weekday, and daypart. */
export function dayTypeParts(tsMs: number): { date: string; weekday: string; daypart: string } {
  const parts = partsFormatter.formatToParts(new Date(tsMs));
  const get = (type: string) => parts.find((p) => p.type === type)?.value ?? "";
  const hour = Number(get("hour")) % 24;
  const daypart = hour < 6 ? "night" : hour < 12 ? "morning" : hour < 18 ? "afternoon" : "evening";
  return { date: `${get("year")}-${get("month")}-${get("day")}`, weekday: get("weekday"), daypart };
}

function cellNames(grain: DayTypeGrain): string[] {
  switch (grain) {
    case "weekday":
      return [...DAY_TYPE_WEEKDAYS];
    case "dayclass":
      return [...DAY_TYPE_DAYCLASSES];
    case "daypart":
      return [...DAY_TYPE_DAYPARTS];
    case "weekday-daypart":
      return DAY_TYPE_WEEKDAYS.flatMap((w) => DAY_TYPE_DAYPARTS.map((d) => `${w} ${d}`));
  }
}

function cellsFor(parts: { weekday: string; daypart: string }): Record<DayTypeGrain, string> {
  const dayclass = parts.weekday === "Sat" || parts.weekday === "Sun" ? "weekend" : "weekday";
  return {
    weekday: parts.weekday,
    dayclass,
    daypart: parts.daypart,
    "weekday-daypart": `${parts.weekday} ${parts.daypart}`,
  };
}

/**
 * Measure day-type sample sufficiency over snapshot records.
 * `asOfMs` (optional) excludes readings after that instant, so a
 * run over a fixed store copy is reproducible (the backtest
 * script's --as-of discipline).
 */
export function measureDayTypeSufficiency(
  records: readonly SnapshotRecord[],
  opts: { asOfMs?: number } = {},
): DayTypeSufficiencyMeasurement {
  const { index, joinedIds } = indexPercentRecords(records);
  let pctRecords = 0;
  let fromMs: number | null = null;
  let toMs: number | null = null;
  const poolWindows: DayTypePoolWindowMeasurement[] = [];

  for (const pool of [...index.keys()].sort()) {
    const series = index.get(pool) as SnapshotRecord[];
    for (const window of FORECAST_WINDOWS) {
      const aliases = FORECAST_WINDOW_METRICS[window] as string[];
      const metrics = new Set(aliases);
      const matched = series.filter(
        (r) => metrics.has(r.metricId) || aliases.some((a) => r.metricId.endsWith(`:${a}`)),
      );
      if (!matched.length) continue;

      const daysByCell = new Map<string, Set<string>>();
      const readingsByCell = new Map<string, number>();
      for (const grain of DAY_TYPE_GRAINS) {
        for (const cell of cellNames(grain)) {
          daysByCell.set(`${grain}:${cell}`, new Set());
          readingsByCell.set(`${grain}:${cell}`, 0);
        }
      }
      const allDays = new Set<string>();
      let readings = 0;
      for (const r of matched) {
        const ts = Date.parse(r.ts);
        if (!Number.isFinite(ts)) continue;
        if (opts.asOfMs !== undefined && ts > opts.asOfMs) continue;
        readings++;
        const parts = dayTypeParts(ts);
        allDays.add(parts.date);
        const cells = cellsFor(parts);
        for (const grain of DAY_TYPE_GRAINS) {
          const key = `${grain}:${cells[grain]}`;
          (daysByCell.get(key) as Set<string>).add(parts.date);
          readingsByCell.set(key, (readingsByCell.get(key) as number) + 1);
        }
      }
      if (!readings) continue;

      const cells: DayTypeCellMeasurement[] = [];
      for (const grain of DAY_TYPE_GRAINS) {
        for (const cell of cellNames(grain)) {
          const key = `${grain}:${cell}`;
          const samples = (daysByCell.get(key) as Set<string>).size;
          cells.push({
            grain,
            cell,
            samples,
            readings: readingsByCell.get(key) as number,
            state: samples >= DAY_TYPE_SAMPLE_GATE ? "admitted" : "adjustment_unavailable",
          });
        }
      }
      poolWindows.push({
        pool,
        serviceIds: joinedIds.get(pool) ?? [pool],
        window,
        readings,
        days: allDays.size,
        cells,
      });
    }
  }

  for (const r of records) {
    if (typeof r.value !== "number" || r.unit !== "pct") continue;
    const ts = Date.parse(r.ts);
    if (!Number.isFinite(ts)) continue;
    if (opts.asOfMs !== undefined && ts > opts.asOfMs) continue;
    pctRecords++;
    if (fromMs === null || ts < fromMs) fromMs = ts;
    if (toMs === null || ts > toMs) toMs = ts;
  }

  return {
    gate: DAY_TYPE_SAMPLE_GATE,
    timeZone: DAY_TYPE_TIME_ZONE,
    store: {
      records: records.length,
      pctRecords,
      fromIso: fromMs === null ? null : new Date(fromMs).toISOString(),
      toIso: toMs === null ? null : new Date(toMs).toISOString(),
    },
    poolWindows,
  };
}

/** The human report: gate + basis, per-grain totals, the admitted
 * cells (the decision-relevant set), and one summary row per
 * pool/window. Full per-cell detail rides in the JSON form. */
export function renderDayTypeSufficiencyReport(m: DayTypeSufficiencyMeasurement): string {
  const lines: string[] = [];
  lines.push("# Day-type sample sufficiency — C4.3 re-measure (#13259)");
  lines.push("");
  lines.push(
    `Gate: **≥${m.gate} independent samples** per cell (C4 design §5). A sample is one ` +
      `${m.timeZone} calendar date with ≥1 reading in the cell; cells are ${m.timeZone} ` +
      `calendar cells. Below the gate the state is \`adjustment_unavailable\` and the ` +
      `unadjusted forecast stands.`,
  );
  lines.push(
    `Store: ${m.store.records} records (${m.store.pctRecords} percent readings), ` +
      `${m.store.fromIso ?? "?"} → ${m.store.toIso ?? "?"}. Pools are folded series keys ` +
      `(the S2 rename join); windows are the canonical forecast windows.`,
  );
  lines.push("");

  lines.push("## By grain");
  lines.push("");
  lines.push("| grain | cells | admitted | adjustment_unavailable | max samples |");
  lines.push("| --- | ---: | ---: | ---: | ---: |");
  for (const grain of DAY_TYPE_GRAINS) {
    const cells = m.poolWindows.flatMap((pw) => pw.cells.filter((c) => c.grain === grain));
    const admitted = cells.filter((c) => c.state === "admitted").length;
    const max = cells.reduce((acc, c) => Math.max(acc, c.samples), 0);
    lines.push(`| ${grain} | ${cells.length} | ${admitted} | ${cells.length - admitted} | ${max} |`);
  }
  lines.push("");

  lines.push("## Admitted cells");
  lines.push("");
  const admittedRows: string[] = [];
  for (const pw of m.poolWindows) {
    for (const c of pw.cells) {
      if (c.state === "admitted") {
        admittedRows.push(`| ${pw.pool} | ${pw.window} | ${c.grain} | ${c.cell} | ${c.samples} |`);
      }
    }
  }
  if (admittedRows.length) {
    lines.push("| pool | window | grain | cell | samples |");
    lines.push("| --- | --- | --- | --- | ---: |");
    lines.push(...admittedRows);
  } else {
    lines.push("None — every cell is under the gate; every adjustment stays unavailable.");
  }
  lines.push("");

  lines.push("## Per pool/window");
  lines.push("");
  lines.push(
    "| pool | window | readings | days | weekday adm/max | dayclass adm/max | daypart adm/max | weekday-daypart adm/max |",
  );
  lines.push("| --- | --- | ---: | ---: | --- | --- | --- | --- |");
  for (const pw of m.poolWindows) {
    const per = (grain: DayTypeGrain) => {
      const cells = pw.cells.filter((c) => c.grain === grain);
      const admitted = cells.filter((c) => c.state === "admitted").length;
      const max = cells.reduce((acc, c) => Math.max(acc, c.samples), 0);
      return `${admitted}/${cells.length} · ${max}`;
    };
    lines.push(
      `| ${pw.pool} | ${pw.window} | ${pw.readings} | ${pw.days} | ${per("weekday")} | ` +
        `${per("dayclass")} | ${per("daypart")} | ${per("weekday-daypart")} |`,
    );
  }
  lines.push("");
  return lines.join("\n");
}
