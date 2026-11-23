// Stage-3 anomaly detectors (#12699, slice S5 of epic #12694;
// design doc/2026-11-22T22-35-00Z_design_quota-plane-stage-3 §6,
// decision D6): three detectors running aggregator-side beside the
// gatherer tick, in the quota-watchdog shape — own state file, own
// history JSONL, fail-soft — with every finding landing in the two
// places findings already go: an evidence-baseline diff record (the
// history JSONL: what changed against the recorded baseline, stated
// in the baseline's terms) and the alerts ledger (an incident via
// AlertsStore.postIncident, the C3 record of detail per #12599).
// No new plumbing: the tick hook in scripts/gather-quota.ts is the
// only live caller, and scripts/quota-anomaly-replay.ts replays the
// same machines over the snapshot store for the acceptance record.
//
// The detectors:
// - burn-spike: realized burn over the last hour exceeds the pool's
//   own forecast band — the S1 forecast rate at the origin plus the
//   residual spread S2's method measures for that pool/window
//   (realized minus forecast burn, replayed over the series; the
//   band is the p99 of those residuals, floored at the forecast
//   rate itself; residuals from origins judged anomalous never
//   join the dispersion, so spikes cannot ratchet their own band).
//   A spike is defined against
//   the pool's own history, never a global threshold, so a pool
//   that always burns hot does not cry wolf — and the known v1
//   model bias (S2: weekly windows run systematically late under
//   linear pace, i.e. realized burn sits above the forecast rate by
//   a steady margin) is INSIDE the band by construction, because
//   the residuals that set the band carry that same bias. A spike
//   is burn beyond even the pool's own biased norm.
// - schema-drift: a provider response whose parsed shape changes —
//   fields appearing/disappearing against the recorded shape, or a
//   parse falling into the fail-closed unparseable state. The
//   gatherer's adapters already fail closed (previous value ages,
//   never zeros); this detector makes the drift itself a finding at
//   first occurrence, not after the aged value goes stale.
// - plan-change: a window's limit (the snapshot records'
//   denominator) changes between consecutive records for the same
//   service+metric, or a canonical window kind appears/disappears
//   for a pool. A changed denominator breaks pace continuity (the
//   admission module already treats it as breaking the series);
//   this surfaces the break the tick it happens.
//
// Honesty rules (mirroring quota-analytics.ts / S1 / S2):
// - A detector that cannot evaluate says so by name: every series
//   evaluated yields an evaluation row whose state is "fired",
//   "silent", "unevaluable" (with the reason), or
//   "baseline-established". Absent data is never an all-clear — a
//   series with no readings gets no row at all, and a series whose
//   band is unmeasured is "unevaluable", never "silent".
// - Readings already at/over 100% are detections, not predictions
//   (S2's rule): they are never burn-spike origins.
// - A null limit is the absence of a denominator, never a change:
//   the plan-change limit leg fires only between two recorded
//   non-null limits.
// - Losing the state file re-baselines silently (baseline-
//   established rows, no findings): a state loss must never become
//   an alert storm of rediscovered history.
//
// Pure functions over plain values — no I/O, no clock reads except
// through arguments. The tick and the replay CLI do the file work.
import {
  PROJECTION_PERIOD_TOLERANCE_MS,
  projectWindow,
  type PctPoint,
} from "./quota-analytics.ts";
import {
  FORECAST_POOLS,
  FORECAST_WINDOW_METRICS,
  forecastAccuracySeriesKey,
  indexPercentRecords,
  pointsForWindow,
  type ForecastPoolDef,
} from "./quota-forecast-accuracy.ts";
import type { QuotaCurrentEntry, SnapshotRecord } from "./model.ts";

export type AnomalyDetectorId = "burn-spike" | "schema-drift" | "plan-change";

/** One finding: the evidence-baseline diff record. `baseline` and
 * `observed` state the diff in the baseline's terms — what the
 * recorded baseline said, and what was observed instead. */
export interface AnomalyFinding {
  detector: AnomalyDetectorId;
  /** Transition-unique identity: the tick dedupes re-posts on it
   * (state firedKeys) and it is the alerts-ledger incident key, so
   * each distinct transition is its own ledger entry with its own
   * opened-event detail, while a re-post of the same finding bumps
   * instead of duplicating. */
  key: string;
  /** ISO instant the finding is about (the firing observation). */
  at: string;
  /** Service identity in the spelling of the surface that observed
   * it: the store/aggregator serviceId (pool serviceId for the
   * pool-scoped legs). */
  serviceId: string;
  providerId: string | null;
  /** Canonical window (FORECAST_WINDOW_METRICS spelling) when the
   * finding is about one window; null otherwise. */
  window: string | null;
  severity: "warning";
  title: string;
  baseline: string;
  observed: string;
  detail: Record<string, unknown>;
}

export type DetectorEvaluationState = "fired" | "silent" | "unevaluable" | "baseline-established";

/** The per-series verdict row: how a reader tells a real all-clear
 * ("silent" over a measured band / a recorded shape) from an
 * absence of evidence ("unevaluable", reason named). */
export interface DetectorEvaluation {
  detector: AnomalyDetectorId;
  serviceId: string;
  window: string | null;
  state: DetectorEvaluationState;
  reason?: string;
  findings: number;
}

// ---------------------------------------------------------------------
// Shared: canonical windows and quantiles
// ---------------------------------------------------------------------

/** The canonical Stage-3 window a snapshot metricId (or a window
 * name in the same vocabulary) belongs to, via S2's window/metric
 * mapping — so a rename of the same window (7-day -> weekly,
 * 5-hour -> 5h) is the same canonical window here, exactly as the
 * forecast and the backtest already treat it. Null when the metric
 * is not a quota window at all (balances, counters, estimates). */
