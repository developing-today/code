// Forecast push alerts (#13261, C4.5 of #9572; design
// doc/2026-11-13T15-12-00Z_design_reporting-architecture-event-driven/
// c4-forecasting-design.md §1 + §8, statements S2.12 + S3.15):
// C4 prediction records that meet the design's selectivity become
// incidents in the alerts ledger via the S5 precedent
// (AlertsStore.postIncident, source `quota-prediction`) — the same
// throat the quota-anomaly detectors post through, with no new
// plumbing of its own.
//
// The selectivity (design §8, per S3.15 — not every wall/reset
// alerts):
// - `downtime_window` records in state `predicted` alert: a
//   predicted complete downtime is the breakthrough case.
// - `pool_exhaustion` records in state `predicted` alert only when
//   the subject pool serves the workhorse route section (the
//   predicted workhorse full-wall); ordinary pool walls do not
//   alert. The workhorse pool set is a caller input, joined from
//   the same section→pools config the section-risk notifications
//   use (aa.ts sectionPoolIdsFromConfig) — this module names no
//   roster of its own.
// - `reset_duration` and `model_plan_arrival` records never alert
//   standalone in this slice: the design's alert set is the two
//   cases above, and a reset duration rides its downtime/workhorse
//   context rather than pushing on its own.
//
// Dedupe is keyed on the prediction record (design §8): the ledger
// incident key IS the record id, and the tick state remembers every
// alerted record id. A superseding record that re-issues the same
// prediction (same kind + subject + range) is the same prediction
// for alert purposes and does not re-alert; a superseding record
// whose range moved is a new prediction and alerts.
//
// Text discipline (design §1: "`rendered` is stored, not
// recomputed"): the alert body is the record's stored `rendered`
// string verbatim — this module never formats a range, so what
// Drew read in the alert is exactly what the scorer grades.
//
// Honesty rules, mirroring the neighbourhood:
// - A record in a non-`predicted` state (insufficient_history /
//   no_basis / adjustment_unavailable) is a published honesty
//   state: evaluated "silent" with the state named, never an
//   alert, never an error.
// - A `predicted` record that cannot be alerted on as recorded
//   (no rendered text, an unparseable or inverted range) is
//   "unevaluable" with the reason named — never fired on a
//   partial record, never silently dropped.
//
// Pure functions over plain values — no I/O, no clock reads except
// through arguments. The gatherer tick hook that feeds records in
// and posts findings lands with the C4.1/C4.2 producers (#13257 /
// #13258), whose records are this module's input.
import type { Severity } from "./alerts.ts";

// --- The prediction record (design §1), as consumed here -----------

export type ForecastPredictionKind =
  | "pool_exhaustion"
  | "downtime_window"
  | "reset_duration"
  | "model_plan_arrival";

export type ForecastPredictionState =
  | "predicted"
  | "insufficient_history"
  | "no_basis"
  | "adjustment_unavailable";

/** The C4 prediction record (design §1). Only the fields the alert
 * path reads are required here; producers write the full record. */
export interface ForecastPredictionRecord {
  record_version: 1;
  id: string;
  kind: ForecastPredictionKind;
  subject: {
    pool?: string;
    window?: string;
    pools?: string[];
    plan_or_model?: string;
  };
  issued_at: string;
  basis: { name: string; detail: string };
  prediction: {
    earliest: string;
    latest: string;
    likely?: string;
    rendered: string;
  };
  methodology_version: string;
  state: ForecastPredictionState;
  state_reason?: string;
  supersedes: string | null;
}

// --- Findings, evaluations, state -----------------------------------

/** One alertable prediction, shaped for the alerts ledger. `key`
 * is the ledger incident key and IS the prediction record id, so
 * the throat's own (source, incident_key) dedupe is the second
 * layer of the design's record-keyed dedupe. */
export interface ForecastAlertFinding {
  key: string;
  predictionId: string;
  kind: ForecastPredictionKind;
  /** The record's issued_at: the instant the finding is about. */
  at: string;
  severity: Extract<Severity, "warning">;
  title: string;
  /** The record's stored `rendered` string, verbatim. */
  body: string;
  detail: Record<string, unknown>;
}

export type ForecastAlertEvaluationState =
  | "fired"
  | "silent"
  | "already-alerted"
  | "unevaluable";

/** The per-record verdict row: how a reader tells a real all-clear
 * ("silent", reason named) from a record that could not be judged
 * ("unevaluable", reason named) or one already pushed
 * ("already-alerted"). */
export interface ForecastAlertEvaluation {
  recordId: string;
  kind: ForecastPredictionKind;
  state: ForecastAlertEvaluationState;
  reason?: string;
}

/** One alerted prediction, remembered across ticks. The range
 * rides so a superseding re-issue of the same prediction is
 * recognised as the same prediction (see the header). */
export interface ForecastAlertFiredEntry {
  recordId: string;
  kind: ForecastPredictionKind;
  subjectKey: string;
  earliest: string;
  latest: string;
  firedAt: string;
}

export interface ForecastAlertState {
  version: 1;
  fired: ForecastAlertFiredEntry[];
}

export function emptyForecastAlertState(): ForecastAlertState {
  return { version: 1, fired: [] };
}

/** The alert-identity of a record's subject: the member set for a
 * downtime window (order-free), pool+window for a pool kind. */
