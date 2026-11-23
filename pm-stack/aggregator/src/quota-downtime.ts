// C4.1 — complete-downtime window predictions (#13257, component
// of the C4 quota-forecasting epic #9572). The record format,
// storage, and scoring contract are the C4 design:
// doc/2026-11-13T15-12-00Z_design_reporting-architecture-event-driven/
// c4-forecasting-design.md (§1 the prediction record, §7 the
// downtime kind), landed by PR #13268. The input product is the
// Stage-3 forecast (quota-forecast.json, #12695).
//
// Derivation (§7): from the per-pool exhaustion projections +
// reset times — the interval where every routing pool's predicted
// wall has passed and the earliest reopening (any pool's reset)
// has not yet arrived. A pool window is down over [wall, reset):
// from its projected wall (or the tick itself, when the window is
// already walled) until its reset. A provider is down while any
// of its pool rows' windows is down (any binding limit blocks
// it), so a provider's down-set is the union of its windows'
// intervals; a complete-downtime window is a maximal interval in
// the intersection of every provider's down-set. Sets are the
// routing pools only — the forecast product's coverage — keyed by
// canonical provider id (the S1 keying lesson).
//
// The prediction range (§1): earliest is the all-walled
// interval's start (the last member wall), latest its end (the
// earliest member reset — the first reopening). earliest ==
// latest is refused by construction (a zero-width overlap is no
// window), and an interval no member reset bounds is not a
// prediction at all: it publishes as an honesty state, because a
// model that cannot bound its answer emits one (§1).
//
// Honesty (§1, and #13257 criterion 3): insufficient_history is a
// published state with a named reason — never an invented range,
// never a silent absence. A provider whose windows the forecast
// could not project can only add downtime, never remove a known
// overlap, so a known overlap still publishes as predicted (the
// unknown providers named in the basis detail); but unknowns veto
// a no-window verdict, because absence of a record is the
// no-prediction signal the scorer reads (§3: a realized downtime
// with no prediction is a miss). A tick where every provider is
// projectable and no overlap exists therefore publishes NO
// record — that absence is the defensible "no window" claim.
//
// Basis (§1: mandatory, singular): since_last_reset — the S1
// projections are fit on the current reset period's same-period
// series, and this module fits nothing itself; it intersects the
// oracle's published intervals. Methodology version c4-meth:3
// (c4-methodology-log.jsonl beside this module's package root,
// §6). Advisory only: no placement path reads these records —
// the C6 pages and forecast alerts named by #9572 consume them.

import type { ForecastLedgerRow, QuotaForecastFile } from "./quota-forecast.ts";

/** Turso ledger source for C4 prediction records (design §1):
 * distinct from S1's "quota-forecast" — C4 records never mix
 * into the S1 source. */
export const PREDICTION_LEDGER_SOURCE = "quota-prediction";

export const C4_METHODOLOGY_VERSION = "c4-meth:3" as const;