export function canonicalWindowForMetric(metricId: string): string | null {
  for (const [window, aliases] of Object.entries(FORECAST_WINDOW_METRICS)) {
    if (aliases.includes(metricId)) return window;
    if (aliases.some((a) => metricId.endsWith(`:${a}`))) return window;
  }
  return null;
}

function quantile(sortedAsc: readonly number[], q: number): number {
  if (sortedAsc.length === 1) return sortedAsc[0] as number;
  const pos = (sortedAsc.length - 1) * q;
  const base = Math.floor(pos);
  const rest = pos - base;
  const next = sortedAsc[base + 1];
  return next !== undefined
    ? (sortedAsc[base] as number) + rest * (next - (sortedAsc[base] as number))
    : (sortedAsc[base] as number);
}

const round1 = (n: number): number => Math.round(n * 10) / 10;

// ---------------------------------------------------------------------
// The field-set machine: one recorded shape, diffed per observation.
// Shared by schema-drift (a service's parsed shape) and the
// plan-change window-kind leg (a pool's canonical window set) —
// the two differ in vocabulary and sustain, not in mechanics.
// ---------------------------------------------------------------------

export interface FieldSetState {
  /** The recorded baseline field set, sorted. */
  fields: string[];
  /** Field -> consecutive observations it has been missing from;
   * only fields currently counted appear. */
  absent: Record<string, number>;
  /** A parse is currently in the fail-closed unparseable state
   * (the episode's finding already fired). */
  unparseableActive: boolean;
  /** Whether any parseable observation has established a baseline.
   * False after an unparseable first contact: the next parseable
   * observation establishes the baseline, it does not "add" every
   * field as drift. */
  hasBaseline: boolean;
  /** When the current baseline was established (ISO). */
  baselineAt: string;
}

export function emptyFieldSetState(atIso: string): FieldSetState {
  return { fields: [], absent: {}, unparseableActive: false, hasBaseline: false, baselineAt: atIso };
}

export interface FieldSetObservation {
  ts: string;
  /** The observed field set; null when this observation is a parse
   * that fell closed (see unparseable). */
  fields: string[] | null;
  /** Why the parse fell closed, when fields is null. */
  unparseable?: string;
}

export type FieldSetEvent =
  | { kind: "baseline-established"; fields: string[] }
  | { kind: "shape-changed"; added: string[]; removed: string[]; fields: string[] }
  | { kind: "unparseable-started"; reason: string }
  | { kind: "unevaluable"; reason: string };

/**
 * Diff one observation against the recorded shape.
 *
 * Additions fire at first occurrence (a field appearing is
 * unambiguous in any complete observation). Removals fire only
 * after `removalConfirm` consecutive observations missing the
 * field: a live tick's parse is complete every 15 minutes (confirm
 * 1 for response shapes; pool window sets use 2 — a single window
 * can drop out of one tick's parse on a rejected value), while the
 * snapshot store appends only when a value MOVES, so a replay
 * day-set missing a field may mean stasis, not disappearance
 * (confirm 3 active days). The sustain is a property of the
 * observation stream's completeness, stated at each call site.
 */
export function observeFieldSet(
  prior: FieldSetState | undefined,
  obs: FieldSetObservation,
  removalConfirm: number,
): { state: FieldSetState; events: FieldSetEvent[] } {
  const events: FieldSetEvent[] = [];
  if (obs.fields === null) {
    const state: FieldSetState = prior
      ? { ...prior, unparseableActive: true }
      : { ...emptyFieldSetState(obs.ts), unparseableActive: true };
    if (!prior?.unparseableActive) {
      if (prior?.hasBaseline) {
        events.push({ kind: "unparseable-started", reason: obs.unparseable ?? "parse fell closed" });
      } else {
        events.push({
          kind: "unevaluable",
          reason: "first observation is an unparseable parse — no baseline shape recorded yet",
        });
      }
    }
    return { state, events };
  }
  const fields = [...new Set(obs.fields)].sort();
  if (!prior || !prior.hasBaseline) {
    events.push({ kind: "baseline-established", fields });
    return {
      state: { fields, absent: {}, unparseableActive: false, hasBaseline: true, baselineAt: obs.ts },
      events,
    };
  }
  const recorded = new Set(prior.fields);
  const observed = new Set(fields);
  const added = fields.filter((f) => !recorded.has(f));
  const absent: Record<string, number> = {};
  const removed: string[] = [];
  for (const f of prior.fields) {
    if (observed.has(f)) continue;
    const count = (prior.absent[f] ?? 0) + 1;
    if (count >= removalConfirm) removed.push(f);
    else absent[f] = count;
  }
  const nextFields = prior.fields.filter((f) => !removed.includes(f)).concat(added).sort();
  if (added.length || removed.length) {
    events.push({ kind: "shape-changed", added, removed, fields: nextFields });
  }
  return {
    state: { fields: nextFields, absent, unparseableActive: false, hasBaseline: true, baselineAt: prior.baselineAt },
    events,
  };
}

// ---------------------------------------------------------------------
// Detector (a): burn spike
// ---------------------------------------------------------------------

/** Matured forecast residuals a pool/window must have before its
 * band is measured; below this the series is unevaluable, never
 * silently "in band". The count and the span are both required:
 * S2's baselines are store-scale measurements, and a band fit on
 * the first hours of a series is a cold-start artifact, not a
 * measurement (the replay's first-day marginal fires were exactly
 * that). */
