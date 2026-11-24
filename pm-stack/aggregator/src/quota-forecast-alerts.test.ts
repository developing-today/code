import { describe, expect, test } from "bun:test";
import {
  emptyForecastAlertState,
  evaluateForecastAlerts,
  forecastAlertIncidentInput,
  type ForecastPredictionRecord,
} from "./quota-forecast-alerts.ts";

// Oracle: #13261's acceptance criteria, per the C4 design
// (doc/2026-11-13T15-12-00Z_design_reporting-architecture-event-driven/
// c4-forecasting-design.md §1 + §8):
// 1. Alerts fire for predicted downtime windows and predicted
//    workhorse full-wall predictions meeting the design's thresholds
//    (S3.15 selectivity: ordinary walls do not alert), with the
//    prediction record id attached.
// 2. Repeat ticks do not re-alert the same prediction — dedupe keyed
//    on the prediction record.
// 3. Alert text is the record's stored `rendered` string verbatim,
//    never re-derived at send time.

const NOW = new Date("2026-11-23T12:00:00.000Z");
const WORKHORSE = new Set(["opencode-go"]);

function record(overrides: Partial<ForecastPredictionRecord> = {}): ForecastPredictionRecord {
  return {
    record_version: 1,
    id: "c4:downtime_window:all-pools:1764000000000",
    kind: "downtime_window",
    subject: { pools: ["opencode-go", "meta"] },
    issued_at: "2026-11-23T11:45:00.000Z",
    basis: { name: "since_last_reset", detail: "same-period series per member pool" },
    prediction: {
      earliest: "2026-11-23T18:00:00.000Z",
      latest: "2026-11-24T02:00:00.000Z",
      rendered: "likely runs out between this evening and Monday 2 am",
    },
    methodology_version: "c4-meth:1",
    state: "predicted",
    supersedes: null,
    ...overrides,
  };
}

describe("forecast alerts: firing set (criterion 1)", () => {
  test("a predicted downtime window fires, with the record id attached", () => {
    const r = record();
    const out = evaluateForecastAlerts({
      records: [r],
      state: emptyForecastAlertState(),
      workhorsePools: WORKHORSE,
      now: NOW,
    });
    expect(out.findings).toHaveLength(1);
    const f = out.findings[0];
    expect(f.predictionId).toBe(r.id);
    const input = forecastAlertIncidentInput(f);
    expect(input.source).toBe("quota-prediction");
    // The ledger dedupe key IS the prediction record id (design §8).
    expect(input.key).toBe(r.id);
    expect(f.detail.predictionId).toBe(r.id);
    expect(out.evaluations[0].state).toBe("fired");
  });

  test("a predicted workhorse full-wall (pool_exhaustion for a workhorse pool) fires", () => {
    const r = record({
      id: "c4:pool_exhaustion:opencode-go:weekly:1764000000000",
      kind: "pool_exhaustion",
      subject: { pool: "opencode-go", window: "weekly" },
      prediction: {
        earliest: "2026-11-23T20:00:00.000Z",
        latest: "2026-11-24T04:00:00.000Z",
        rendered: "between 8 and 16 hours from now",
      },
    });
    const out = evaluateForecastAlerts({
      records: [r],
      state: emptyForecastAlertState(),
      workhorsePools: WORKHORSE,
      now: NOW,
    });
    expect(out.findings).toHaveLength(1);
    expect(out.findings[0].kind).toBe("pool_exhaustion");
  });

  test("an ordinary pool wall does not alert (S3.15 selectivity)", () => {
    const r = record({
      id: "c4:pool_exhaustion:meta:weekly:1764000000000",
      kind: "pool_exhaustion",
      subject: { pool: "meta", window: "weekly" },
    });
    const out = evaluateForecastAlerts({
      records: [r],
      state: emptyForecastAlertState(),
      workhorsePools: WORKHORSE,
      now: NOW,
    });
    expect(out.findings).toHaveLength(0);
    expect(out.evaluations[0].state).toBe("silent");
    expect(out.evaluations[0].reason).toContain("workhorse");
  });

  test("reset_duration and model_plan_arrival records do not alert standalone", () => {
    const reset = record({
      id: "c4:reset_duration:opencode-go:1764000000000",
      kind: "reset_duration",
      subject: { pool: "opencode-go" },
    });
    const arrival = record({
      id: "c4:model_plan_arrival:new-plan:1764000000000",
      kind: "model_plan_arrival",
      subject: { plan_or_model: "new-plan" },
    });
    const out = evaluateForecastAlerts({
      records: [reset, arrival],
      state: emptyForecastAlertState(),
      workhorsePools: WORKHORSE,
      now: NOW,
    });
    expect(out.findings).toHaveLength(0);
    expect(out.evaluations.map((e) => e.state)).toEqual(["silent", "silent"]);
  });
});

