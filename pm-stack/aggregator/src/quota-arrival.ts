// C4.4 — new model / plan arrival predictions (#13260, component of
// #9572; design doc/2026-11-13T15-12-00Z_design_reporting-archi-
// tecture-event-driven/c4-forecasting-design.md §1, §3, §7).
//
// Watch-based and honesty-first, because this kind's evidence is the
// weakest of the C4 kinds: S5 detects plan changes after the fact
// (#12699's plan-change detector, quota-anomaly.ts); nothing
// observes an arrival before it happens. The only bases a prediction
// may stand on are the ones the design names:
//
// - detector_history: the pool's own plan-change findings, read as
//   arrival instants (several findings at one instant — one per
//   window that noticed — are ONE arrival). Two or more measured
//   intervals between arrivals bound the next one: [last + shortest
//   interval, last + longest interval], likely at the median. One
//   interval is a sample of one — no spread is measurable — and
//   intervals with no spread at all cannot bound a range either (the
//   design refuses earliest == latest), so both are the
//   insufficient_history honesty state, never a widened guess.
// - vendor_cadence: a cadence actually recorded somewhere (config,
//   issue, doc — the record names where), with a measured spread. A
//   recorded period with no spread would be a point date, and a
//   point date is exactly the speculative date this kind forbids.
//
// Absent every basis, the subject carries the explicit no_basis
// state — a published answer with a named reason, not a silent
// absence (the design: "no_basis is the expected common state"). A
// no_basis record names basis "none": naming a real basis for it
// would be the speculative move the kind forbids. A record whose
// state is not "predicted" never carries a prediction range;
// validateArrivalRecord enforces the whole contract and the builder
// runs every record it emits through it, so a speculative date
// cannot leave this module (acceptance: "a record without a basis
// is never published").
//
// Scoring (§3): hit/miss per predicted record plus lead time on
// hits, via scoreArrivalPrediction when an arrival is realized.
// no_basis / insufficient_history records are never scored — they
// are the honest answer, and scoring them would pay for guessing.
//
// rendered (§4): the stored string comes from the one shared
// formatter (renderPredictionRange in quota-downtime.ts) at
// issuance — what was shown is exactly what the scorer grades;
// the machine range never leaves the record regardless.
//
// Pure functions over plain values — no I/O, no clock reads except
// through arguments (the quota-anomaly.ts discipline). The live
// wiring (gatherer tick -> quota-predictions.jsonl + the Turso
// mirror under source "quota-prediction", design §1) is a follow-up
// slice; this module is the record contract that wiring writes.
import type { AnomalyFinding } from "./quota-anomaly.ts";
import { renderPredictionRange } from "./quota-downtime.ts";

/** Methodology version for the arrival kind (design §6; the
 * c4-methodology-log.jsonl entry of the same name): every record
 * this module emits names it, so a score is always attributable
 * to the method that produced it. c4-meth:3 (#13536) dates
 * far-out calendar bounds; c4-meth:2 (#13408) unified the stored
 * rendering on the one shared §4 formatter; c4-meth:1 records
 * rendered with this module's own duration/day-only copy. */
export const ARRIVAL_METHODOLOGY_VERSION = "c4-meth:3";

/** Ledger source the live wiring mirrors arrival records under
 * (design §1) — distinct from S1's "quota-forecast"; C4 records
 * never mix into it. Named here so the wiring slice has exactly one
 * spelling to import. */
export const PREDICTION_LEDGER_SOURCE = "quota-prediction";

export type ArrivalBasisName = "detector_history" | "vendor_cadence";
export type ArrivalState = "predicted" | "insufficient_history" | "no_basis";

export interface ArrivalSubject {
  /** Canonical provider id (the S1 keying: the provider id sessions
   * carry, never the aggregator's service ids). */
  pool: string;
  /** The plan or model whose arrival is watched, when named. */
  planOrModel?: string;
}

/** A vendor cadence that is actually recorded somewhere. */
export interface RecordedVendorCadence {
  /** Mean interval between arrivals, in milliseconds. */
  periodMs: number;
  /** Measured spread around the period (half-width), milliseconds.
   * Must be > 0 for the cadence to bound a range. */
  spreadMs: number;
  /** ISO instant of the most recent arrival the cadence anchors on. */
  lastArrivalAt: string;
  /** Where the cadence is recorded (config / issue / doc ref) —
   * named verbatim in the record's basis detail. */
  recordedIn: string;
  /** Arrivals behind the recorded cadence. */
  samples: number;
}