export const BURN_SPIKE_MIN_RESIDUALS = 24;
/** The matured residuals must also span at least this much series
 * time (see BURN_SPIKE_MIN_RESIDUALS). */
export const BURN_SPIKE_MIN_RESIDUAL_SPAN_MS = 12 * 3_600_000;
/** A spike must clear the band by this margin, not merely cross
 * it — the §4 idiom (a forecast acts only "at least the
 * configured margin" past its threshold). Without it, a pool
 * whose normal burn sits at its own p99 fires on every heavy-tail
 * hour (the replay's ratio-1.02 squeakers). */
export const BURN_SPIKE_BAND_MARGIN = 1.25;
/** One finding per spike episode: after an exceedance, the
 * detector re-arms only after the burn has stayed inside the band
 * for this long — a sustained heavy-burn regime is ONE episode,
 * however the interleaved sub-series points flicker around the
 * band in the meantime. */
export const BURN_SPIKE_EPISODE_REARM_MS = 2 * 3_600_000;
/** The band is the forecast rate plus this quantile of the pool's
 * own realized residuals (the S2-measured spread). p99, not p90:
 * a band at the p90 would be exceeded by ~10% of origins in steady
 * state by construction; at the p99 an exceedance is a regime
 * change, and the episode dedupe absorbs the residual tail. */
export const BURN_SPIKE_RESIDUAL_QUANTILE = 0.99;
/** Materiality floor: an excess over the forecast smaller than
 * this (pts/hour) is pace noise on near-zero pools, not a spike. */
export const BURN_SPIKE_MIN_EXCESS_PTS_PER_HOUR = 1;
/** The realized point for an origin is the first reading at/after
 * origin+1h, accepted within this slack (the gatherer cadence is
 * 15 min; the manual era's daily cadence simply yields no realized
 * point, so those origins never evaluate). */
export const BURN_SPIKE_REALIZED_TOLERANCE_MS = 45 * 60_000;

const HOUR_MS = 3_600_000;

interface BurnOrigin {
  ts: number;
  pct: number;
  resetsAt: string | null;
  /** The S1 forecast rate issued at this origin; null when the
   * pace was unmeasurable or the reading was already walled. */
  rate: number | null;
  /** Realized burn over the next hour, pts/hour; null when no
   * same-period reading lands in the realized window. */
  burn: number | null;
}

function sameResets(a: string | null, b: string | null): boolean {
  if (a === null || b === null) return true;
  const x = Date.parse(a);
  const y = Date.parse(b);
  if (!Number.isFinite(x) || !Number.isFinite(y)) return a === b;
  return Math.abs(x - y) <= PROJECTION_PERIOD_TOLERANCE_MS;
}

function burnOrigins(points: readonly PctPoint[]): BurnOrigin[] {
  const pts = [...points].sort((a, b) => a.ts - b.ts);
  return pts.map((p, i) => {
    let rate: number | null = null;
    if (p.pct < 100) {
      const proj = projectWindow({ points: pts, pctNow: p.pct, resetsAt: p.resetsAt, now: p.ts });
      rate = proj.ratePtsPerHour;
    }
    let burn: number | null = null;
    const realized = pts
      .slice(i + 1)
      .find((q) => q.ts >= p.ts + HOUR_MS && q.ts <= p.ts + HOUR_MS + BURN_SPIKE_REALIZED_TOLERANCE_MS);
    if (realized && sameResets(realized.resetsAt, p.resetsAt)) {
      burn = (realized.pct - p.pct) / ((realized.ts - p.ts) / HOUR_MS);
    }
    return { ts: p.ts, pct: p.pct, resetsAt: p.resetsAt, rate, burn };
  });
}

/**
 * Evaluate one pool/window series for burn spikes. Evaluation and
 * residual maturation run over every origin in order;
 * `originSinceMs` gates only firing (the live tick passes now-2h
 * so only fresh spikes alert; the replay passes nothing and
 * scores the whole series). A spike is a realized burn clearing
 * the band by BURN_SPIKE_BAND_MARGIN with at least the materiality
 * excess over the forecast. An episode fires once: the detector
 * re-arms only after the burn has stayed inside the band for
 * BURN_SPIKE_EPISODE_REARM_MS.
 */