export function forecastAlertSubjectKey(r: ForecastPredictionRecord): string {
  if (r.kind === "downtime_window") {
    return [...(r.subject.pools ?? [])].sort().join(",");
  }
  if (r.subject.pool) {
    return r.subject.window ? `${r.subject.pool}:${r.subject.window}` : r.subject.pool;
  }
  return r.subject.plan_or_model ?? "";
}

// --- Evaluation -------------------------------------------------------

export interface ForecastAlertInput {
  records: ForecastPredictionRecord[];
  state: ForecastAlertState;
  /** Provider ids serving the workhorse route section, joined by
   * the caller from the section→pools config. */
  workhorsePools: ReadonlySet<string>;
  now: Date;
}

export interface ForecastAlertResult {
  findings: ForecastAlertFinding[];
  evaluations: ForecastAlertEvaluation[];
  state: ForecastAlertState;
}

function titleFor(r: ForecastPredictionRecord): string {
  if (r.kind === "downtime_window") {
    return "Predicted complete downtime — every routing pool walled";
  }
  // pool_exhaustion on a workhorse pool.
  const window = r.subject.window ? ` ${r.subject.window}` : "";
  return `Predicted workhorse full-wall: ${r.subject.pool ?? ""}${window}`;
}

export function evaluateForecastAlerts(input: ForecastAlertInput): ForecastAlertResult {
  const findings: ForecastAlertFinding[] = [];
  const evaluations: ForecastAlertEvaluation[] = [];
  const fired: ForecastAlertFiredEntry[] = [...input.state.fired];
  const firedIds = new Set(fired.map((e) => e.recordId));

  for (const r of input.records) {
    const verdict = (state: ForecastAlertEvaluationState, reason?: string) => {
      evaluations.push({ recordId: r.id, kind: r.kind, state, ...(reason ? { reason } : {}) });
    };

    // Honesty states are published answers, not alertable input.
    if (r.state !== "predicted") {
      verdict(
        "silent",
        `prediction state ${r.state}${r.state_reason ? `: ${r.state_reason}` : ""}`,
      );
      continue;
    }

    // Selectivity (design §8 / S3.15): which kinds alert at all.
    if (r.kind === "reset_duration" || r.kind === "model_plan_arrival") {
      verdict("silent", `${r.kind} records do not alert standalone (design §8 selectivity)`);
      continue;
    }
    if (r.kind === "pool_exhaustion") {
      if (!r.subject.pool || !input.workhorsePools.has(r.subject.pool)) {
        verdict(
          "silent",
          "ordinary wall: subject pool does not serve the workhorse section (S3.15 selectivity)",
        );
        continue;
      }
    }

    // A predicted record must carry its alert text and a sane
    // range as recorded; anything less is unevaluable, never fired.
    if (!r.prediction || typeof r.prediction.rendered !== "string" || r.prediction.rendered.length === 0) {
      verdict("unevaluable", "predicted record carries no rendered text");
      continue;
    }
    const earliestMs = Date.parse(r.prediction.earliest);
    const latestMs = Date.parse(r.prediction.latest);
    if (!Number.isFinite(earliestMs) || !Number.isFinite(latestMs)) {
      verdict("unevaluable", "predicted range bounds are not parseable instants");
      continue;
    }
    if (earliestMs > latestMs) {
      verdict("unevaluable", "predicted range is inverted (earliest after latest)");
      continue;
    }

    // Dedupe, keyed on the prediction record: the same record id,
    // or a superseding re-issue of the same prediction (same kind,
    // subject, and range), never re-alerts.
    const subjectKey = forecastAlertSubjectKey(r);
    if (firedIds.has(r.id)) {
      verdict("already-alerted", "record id already alerted");
      continue;
    }
    const samePrediction = fired.find(
      (e) =>
        e.kind === r.kind &&
        e.subjectKey === subjectKey &&
        e.earliest === r.prediction.earliest &&
        e.latest === r.prediction.latest,
    );
    if (samePrediction) {
      verdict("already-alerted", `same prediction already alerted as ${samePrediction.recordId}`);
      continue;
    }

    const finding: ForecastAlertFinding = {
      key: r.id,
      predictionId: r.id,
      kind: r.kind,
      at: r.issued_at,
      severity: "warning",
      title: titleFor(r),
      body: r.prediction.rendered,
      detail: {
        predictionId: r.id,
        kind: r.kind,
        subjectKey,
        basis: r.basis?.name ?? null,
        earliest: r.prediction.earliest,
        latest: r.prediction.latest,
        methodologyVersion: r.methodology_version,
      },
    };
    findings.push(finding);
    const entry: ForecastAlertFiredEntry = {
      recordId: r.id,
      kind: r.kind,
      subjectKey,
      earliest: r.prediction.earliest,
      latest: r.prediction.latest,
      firedAt: input.now.toISOString(),
    };
    fired.push(entry);
    firedIds.add(r.id);
    verdict("fired");
  }

  return { findings, evaluations, state: { version: 1, fired } };
}

// --- Ledger posting (the S5 precedent's shape) ------------------------

export const FORECAST_ALERT_SOURCE = "quota-prediction";

export function forecastAlertIncidentInput(f: ForecastAlertFinding): {
  source: string;
  key: string;
  severity: "warning";
  title: string;
  body: string;
} {
  return {
    source: FORECAST_ALERT_SOURCE,
    key: f.key,
    severity: f.severity,
    title: f.title,
    body: f.body,
  };
}