export interface ArrivalBuildInput {
  subject: ArrivalSubject;
  /** ISO instant the records are issued at. */
  issuedAt: string;
  /** S5 findings to read detector_history from; only plan-change
   * findings attributed to the subject pool (providerId) count. */
  detectorFindings: readonly AnomalyFinding[];
  /** The recorded vendor cadence for this subject, or null. */
  vendorCadence: RecordedVendorCadence | null;
  methodologyVersion?: string;
  supersedes?: string | null;
}

export interface ArrivalPrediction {
  earliest: string;
  latest: string;
  likely?: string;
  /** The range in Drew's formats (§4), stored at issuance: what was
   * shown is exactly what the scorer grades. */
  rendered: string;
}

export interface ArrivalPredictionRecord {
  record_version: 1;
  id: string;
  kind: "model_plan_arrival";
  subject: { pool: string; plan_or_model?: string };
  issued_at: string;
  basis: { name: ArrivalBasisName | "none"; detail: string };
  inputs: {
    series: string;
    samples: number;
    span: { from: string; to: string } | null;
    adjustments: string[];
  };
  prediction: ArrivalPrediction | null;
  methodology_version: string;
  state: ArrivalState;
  state_reason?: string;
  supersedes: string | null;
}

const HOUR_MS = 3_600_000;
const DAY_MS = 86_400_000;

const iso = (ms: number): string => new Date(ms).toISOString();

function median(sortedAsc: readonly number[]): number {
  if (sortedAsc.length === 1) return sortedAsc[0];
  const pos = (sortedAsc.length - 1) / 2;
  const base = Math.floor(pos);
  const rest = pos - base;
  const next = sortedAsc[base + 1];
  return next !== undefined ? sortedAsc[base] + rest * (next - sortedAsc[base]) : sortedAsc[base];
}

/** A duration in words for basis details: whole hours under two
 * days, else days to one decimal. */
function dur(ms: number): string {
  if (ms < 48 * HOUR_MS) {
    const h = Math.round((ms / HOUR_MS) * 10) / 10;
    return `${Number.isInteger(h) ? String(h) : h.toFixed(1)} hours`;
  }
  const d = Math.round((ms / DAY_MS) * 10) / 10;
  return `${Number.isInteger(d) ? String(d) : d.toFixed(1)} days`;
}

/** The distinct arrival instants a pool's plan-change findings
 * record, ascending. Findings are per-window; several at one
 * instant are one arrival (the plan changed once). Findings with no
 * canonical providerId cannot be attributed to a pool and never
 * count — unattributable evidence is not evidence for this pool. */
function arrivalInstants(findings: readonly AnomalyFinding[], pool: string): number[] {
  const seen = new Set<number>();
  for (const f of findings) {
    if (f.detector !== "plan-change") continue;
    if (f.providerId !== pool) continue;
    const t = Date.parse(f.at);
    if (Number.isFinite(t)) seen.add(t);
  }
  return [...seen].sort((a, b) => a - b);
}

function subjectKey(subject: ArrivalSubject): string {
  const clean = (s: string): string => s.replace(/[^A-Za-z0-9._-]+/g, "-");
  return subject.planOrModel ? `${clean(subject.pool)}:${clean(subject.planOrModel)}` : clean(subject.pool);
}

function makeRecord(
  input: ArrivalBuildInput,
  issuedMs: number,
  basisName: ArrivalBasisName | "none",
  basisDetail: string,
  rest: Pick<ArrivalPredictionRecord, "inputs" | "prediction" | "state" | "state_reason">,
): ArrivalPredictionRecord {
  const subject: ArrivalPredictionRecord["subject"] = { pool: input.subject.pool };
  if (input.subject.planOrModel) subject.plan_or_model = input.subject.planOrModel;
  return {
    record_version: 1,
    id: `c4:model_plan_arrival:${subjectKey(input.subject)}:${basisName}:${issuedMs}`,
    kind: "model_plan_arrival",
    subject,
    issued_at: iso(issuedMs),
    basis: { name: basisName, detail: basisDetail },
    inputs: rest.inputs,
    prediction: rest.prediction,
    methodology_version: input.methodologyVersion ?? ARRIVAL_METHODOLOGY_VERSION,
    state: rest.state,
    ...(rest.state_reason !== undefined ? { state_reason: rest.state_reason } : {}),
    supersedes: input.supersedes ?? null,
  };
}