export function evaluateBurnSpikes(args: {
  pool: ForecastPoolDef;
  window: string;
  points: readonly PctPoint[];
  originSinceMs?: number;
}): { findings: AnomalyFinding[]; evaluation: DetectorEvaluation } {
  const { pool, window } = args;
  const origins = burnOrigins(args.points);
  const findings: AnomalyFinding[] = [];
  const residuals: number[] = []; // kept sorted ascending
  const insertResidual = (v: number): void => {
    let lo = 0;
    let hi = residuals.length;
    while (lo < hi) {
      const mid = (lo + hi) >> 1;
      if ((residuals[mid] as number) < v) lo = mid + 1;
      else hi = mid;
    }
    residuals.splice(lo, 0, v);
  };
  let armed = true;
  let lastExceedAt = Number.NEGATIVE_INFINITY;
  let evaluableOrigins = 0;
  let originsWithForecast = 0;
  let originsWithRealized = 0;
  let residualSpanMs = 0;
  let firstResidualTs: number | null = null;
  const bandFromResiduals = (rate: number): number =>
    rate + Math.max(quantile(residuals, BURN_SPIKE_RESIDUAL_QUANTILE), 0);
  const bandMeasurable = (): boolean =>
    residuals.length >= BURN_SPIKE_MIN_RESIDUALS && residualSpanMs >= BURN_SPIKE_MIN_RESIDUAL_SPAN_MS;
  // Residuals mature monotonically (origins are ts-sorted): a
  // predecessor's residual joins the band only once its realized
  // point exists, i.e. at origins at/after prev.ts + 1h + slack.
  // At maturation the predecessor is judged against the band its
  // own predecessors formed: a residual from an origin that itself
  // cleared the spike threshold is trimmed — the band measures
  // NORMAL dispersion, and letting spike residuals in ratchets
  // the band up behind each spike until only a bigger spike can
  // fire. (An origin too early to be judged — band not yet
  // measurable — is admitted; the margin below carries those.)
  // Evaluation runs over every origin, in order; originSinceMs
  // gates only FIRING.
  let maturedUpTo = 0;
  for (const o of origins) {
    while (
      maturedUpTo < origins.length &&
      (origins[maturedUpTo] as BurnOrigin).ts + HOUR_MS + BURN_SPIKE_REALIZED_TOLERANCE_MS <= o.ts
    ) {
      const prev = origins[maturedUpTo] as BurnOrigin;
      if (prev !== o && prev.rate !== null && prev.burn !== null) {
        const trimmed =
          bandMeasurable() &&
          prev.burn > bandFromResiduals(prev.rate) * BURN_SPIKE_BAND_MARGIN &&
          prev.burn - prev.rate >= BURN_SPIKE_MIN_EXCESS_PTS_PER_HOUR;
        if (!trimmed) {
          insertResidual(prev.burn - prev.rate);
          if (firstResidualTs === null) firstResidualTs = prev.ts;
          residualSpanMs = Math.max(residualSpanMs, prev.ts - firstResidualTs);
        }
      }
      maturedUpTo++;
    }
    if (o.rate !== null) originsWithForecast++;
    if (o.burn !== null) originsWithRealized++;
    if (o.rate === null || o.burn === null) continue;
    if (!bandMeasurable()) continue;
    // The spread widens the band; it never narrows it below the
    // forecast itself (a negative p99 means the model over-predicts
    // this pool — conservatism, not a hair trigger).
    const spread = Math.max(quantile(residuals, BURN_SPIKE_RESIDUAL_QUANTILE), 0);
    const band = o.rate + spread;
    const threshold = band * BURN_SPIKE_BAND_MARGIN;
    const exceeds = o.burn > threshold && o.burn - o.rate >= BURN_SPIKE_MIN_EXCESS_PTS_PER_HOUR;
    const firingEligible = args.originSinceMs === undefined || o.ts >= args.originSinceMs;
    if (firingEligible) evaluableOrigins++;
    if (exceeds) {
      lastExceedAt = o.ts;
      if (armed && firingEligible) {
        const at = new Date(o.ts).toISOString();
        findings.push({
          detector: "burn-spike",
          key: `burn-spike:${pool.serviceId}:${window}:${o.resetsAt ?? "no-reset"}:${at.slice(0, 13)}`,
          at,
          serviceId: pool.serviceId,
          providerId: pool.providerId,
          window,
          severity: "warning",
          title: `burn spike: ${pool.serviceId} ${window} burned ${round1(o.burn)} pts/h against a spike threshold of ${round1(threshold)} pts/h`,
          baseline: `forecast ${round1(o.rate)} pts/h + residual p99 ${round1(spread)} pts/h (${residuals.length} realized residuals for this pool/window) = band ${round1(band)} pts/h; spike threshold = band x ${BURN_SPIKE_BAND_MARGIN} = ${round1(threshold)} pts/h`,
          observed: `realized burn ${round1(o.burn)} pts/h over the hour from ${round1(o.pct)}% at ${at}`,
          detail: {
            ratePtsPerHour: round1(o.rate),
            realizedBurnPtsPerHour: round1(o.burn),
            bandPtsPerHour: round1(band),
            thresholdPtsPerHour: round1(threshold),
            bandMargin: BURN_SPIKE_BAND_MARGIN,
            residualP99PtsPerHour: round1(spread),
            residuals: residuals.length,
            pctAtOrigin: o.pct,
            resetsAt: o.resetsAt,
          },
        });
      }
      armed = false;
    } else if (o.ts - lastExceedAt >= BURN_SPIKE_EPISODE_REARM_MS) {
      armed = true;
    }
  }
  let evaluation: DetectorEvaluation;
  if (evaluableOrigins === 0) {
    const reason =
      originsWithForecast === 0
        ? "the pace was never measurable at any origin (projectWindow insufficient-history throughout, or every reading already walled)"
        : originsWithRealized === 0
          ? "no origin had a realized next-hour reading in the same reset period (cadence gaps)"
          : residuals.length < BURN_SPIKE_MIN_RESIDUALS
            ? `fewer than ${BURN_SPIKE_MIN_RESIDUALS} matured forecast residuals (${residuals.length}) — the band is unmeasured`
            : `matured residuals span only ${Math.round(residualSpanMs / HOUR_MS)}h (< ${BURN_SPIKE_MIN_RESIDUAL_SPAN_MS / HOUR_MS}h) — the band is unmeasured`;
    evaluation = {
      detector: "burn-spike",
      serviceId: pool.serviceId,
      window,
      state: "unevaluable",
      reason,
      findings: 0,
    };
  } else {
    evaluation = {
      detector: "burn-spike",
      serviceId: pool.serviceId,
      window,
      state: findings.length ? "fired" : "silent",
      findings: findings.length,
    };
  }
  return { findings, evaluation };
}

// ---------------------------------------------------------------------
// Detector (b): adapter schema drift
// ---------------------------------------------------------------------

/** Live observations are complete parses (every tick), so a field
 * missing from one is gone: confirm 1. */