export interface C4PredictionRecord {
  record_version: 1;
  id: string;
  kind: "downtime_window";
  subject: { pools: string[] };
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

interface WallInterval {
  start: number;
  /** ms epoch; Infinity when the pool reports no reset. */
  end: number;
}

// --- Range rendering (design §4) ----------------------------------
// One formatter for (earliest, latest, issued_at), in
// America/Chicago wall-clock terms. Duration form under 48h,
// day form at/over 48h, calendar form when the range crosses a
// Chicago day boundary — the form preferred for downtime windows
// and any range whose latest bound falls on a different day than
// its earliest. A calendar bound a week or more from issuance
// carries its month/day date (a bare weekday name repeats every
// seven days — #13536); this/tomorrow/today-anchored bounds keep
// their short form.

const CHICAGO = "America/Chicago";
const dayKeyFmt = new Intl.DateTimeFormat("en-US", { timeZone: CHICAGO, year: "numeric", month: "2-digit", day: "2-digit" });
const weekdayFmt = new Intl.DateTimeFormat("en-US", { timeZone: CHICAGO, weekday: "long" });
const monthDayFmt = new Intl.DateTimeFormat("en-US", { timeZone: CHICAGO, month: "short", day: "numeric" });
const hourFmt = new Intl.DateTimeFormat("en-US", { timeZone: CHICAGO, hour: "numeric", hourCycle: "h23" });

function chicagoDayRef(ms: number, issuedMs: number): string {
  const day = dayKeyFmt.format(new Date(ms));
  if (day === dayKeyFmt.format(new Date(issuedMs))) return "this";
  // Tomorrow in Chicago terms: the issued instant + 24h shares the
  // next Chicago calendar day in all but pathological DST cases;
  // the weekday name is the fallback and is never wrong.
  if (day === dayKeyFmt.format(new Date(issuedMs + 24 * 3_600_000))) return "tomorrow";
  return weekdayFmt.format(new Date(ms));
}

// A bare weekday name repeats every seven days, so a bound a
// week or more from issuance carries its calendar date after the
// daypart — otherwise a range like Sep 22 → Oct 6 renders as the
// ambiguous "between Tuesday afternoon and Tuesday
// afternoon" (#13536). Bounds anchored to this/tomorrow/today
// are never ambiguous and keep their short form.
function chicagoDateSuffix(ms: number, issuedMs: number): string {
  const ref = chicagoDayRef(ms, issuedMs);
  if (ref === "this" || ref === "tomorrow") return "";
  if (Math.abs(ms - issuedMs) >= 7 * 86_400_000) return `, ${monthDayFmt.format(new Date(ms))}`;
  return "";
}

function chicagoDaypart(ms: number, issuedMs: number): string {
  const ref = chicagoDayRef(ms, issuedMs);
  const suffix = chicagoDateSuffix(ms, issuedMs);
  const hour = Number(hourFmt.format(new Date(ms)));
  const namedHour = hour === 0 ? "12 am" : hour < 12 ? `${hour} am` : hour === 12 ? "12 pm" : `${hour - 12} pm`;
  if (hour >= 5 && hour < 12) return ref === "this" ? "this morning" : `${ref} morning${suffix}`;
  if (hour >= 12 && hour < 17) return ref === "this" ? "this afternoon" : `${ref} afternoon${suffix}`;
  if (hour >= 17 && hour < 22) return ref === "this" ? "this evening" : `${ref} evening${suffix}`;
  // Night bounds are sharp in Drew's formats ("Sunday 2 am").
  if (ref === "tomorrow") return `tomorrow ${namedHour}`;
  if (ref === "this") return `today ${namedHour}`;
  return `${ref} ${namedHour}${suffix}`;
}

const roundHalf = (x: number) => Math.round(x * 2) / 2;
const fmtNum = (x: number) => (Number.isInteger(x) ? String(x) : x.toFixed(1));

/** The stored `rendered` string for a prediction range (§4):
 * what was shown is exactly what is scored, so it is produced
 * once, at issuance, by this formatter. */
export function renderPredictionRange(earliestISO: string, latestISO: string, issuedAtISO: string): string {
  const earliest = Date.parse(earliestISO);
  const latest = Date.parse(latestISO);
  const issued = Date.parse(issuedAtISO);
  if (dayKeyFmt.format(new Date(earliest)) !== dayKeyFmt.format(new Date(latest))) {
    return `between ${chicagoDaypart(earliest, issued)} and ${chicagoDaypart(latest, issued)}`;
  }
  const spanH = (latest - earliest) / 3_600_000;
  if (spanH >= 48) {
    const roundTenth = (x: number) => Math.round(x * 10) / 10;
    return `${fmtNum(roundTenth((earliest - issued) / 86_400_000))} to ${fmtNum(
      roundTenth((latest - issued) / 86_400_000),
    )} days`;
  }
  if (earliest <= issued) {
    // Already in progress: "from now" cannot express a start in
    // the past, so the same-day calendar form carries it.
    return `between ${chicagoDaypart(earliest, issued)} and ${chicagoDaypart(latest, issued)}`;
  }
  const fromH = (t: number) => roundHalf((t - issued) / 3_600_000);
  return `between ${fmtNum(fromH(earliest))} and ${fmtNum(fromH(latest))} hours from now`;
}

// --- Derivation ----------------------------------------------------

interface ProviderDown {
  providerId: string;
  intervals: WallInterval[];
  unknown: boolean;
  samples: number;
  measuredFrom: string | null;
}

/**
 * Derive one tick's downtime_window records from the tick's
 * forecast product. Pure: no I/O, no clock reads — the tick time
 * is the forecast's own generatedAt. `previous` maps a subject
 * key (the pool set, joined) to the last record id issued for
 * it, for the supersedes link (§1); pass an empty map when the
 * caller has no history.
 *
 * Returns [] when the defensible verdict is "no complete
 * downtime in the horizon" — absence is the no-prediction
 * signal, per the design's scoring rules.
 */
export function deriveDowntimeWindowRecords(
  forecast: QuotaForecastFile,
  previous: ReadonlyMap<string, string> = new Map(),
): C4PredictionRecord[] {
  const issuedAt = forecast.generatedAt;
  const issuedMs = Date.parse(issuedAt);
  const providerIds = Object.keys(forecast.pools).sort();
  const subjectKey = providerIds.join("+");

  const providers: ProviderDown[] = [];
  for (const providerId of providerIds) {
    const intervals: WallInterval[] = [];
    let unknown = false;
    let samples = 0;
    let measuredFrom: string | null = null;
    for (const pool of forecast.pools[providerId]) {
      if (pool.windows.length === 0) unknown = true;
      for (const w of pool.windows) {
        if (measuredFrom === null || w.measuredAt < measuredFrom) measuredFrom = w.measuredAt;
        if (w.state === "insufficient-history") {
          unknown = true;
          continue;
        }
        samples += w.samples;
        const resetMs = w.resetsAt ? Date.parse(w.resetsAt) : NaN;
        const end = Number.isFinite(resetMs) ? resetMs : Infinity;
        if (w.state === "walled") {
          intervals.push({ start: issuedMs, end });
        } else if (w.state === "projected" && w.projectedWallAt != null && w.wallsBeforeReset !== false) {
          intervals.push({ start: w.projectedWallAt, end });
        }
      }
    }
    providers.push({ providerId, intervals, unknown, samples, measuredFrom });
  }

  const totalSamples = providers.reduce((n, p) => n + p.samples, 0);
  const spanFrom = providers.reduce<string | null>(
    (acc, p) => (p.measuredFrom !== null && (acc === null || p.measuredFrom < acc) ? p.measuredFrom : acc),
    null,
  );
  const unknownIds = providers.filter((p) => p.unknown).map((p) => p.providerId);

  const record = (over: Partial<C4PredictionRecord> & Pick<C4PredictionRecord, "id" | "state">): C4PredictionRecord => ({
    record_version: 1,
    kind: "downtime_window",
    subject: { pools: providerIds },
    issued_at: issuedAt,
    basis: {
      name: "since_last_reset",
      detail:
        "complete-downtime interval intersected from the Stage-3 per-pool pace projections (quota-forecast.json, #12695): " +
        "every routing pool's predicted wall has passed and the earliest reopening (any pool's reset) has not yet arrived",
    },
    inputs: { series: "quota-forecast.json", samples: totalSamples, span: { from: spanFrom, to: issuedAt }, adjustments: [] },
    prediction: null,
    methodology_version: C4_METHODOLOGY_VERSION,
    state_reason: null,
    supersedes: previous.get(subjectKey) ?? null,
    ...over,
  });

  const honesty = (reason: string): C4PredictionRecord[] => [
    record({
      id: `c4:downtime_window:${subjectKey}@honesty:${issuedMs}`,
      state: "insufficient_history",
      state_reason: reason,
    }),
  ];

  if (providerIds.length === 0) {
    return honesty("the forecast product carries no pools; no downtime verdict is defensible");
  }

  // Maximal intervals where every provider has a covering
  // interval: sweep the sorted boundary points; coverage is
  // constant on each segment between consecutive points.
  const covers = (intervals: WallInterval[], t: number) => intervals.some((iv) => iv.start <= t && t < iv.end);
  const boundarySet = new Set<number>([issuedMs]);
  for (const p of providers) {
    for (const iv of p.intervals) {
      boundarySet.add(iv.start);
      if (Number.isFinite(iv.end)) boundarySet.add(iv.end);
    }
  }
  const points = [...boundarySet].sort((a, b) => a - b);
  const overlaps: { start: number; end: number }[] = [];
  let runStart: number | null = null;
  for (let i = 0; i < points.length; i++) {
    const t = points[i];
    const allDown = providers.every((p) => covers(p.intervals, t));
    if (allDown) {
      if (runStart === null) runStart = t;
      if (i === points.length - 1) {
        const end = Math.min(
          ...providers.map((p) => Math.min(...p.intervals.filter((iv) => iv.start <= t && t < iv.end).map((iv) => iv.end))),
        );
        if (end > runStart) overlaps.push({ start: runStart, end });
        runStart = null;
      }
    } else if (runStart !== null) {
      if (t > runStart) overlaps.push({ start: runStart, end: t });
      runStart = null;
    }
  }

  const bounded = overlaps.filter((o) => Number.isFinite(o.end));
  if (bounded.length > 0) {
    return bounded.map((o) => {
      const earliest = new Date(o.start).toISOString();
      const latest = new Date(o.end).toISOString();
      return record({
        id: `c4:downtime_window:${subjectKey}@${o.start}:${issuedMs}`,
        state: "predicted",
        basis: {
          name: "since_last_reset",
          detail:
            "complete-downtime interval intersected from the Stage-3 per-pool pace projections (quota-forecast.json, #12695): " +
            "every routing pool's predicted wall has passed and the earliest reopening (any pool's reset) has not yet arrived" +
            (unknownIds.length > 0
              ? `; pools the forecast could not project (${unknownIds.join(", ")}) can only add downtime and do not affect this known overlap`
              : ""),
        },
        prediction: { earliest, latest, rendered: renderPredictionRange(earliest, latest, issuedAt) },
      });
    });
  }
  if (overlaps.length > 0) {
    return honesty(
      "every pool is projected walled with no reported reset in common; the all-walled interval cannot be bounded, and an unbounded answer is not a prediction",
    );
  }
  if (unknownIds.length > 0) {
    return honesty(
      `no complete-downtime window is derivable and the forecast could not project: ${unknownIds.join(", ")} — absence of a window is not asserted`,
    );
  }
  return [];
}

/**
 * The ledger mirror (design §1): one row per record under the
 * C4 source "quota-prediction", beside the S1 forecast rows in
 * agent_quota_records, payload = the record verbatim so the
 * accuracy scoring (§3) joins predictions to outcomes in one
 * place. resets carries the predicted window end (the earliest
 * reopening), mirroring the forecast rows' resets convention;
 * null on honesty records, which publish no range.
 */
export function buildDowntimeLedgerRows(records: readonly C4PredictionRecord[], tickAt: string): ForecastLedgerRow[] {
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