/**
 * Build the arrival records for one subject at one issuance: one
 * predicted record per basis that can honestly bound a range, or —
 * when no basis can — exactly one honesty-state record (no_basis
 * when no basis has any evidence, insufficient_history when a basis
 * has evidence that cannot bound a range) naming every reason.
 * Every emitted record passes validateArrivalRecord first.
 */
export function buildArrivalRecords(input: ArrivalBuildInput): ArrivalPredictionRecord[] {
  const issuedMs = Date.parse(input.issuedAt);
  if (!Number.isFinite(issuedMs)) throw new Error(`arrival issuedAt does not parse: ${input.issuedAt}`);
  const records: ArrivalPredictionRecord[] = [];
  const reasons: string[] = [];
  let evidenceSeen = false;

  // --- detector_history -------------------------------------------
  const instants = arrivalInstants(input.detectorFindings, input.subject.pool);
  if (instants.length >= 2) {
    evidenceSeen = true;
    const intervals: number[] = [];
    for (let i = 1; i < instants.length; i++) intervals.push(instants[i] - instants[i - 1]);
    const sorted = [...intervals].sort((a, b) => a - b);
    const minI = sorted[0];
    const maxI = sorted[sorted.length - 1];
    if (intervals.length >= 2 && maxI > minI) {
      const last = instants[instants.length - 1];
      const earliest = last + minI;
      const latest = last + maxI;
      records.push(
        makeRecord(
          input,
          issuedMs,
          "detector_history",
          `${instants.length} plan-change arrivals recorded for ${input.subject.pool} between ${iso(instants[0])} and ${iso(last)}; ${intervals.length} intervals, shortest ${dur(minI)}, longest ${dur(maxI)}`,
          {
            inputs: {
              series: "quota-anomaly:plan-change",
              samples: instants.length,
              span: { from: iso(instants[0]), to: iso(last) },
              adjustments: [],
            },
            prediction: {
              earliest: iso(earliest),
              latest: iso(latest),
              likely: iso(last + median(sorted)),
              rendered: renderPredictionRange(iso(earliest), iso(latest), iso(issuedMs)),
            },
            state: "predicted",
            state_reason: undefined,
          },
        ),
      );
    } else if (intervals.length >= 2) {
      reasons.push(
        `detector_history: ${intervals.length} intervals between ${instants.length} arrivals are all identical (${dur(minI)}) — no measured spread, so a range cannot be bounded`,
      );
    } else {
      reasons.push("detector_history: one interval between 2 arrivals — a sample of one, no spread measurable");
    }
  } else {
    reasons.push(
      `detector_history: ${instants.length} plan-change arrival(s) recorded for this pool — no interval measurable`,
    );
  }

  // --- vendor_cadence ---------------------------------------------
  const cad = input.vendorCadence;
  if (cad === null) {
    reasons.push("vendor_cadence: no cadence recorded for this pool");
  } else {
    const anchorMs = Date.parse(cad.lastArrivalAt);
    if (!Number.isFinite(anchorMs)) {
      evidenceSeen = true;
      reasons.push(
        `vendor_cadence: the cadence recorded in ${cad.recordedIn} anchors on an unparseable instant (${cad.lastArrivalAt})`,
      );
    } else if (cad.periodMs <= 0) {
      evidenceSeen = true;
      reasons.push(`vendor_cadence: the cadence recorded in ${cad.recordedIn} has no positive period`);
    } else if (cad.spreadMs <= 0) {
      evidenceSeen = true;
      reasons.push(
        `vendor_cadence: the cadence recorded in ${cad.recordedIn} has no measured spread — a period alone would be a point date, and a point date is speculative`,
      );
    } else {
      // The first occurrence whose window has not fully passed.
      let k = 1;
      while (anchorMs + k * cad.periodMs + cad.spreadMs < issuedMs && k < 100_000) k++;
      const earliest = anchorMs + k * cad.periodMs - cad.spreadMs;
      const latest = anchorMs + k * cad.periodMs + cad.spreadMs;
      records.push(
        makeRecord(
          input,
          issuedMs,
          "vendor_cadence",
          `cadence recorded in ${cad.recordedIn}: period ${dur(cad.periodMs)}, spread ±${dur(cad.spreadMs)}, anchored on the arrival at ${cad.lastArrivalAt} (${cad.samples} arrivals behind it); occurrence ${k} projected`,
          {
            inputs: {
              series: `vendor-cadence:${cad.recordedIn}`,
              samples: cad.samples,
              span: { from: cad.lastArrivalAt, to: cad.lastArrivalAt },
              adjustments: [],
            },
            prediction: {
              earliest: iso(earliest),
              latest: iso(latest),
              likely: iso(anchorMs + k * cad.periodMs),
              rendered: renderPredictionRange(iso(earliest), iso(latest), iso(issuedMs)),
            },
            state: "predicted",
            state_reason: undefined,
          },
        ),
      );
    }
  }

  // --- no basis could bound a range: the honesty state ------------
  if (records.length === 0) {
    records.push(
      makeRecord(input, issuedMs, "none", "no basis supports a prediction for this subject", {
        inputs: { series: "", samples: 0, span: null, adjustments: [] },
        prediction: null,
        state: evidenceSeen ? "insufficient_history" : "no_basis",
        state_reason: reasons.join("; "),
      }),
    );
  }

  for (const rec of records) validateArrivalRecord(rec);
  return records;
}