export const SCHEMA_LIVE_REMOVAL_CONFIRM = 1;
/** Replay observations are per-day store sets and the store
 * appends only on movement, so a field missing from a day may be
 * stasis: disappearance needs this many consecutive ACTIVE days
 * absent before it is a drift finding. */
export const SCHEMA_REPLAY_REMOVAL_CONFIRM_DAYS = 3;

/** Field vocabulary for a live parsed shape: the raw response's
 * top-level keys (`usage:<key>`) and the parsed window kinds
 * (`window:<kind>`, model-scoped names collapsed to their kind —
 * the set of models with usage is data, not schema). */
export function shapeFieldsFromEntry(entry: QuotaCurrentEntry): string[] {
  const fields = new Set<string>();
  if (entry.usage && typeof entry.usage === "object") {
    for (const k of Object.keys(entry.usage as Record<string, unknown>)) fields.add(`usage:${k}`);
  }
  for (const w of entry.windows) {
    const kind = w.name.includes(":") ? (w.name.split(":").at(-1) as string) : w.name;
    // Canonical windows fold renames (5-hour -> 5h, 7-day ->
    // weekly): a rename is not drift at canonical granularity.
    fields.add(`window:${canonicalWindowForMetric(kind) ?? kind}`);
  }
  return [...fields].sort();
}

/** One tick's schema observation for one entry, or null when the
 * entry is not a fresh parse (retained last-known rows are the
 * previous tick's parse, not this one's; an errored/lapsed run
 * with no parse is an outage, not a shape). An entry whose error
 * names the fail-closed unparseable state IS an observation —
 * of the parse falling closed. */
export function shapeObservationFromEntry(entry: QuotaCurrentEntry, ts: string): FieldSetObservation | null {
  if (entry.retainedFromFailedTick) return null;
  if (entry.error && entry.error.includes("unparseable")) {
    return { ts, fields: null, unparseable: entry.error };
  }
  if (entry.state !== "ok") return null;
  return { ts, fields: shapeFieldsFromEntry(entry) };
}

/** Replay observations: per service, per UTC day, the parsed
 * shape's projection the store actually kept — the (metric, unit)
 * field set, model-scoped metrics collapsed to their window kind.
 * The store records no parse outcomes, so the unparseable leg has
 * no replay trace; it is covered by the live path and the unit
 * fixtures, and the replay report says so rather than implying
 * coverage it does not have. */
export function shapeDayObservations(records: readonly SnapshotRecord[]): Map<string, FieldSetObservation[]> {
  const byService = new Map<string, Map<string, Set<string>>>();
  for (const r of records) {
    const day = r.ts.slice(0, 10);
    let days = byService.get(r.serviceId);
    if (!days) byService.set(r.serviceId, (days = new Map()));
    const set = days.get(day) ?? new Set<string>();
    const rawKind = r.metricId.includes(":") ? (r.metricId.split(":").at(-1) as string) : r.metricId;
    // Same canonical fold as the live shape: the store keeps raw
    // source names, so without it a pure rename reads as drift.
    const kind = canonicalWindowForMetric(rawKind) ?? rawKind;
    set.add(`metric:${kind}:${r.unit}`);
    days.set(day, set);
  }
  const out = new Map<string, FieldSetObservation[]>();
  for (const [serviceId, days] of byService) {
    out.set(
      serviceId,
      [...days.entries()]
        .sort(([a], [b]) => a.localeCompare(b))
        .map(([day, set]) => ({ ts: `${day}T00:00:00.000Z`, fields: [...set].sort() })),
    );
  }
  return out;
}

function fieldSetFindings(args: {
  detector: AnomalyDetectorId;
  serviceId: string;
  providerId: string | null;
  window: string | null;
  obs: FieldSetObservation;
  events: FieldSetEvent[];
  subject: string; // "parsed shape" | "window set"
}): { findings: AnomalyFinding[]; evaluation: DetectorEvaluation } {
  const findings: AnomalyFinding[] = [];
  let state: DetectorEvaluationState = "silent";
  let reason: string | undefined;
  for (const ev of args.events) {
    if (ev.kind === "baseline-established") {
      state = "baseline-established";
      reason = `recorded the baseline ${args.subject} (${ev.fields.length} fields); no prior baseline to diff against`;
    } else if (ev.kind === "unevaluable") {
      state = "unevaluable";
      reason = ev.reason;
    } else if (ev.kind === "unparseable-started") {
      state = "fired";
      findings.push({
        detector: args.detector,
        key: `${args.detector}:${args.serviceId}:unparseable:${args.obs.ts}`,
        at: args.obs.ts,
        serviceId: args.serviceId,
        providerId: args.providerId,
        window: args.window,
        severity: "warning",
        title: `adapter schema drift: ${args.serviceId} parse fell into the fail-closed unparseable state`,
        baseline: `recorded ${args.subject} parses successfully`,
        observed: `parse fell closed: ${ev.reason}`,
        detail: { reason: ev.reason },
      });
    } else if (ev.kind === "shape-changed") {
      state = "fired";
      const parts: string[] = [];
      if (ev.added.length) parts.push(`appeared: ${ev.added.join(", ")}`);
      if (ev.removed.length) parts.push(`disappeared: ${ev.removed.join(", ")}`);
      findings.push({
        detector: args.detector,
        key: `${args.detector}:${args.serviceId}:shape:${args.obs.ts}`,
        at: args.obs.ts,
        serviceId: args.serviceId,
        providerId: args.providerId,
        window: args.window,
        severity: "warning",
        title: `${args.detector === "schema-drift" ? "adapter schema drift" : "vendor plan change"}: ${args.serviceId} ${args.subject} changed (${parts.join("; ")})`,
        baseline: `recorded ${args.subject}: ${ev.fields.filter((f) => !ev.added.includes(f)).concat(ev.removed).sort().join(", ") || "(empty)"}`,
        observed: `${args.subject} now: ${ev.fields.join(", ") || "(empty)"} — ${parts.join("; ")}`,
        detail: { added: ev.added, removed: ev.removed },
      });
    }
  }
  return {
    findings,
    evaluation: {
      detector: args.detector,
      serviceId: args.serviceId,
      window: args.window,
      state,
      ...(reason ? { reason } : {}),
      findings: findings.length,
    },
  };
}