describe("forecast alerts: dedupe (criterion 2)", () => {
  test("a repeat tick over the same records re-alerts nothing", () => {
    const r = record();
    const first = evaluateForecastAlerts({
      records: [r],
      state: emptyForecastAlertState(),
      workhorsePools: WORKHORSE,
      now: NOW,
    });
    expect(first.findings).toHaveLength(1);
    const second = evaluateForecastAlerts({
      records: [r],
      state: first.state,
      workhorsePools: WORKHORSE,
      now: new Date(NOW.getTime() + 15 * 60000),
    });
    expect(second.findings).toHaveLength(0);
    expect(second.evaluations[0].state).toBe("already-alerted");
  });

  test("a superseding record re-issuing the same prediction does not re-alert", () => {
    const r = record();
    const first = evaluateForecastAlerts({
      records: [r],
      state: emptyForecastAlertState(),
      workhorsePools: WORKHORSE,
      now: NOW,
    });
    const reissued = record({
      id: "c4:downtime_window:all-pools:1764000900000",
      issued_at: "2026-11-23T12:00:00.000Z",
      supersedes: r.id,
    });
    const second = evaluateForecastAlerts({
      records: [reissued],
      state: first.state,
      workhorsePools: WORKHORSE,
      now: new Date(NOW.getTime() + 15 * 60000),
    });
    expect(second.findings).toHaveLength(0);
    expect(second.evaluations[0].state).toBe("already-alerted");
  });

  test("a superseding record whose range moved is a new prediction and fires", () => {
    const r = record();
    const first = evaluateForecastAlerts({
      records: [r],
      state: emptyForecastAlertState(),
      workhorsePools: WORKHORSE,
      now: NOW,
    });
    const moved = record({
      id: "c4:downtime_window:all-pools:1764000900000",
      issued_at: "2026-11-23T12:00:00.000Z",
      supersedes: r.id,
      prediction: {
        earliest: "2026-11-23T22:00:00.000Z",
        latest: "2026-11-24T06:00:00.000Z",
        rendered: "likely runs out between tonight and Monday 6 am",
      },
    });
    const second = evaluateForecastAlerts({
      records: [moved],
      state: first.state,
      workhorsePools: WORKHORSE,
      now: new Date(NOW.getTime() + 15 * 60000),
    });
    expect(second.findings).toHaveLength(1);
    expect(second.findings[0].predictionId).toBe(moved.id);
  });
});

describe("forecast alerts: verbatim text (criterion 3)", () => {
  test("the alert body is the record's stored rendered string, byte for byte", () => {
    const r = record();
    const out = evaluateForecastAlerts({
      records: [r],
      state: emptyForecastAlertState(),
      workhorsePools: WORKHORSE,
      now: NOW,
    });
    const input = forecastAlertIncidentInput(out.findings[0]);
    expect(input.body).toBe(r.prediction.rendered);
    expect(out.findings[0].body).toBe(r.prediction.rendered);
  });

  test("the body never re-derives from other record fields", () => {
    const a = record();
    const b = record({
      id: "c4:downtime_window:all-pools:1764000900000",
      basis: { name: "trailing_30d", detail: "a different basis entirely" },
      methodology_version: "c4-meth:99",
    });
    const ea = evaluateForecastAlerts({
      records: [a],
      state: emptyForecastAlertState(),
      workhorsePools: WORKHORSE,
      now: NOW,
    });
    const eb = evaluateForecastAlerts({
      records: [b],
      state: emptyForecastAlertState(),
      workhorsePools: WORKHORSE,
      now: NOW,
    });
    expect(ea.findings[0].body).toBe(eb.findings[0].body);
  });
});

describe("forecast alerts: honesty states", () => {
  test("a non-predicted record is silent with its state named, never an alert", () => {
    const r = record({ state: "insufficient_history", state_reason: "below the sufficiency gate" });
    const out = evaluateForecastAlerts({
      records: [r],
      state: emptyForecastAlertState(),
      workhorsePools: WORKHORSE,
      now: NOW,
    });
    expect(out.findings).toHaveLength(0);
    expect(out.evaluations[0].state).toBe("silent");
    expect(out.evaluations[0].reason).toContain("insufficient_history");
  });

  test("a predicted record with no rendered text is unevaluable, never fired", () => {
    const r = record({
      prediction: { earliest: "2026-11-23T18:00:00.000Z", latest: "2026-11-24T02:00:00.000Z", rendered: "" },
    });
    const out = evaluateForecastAlerts({
      records: [r],
      state: emptyForecastAlertState(),
      workhorsePools: WORKHORSE,
      now: NOW,
    });
    expect(out.findings).toHaveLength(0);
    expect(out.evaluations[0].state).toBe("unevaluable");
    expect(out.evaluations[0].reason).toContain("rendered");
  });

  test("a predicted record whose range is inverted is unevaluable, never fired", () => {
    const r = record({
      prediction: {
        earliest: "2026-11-24T02:00:00.000Z",
        latest: "2026-11-23T18:00:00.000Z",
        rendered: "between 8 and 16 hours from now",
      },
    });
    const out = evaluateForecastAlerts({
      records: [r],
      state: emptyForecastAlertState(),
      workhorsePools: WORKHORSE,
      now: NOW,
    });
    expect(out.findings).toHaveLength(0);
    expect(out.evaluations[0].state).toBe("unevaluable");
  });
});