/**
 * The §1/§7 contract, enforced: a predicted record must name a real
 * basis, carry a strictly-bounded range (earliest < latest — a
 * point date is refused) with its stored rendering and the readings
 * behind the basis; a non-predicted record must carry no range at
 * all (a range on an honesty state would be a speculative date) and
 * must name its reason. Throws on the first violation.
 */
export function validateArrivalRecord(rec: ArrivalPredictionRecord): void {
  function fail(why: string): never {
    throw new Error(`arrival record ${rec.id || "<no id>"}: ${why}`);
  }
  if (rec.record_version !== 1) fail(`record_version must be 1, got ${rec.record_version}`);
  if (rec.kind !== "model_plan_arrival") fail(`kind must be model_plan_arrival, got ${rec.kind}`);
  if (!rec.methodology_version) fail("methodology_version is required");
  if (!Number.isFinite(Date.parse(rec.issued_at))) fail(`issued_at does not parse: ${rec.issued_at}`);
  if (rec.state === "predicted") {
    if (rec.basis.name === "none") {
      fail("a predicted record must name its basis — a record without a basis is never published");
    }
    const prediction = rec.prediction;
    if (!prediction) fail("a predicted record must carry its prediction range");
    const e = Date.parse(prediction.earliest);
    const l = Date.parse(prediction.latest);
    if (!Number.isFinite(e) || !Number.isFinite(l)) fail("prediction bounds must be ISO instants");
    if (!(e < l)) fail("prediction must satisfy earliest < latest — a point date is refused");
    if (!prediction.rendered) fail("prediction.rendered is required (stored, not recomputed)");
    if (rec.inputs.samples <= 0) fail("a predicted record must name the readings behind its basis");
  } else {
    if (rec.prediction !== null) {
      fail(`a ${rec.state} record must not carry a prediction range — that would be a speculative date`);
    }
    if (!rec.state_reason) fail(`a ${rec.state} record must name its reason`);
  }
}

export interface ArrivalScore {
  scored: boolean;
  /** Why not, when scored is false. */
  reason?: string;
  /** Realized instant inside [earliest, latest], when scored. */
  hit?: boolean;
  /** Realization minus issuance on a hit; null on a miss (§3: lead
   * time is reported on hits). */
  leadTimeMs?: number | null;
  realizedAt?: string;
}

/**
 * Score one record against a realized arrival (§3): hit/miss plus
 * lead time on hits. Open predictions (realizedAt null) are
 * unscored, never assumed; honesty-state records are not scored at
 * all — they are the honest answer, and scoring them would pay for
 * guessing.
 */
export function scoreArrivalPrediction(rec: ArrivalPredictionRecord, realizedAt: string | null): ArrivalScore {
  if (rec.state !== "predicted" || !rec.prediction) {
    return {
      scored: false,
      reason: `${rec.state} records are not scored — they are the honest answer, and scoring them would pay for guessing`,
    };
  }
  if (realizedAt === null) {
    return { scored: false, reason: "arrival not yet realized — open predictions are unscored, never assumed" };
  }
  const prediction = rec.prediction;
  const rMs = Date.parse(realizedAt);
  if (!Number.isFinite(rMs)) return { scored: false, reason: `realized instant does not parse: ${realizedAt}` };
  const hit = rMs >= Date.parse(prediction.earliest) && rMs <= Date.parse(prediction.latest);
  return {
    scored: true,
    hit,
    leadTimeMs: hit ? rMs - Date.parse(rec.issued_at) : null,
    realizedAt,
  };
}