// ---------------------------------------------------------------------
// Detector (c): vendor plan change — limit leg + window-kind leg
// ---------------------------------------------------------------------

/** Pool window sets are observed per tick live, but one window can
 * drop out of a single tick's parse on a rejected value: confirm 2
 * consecutive ticks before a disappearance is a plan change. */
export const PLAN_WINDOW_LIVE_REMOVAL_CONFIRM = 2;
/** Replay day-sets carry the store's movement filter, same sustain
 * as the schema replay. */
export const PLAN_WINDOW_REPLAY_REMOVAL_CONFIRM_DAYS = 3;

/** Feed one snapshot record through the limit machine. A record
 * with a null limit is the absence of a denominator: it neither
 * fires nor updates the recorded limit. The first non-null limit
 * establishes the baseline silently (it rides in the state, and
 * the evaluation surface is the caller's per-service rollup). */
export function observeLimitRecord(
  limits: Record<string, number>,
  record: SnapshotRecord,
): AnomalyFinding | null {
  if (record.limit === null || record.limit === undefined) return null;
  const key = `${record.serviceId}|${record.metricId}`;
  const prior = limits[key];
  limits[key] = record.limit;
  if (prior === undefined || prior === record.limit) return null;
  return {
    detector: "plan-change",
    key: `plan-change:${record.serviceId}:${record.metricId}:limit:${prior}->${record.limit}:${record.ts}`,
    at: record.ts,
    serviceId: record.serviceId,
    providerId: null,
    window: canonicalWindowForMetric(record.metricId),
    severity: "warning",
    title: `vendor plan change: ${record.serviceId} ${record.metricId} limit ${prior} -> ${record.limit} (${record.unit})`,
    baseline: `recorded limit for ${record.serviceId}|${record.metricId}: ${prior} ${record.unit}`,
    observed: `limit is now ${record.limit} ${record.unit} (record at ${record.ts})`,
    detail: { metricId: record.metricId, unit: record.unit, from: prior, to: record.limit },
  };
}

/** Canonical window kinds present in one pool observation: live —
 * from a tick entry's parsed windows; replay — from a day's joined
 * records. Both reduce names through canonicalWindowForMetric (with
 * the model-scope suffix retried), so a spelling rename of the
 * same window is NOT a window-kind change (it is schema drift's
 * claim, not this detector's). */
export function poolWindowFields(names: readonly string[]): string[] {
  const out = new Set<string>();
  for (const name of names) {
    const direct = canonicalWindowForMetric(name);
    if (direct) {
      out.add(`window:${direct}`);
      continue;
    }
    if (name.includes(":")) {
      const suffix = canonicalWindowForMetric(name.split(":").at(-1) as string);
      if (suffix) out.add(`window:${suffix}`);
    }
  }
  return [...out].sort();
}

// ---------------------------------------------------------------------
// State file (the tick's recorded baselines) + runners
// ---------------------------------------------------------------------

export interface AnomalyStateFile {
  version: 1;
  updatedAt: string;
  /** schema-drift baselines, by serviceId. */
  shapes: Record<string, FieldSetState>;
  /** plan-change recorded limits, by "serviceId|metricId". */
  limits: Record<string, number>;
  /** plan-change window-kind baselines, by pool serviceId. */
  poolWindows: Record<string, FieldSetState>;
  /** Finding keys already filed (dedupe across ticks); capped,
   * oldest dropped first. */
  firedKeys: string[];
}

export function emptyAnomalyState(now: Date): AnomalyStateFile {
  return { version: 1, updatedAt: now.toISOString(), shapes: {}, limits: {}, poolWindows: {}, firedKeys: [] };
}

export function isAnomalyStateFile(v: unknown): v is AnomalyStateFile {
  if (!v || typeof v !== "object") return false;
  const s = v as Record<string, unknown>;
  return (
    s.version === 1 &&
    typeof s.updatedAt === "string" &&
    typeof s.shapes === "object" && s.shapes !== null &&
    typeof s.limits === "object" && s.limits !== null &&
    typeof s.poolWindows === "object" && s.poolWindows !== null &&
    Array.isArray(s.firedKeys)
  );
}

const FIRED_KEYS_CAP = 1000;

/** The alerts-ledger incident a finding files (the tick posts it
 * via AlertsStore.postIncident; the replay renders the same shape
 * for its replay-injected demonstration). */
export const ANOMALY_ALERT_SOURCE = "quota-anomaly";

export function anomalyIncidentInput(f: AnomalyFinding): {
  source: string;
  key: string;
  severity: "warning";
  title: string;
  body: string;
} {
  return {
    source: ANOMALY_ALERT_SOURCE,
    key: f.key,
    severity: f.severity,
    title: f.title,
    body: `baseline: ${f.baseline}\nobserved: ${f.observed}`,
  };
}

// ---------------------------------------------------------------------
// Replay runner (acceptance harness): all three detectors over the
// snapshot store, in stream order.
// ---------------------------------------------------------------------

export interface AnomalyReplayResult {
  findings: AnomalyFinding[];
  evaluations: DetectorEvaluation[];
}

/** Replay warmup for day-set series: the store appends only when
 * a value moves, so a series' first active day holds only the
 * fields whose values happened to move that day — a partial shape,
 * not the shape's birth. The union of the first two active days is
 * the baseline observation; diffing starts from the third. (Live
 * observations need no warmup: a tick's parse is complete.) */
function warmupDayObservations(observations: FieldSetObservation[]): FieldSetObservation[] {
  if (observations.length < 2) return observations;
  const [first, second, ...rest] = observations as [FieldSetObservation, FieldSetObservation, ...FieldSetObservation[]];
  const union = [...new Set([...(first.fields ?? []), ...(second.fields ?? [])])].sort();
  return [{ ts: second.ts, fields: union }, ...rest];
}

export function runAnomalyReplay(
  records: readonly SnapshotRecord[],
  opts: { asOfMs?: number } = {},
): AnomalyReplayResult {
  const asOfMs = opts.asOfMs ?? Number.POSITIVE_INFINITY;
  const stream = records.filter((r) => Date.parse(r.ts) <= asOfMs);
  const findings: AnomalyFinding[] = [];
  const evaluations: DetectorEvaluation[] = [];

  // (a) burn spike over the pool register's joined series.
  const { index } = indexPercentRecords(stream);
  for (const pool of FORECAST_POOLS) {
    for (const window of Object.keys(FORECAST_WINDOW_METRICS)) {
      const points = pointsForWindow(index, pool.serviceId, window);
      if (!points.length) continue;
      const r = evaluateBurnSpikes({ pool, window, points });
      findings.push(...r.findings);
      evaluations.push(r.evaluation);
    }
  }

  // (b) schema drift over per-service day observations.
  const shapes = new Map<string, FieldSetState>();
  for (const [serviceId, rawObservations] of shapeDayObservations(stream)) {
    const observations = warmupDayObservations(rawObservations);
    let lastEvaluation: DetectorEvaluation | null = null;
    let serviceFindings = 0;
    for (const obs of observations) {
      const { state, events } = observeFieldSet(shapes.get(serviceId), obs, SCHEMA_REPLAY_REMOVAL_CONFIRM_DAYS);
      shapes.set(serviceId, state);
      const r = fieldSetFindings({
        detector: "schema-drift",
        serviceId,
        providerId: null,
        window: null,
        obs,
        events,
        subject: "parsed shape",
      });
      findings.push(...r.findings);
      serviceFindings += r.findings.length;
      lastEvaluation = r.evaluation;
    }
    if (lastEvaluation) {
      evaluations.push({
        ...lastEvaluation,
        state: serviceFindings ? "fired" : lastEvaluation.state === "fired" ? "silent" : lastEvaluation.state,
        findings: serviceFindings,
      });
    }
  }

  // (c1) plan change: limits over the whole stream, in order.
  const limits: Record<string, number> = {};
  let limitFindings = 0;
  for (const r of [...stream].sort((a, b) => a.ts.localeCompare(b.ts))) {
    const f = observeLimitRecord(limits, r);
    if (f) {
      findings.push(f);
      limitFindings++;
    }
  }
  evaluations.push({
    detector: "plan-change",
    serviceId: "(all services)",
    window: null,
    state: limitFindings ? "fired" : "silent",
    reason: "limit leg: consecutive non-null limits per service+metric over the stream",
    findings: limitFindings,
  });

  // (c2) plan change: canonical window kinds per pool, day sets.
  const poolDayWindows = new Map<string, Map<string, Set<string>>>();
  for (const pool of FORECAST_POOLS) {
    const key = forecastAccuracySeriesKey(pool.serviceId);
    const days = new Map<string, Set<string>>();
    for (const [idxKey, recs] of index) {
      if (idxKey !== key && !idxKey.startsWith(`${key}.`)) continue;
      for (const r of recs) {
        const w = canonicalWindowForMetric(r.metricId);
        if (!w) continue;
        const day = r.ts.slice(0, 10);
        const set = days.get(day) ?? new Set<string>();
        set.add(`window:${w}`);
        days.set(day, set);
      }
    }
    poolDayWindows.set(pool.serviceId, days);
  }
  for (const pool of FORECAST_POOLS) {
    const days = poolDayWindows.get(pool.serviceId) as Map<string, Set<string>>;
    if (!days.size) continue;
    let state: FieldSetState | undefined;
    let lastEvaluation: DetectorEvaluation | null = null;
    let poolFindings = 0;
    const dayObservations = warmupDayObservations(
      [...days.keys()].sort().map((day) => ({
        ts: `${day}T00:00:00.000Z`,
        fields: [...(days.get(day) as Set<string>)].sort(),
      })),
    );
    for (const obs of dayObservations) {
      const r0 = observeFieldSet(state, obs, PLAN_WINDOW_REPLAY_REMOVAL_CONFIRM_DAYS);
      state = r0.state;
      const r = fieldSetFindings({
        detector: "plan-change",
        serviceId: pool.serviceId,
        providerId: pool.providerId,
        window: null,
        obs,
        events: r0.events,
        subject: "window set",
      });
      findings.push(...r.findings);
      poolFindings += r.findings.length;
      lastEvaluation = r.evaluation;
    }
    if (lastEvaluation) {
      evaluations.push({
        ...lastEvaluation,
        state: poolFindings ? "fired" : lastEvaluation.state === "fired" ? "silent" : lastEvaluation.state,
        findings: poolFindings,
      });
    }
  }

  findings.sort((a, b) => a.at.localeCompare(b.at) || a.detector.localeCompare(b.detector) || a.key.localeCompare(b.key));
  return { findings, evaluations };
}

// ---------------------------------------------------------------------
// Live tick runner (scripts/gather-quota.ts): this tick's entries +
// fresh records + the store history, against the recorded state.
// ---------------------------------------------------------------------

export interface AnomalyTickResult {
  state: AnomalyStateFile;
  /** Findings not already in the state's firedKeys — the ones the
   * tick files (history record + alerts-ledger incident). */
  findings: AnomalyFinding[];
  evaluations: DetectorEvaluation[];
}

export function runAnomalyTick(args: {
  state: AnomalyStateFile;
  entries: readonly QuotaCurrentEntry[];
  freshRecords: readonly SnapshotRecord[];
  history: readonly SnapshotRecord[];
  now: Date;
}): AnomalyTickResult {
  const state: AnomalyStateFile = {
    version: 1,
    updatedAt: args.now.toISOString(),
    shapes: { ...args.state.shapes },
    limits: { ...args.state.limits },
    poolWindows: { ...args.state.poolWindows },
    firedKeys: [...args.state.firedKeys],
  };
  const findings: AnomalyFinding[] = [];
  const evaluations: DetectorEvaluation[] = [];
  const tickIso = args.now.toISOString();

  // (b) schema drift: merge this tick's per-service observations
  // (several credentials can file under one service; an
  // unparseable parse on any of them is the service's observation).
  const byService = new Map<string, FieldSetObservation[]>();
  for (const entry of args.entries) {
    const obs = shapeObservationFromEntry(entry, tickIso);
    if (!obs) continue;
    const list = byService.get(entry.serviceId) ?? [];
    list.push(obs);
    byService.set(entry.serviceId, list);
  }
  for (const [serviceId, observations] of byService) {
    let merged: FieldSetObservation;
    if (observations.some((o) => o.fields === null)) {
      merged = observations.find((o) => o.fields === null) as FieldSetObservation;
    } else {
      const fields = new Set<string>();
      for (const o of observations) for (const f of o.fields as string[]) fields.add(f);
      merged = { ts: tickIso, fields: [...fields].sort() };
    }
    const { state: next, events } = observeFieldSet(state.shapes[serviceId], merged, SCHEMA_LIVE_REMOVAL_CONFIRM);
    state.shapes[serviceId] = next;
    const r = fieldSetFindings({
      detector: "schema-drift",
      serviceId,
      providerId: null,
      window: null,
      obs: merged,
      events,
      subject: "parsed shape",
    });
    findings.push(...r.findings);
    evaluations.push(r.evaluation);
  }

  // (c1) plan change, limit leg: this tick's appended records.
  for (const r of [...args.freshRecords].sort((a, b) => a.ts.localeCompare(b.ts))) {
    const f = observeLimitRecord(state.limits, r);
    if (f) findings.push(f);
  }

  // (c2) plan change, window-kind leg: pools with a fresh ok entry
  // this tick. A pool with no fresh entry makes no observation —
  // its absence is an outage/retention, not a disappearance.
  const poolNames = new Map<string, { providerId: string | null; names: string[] }>();
  for (const entry of args.entries) {
    if (entry.state !== "ok" || entry.retainedFromFailedTick) continue;
    const pool = FORECAST_POOLS.find((p) => p.serviceId === entry.serviceId);
    if (!pool) continue;
    const slot = poolNames.get(pool.serviceId) ?? { providerId: pool.providerId, names: [] };
    for (const w of entry.windows) slot.names.push(w.name);
    poolNames.set(pool.serviceId, slot);
  }
  for (const [serviceId, slot] of poolNames) {
    const obs: FieldSetObservation = { ts: tickIso, fields: poolWindowFields(slot.names) };
    const { state: next, events } = observeFieldSet(
      state.poolWindows[serviceId],
      obs,
      PLAN_WINDOW_LIVE_REMOVAL_CONFIRM,
    );
    state.poolWindows[serviceId] = next;
    const r = fieldSetFindings({
      detector: "plan-change",
      serviceId,
      providerId: slot.providerId,
      window: null,
      obs,
      events,
      subject: "window set",
    });
    findings.push(...r.findings);
    evaluations.push(r.evaluation);
  }

  // (a) burn spike: trailing-history residuals, fresh origins
  // only. The tick bounds the evaluated history to the trailing
  // 21 days (the replay scores the whole store; the live band is
  // a trailing measurement, and the bound keeps the per-tick cost
  // flat as the store grows).
  const { index } = indexPercentRecords(args.history);
  const originSinceMs = args.now.getTime() - 2 * HOUR_MS;
  const historyFloorMs = args.now.getTime() - 21 * 24 * HOUR_MS;
  for (const pool of FORECAST_POOLS) {
    for (const window of Object.keys(FORECAST_WINDOW_METRICS)) {
      const points = pointsForWindow(index, pool.serviceId, window).filter((p) => p.ts >= historyFloorMs);
      if (!points.length) continue;
      const r = evaluateBurnSpikes({ pool, window, points, originSinceMs });
      findings.push(...r.findings);
      evaluations.push(r.evaluation);
    }
  }

  // Dedupe against the recorded fired keys; record the new ones.
  const seen = new Set(state.firedKeys);
  const fresh = findings.filter((f) => !seen.has(f.key));
  for (const f of fresh) seen.add(f.key);
  state.firedKeys = [...seen].slice(-FIRED_KEYS_CAP);
  return { state, findings: fresh, evaluations };
}
