import { describe, expect, test } from "bun:test";
import {
  buildArrivalRecords,
  scoreArrivalPrediction,
  validateArrivalRecord,
  type ArrivalPredictionRecord,
} from "./quota-arrival.ts";
import { renderPredictionRange } from "./quota-downtime.ts";
import type { AnomalyFinding } from "./quota-anomaly.ts";

// Oracle: the C4 design record (doc/2026-11-13T15-12-00Z_design_
// reporting-architecture-event-driven/c4-forecasting-design.md §1,
// §3, §7) and #13260's acceptance: an arrival record is emitted
// only when a named basis exists; absent a basis the subject
// carries the explicit no_basis state; a record without a basis is
// never published (no speculative dates); a range always satisfies
// earliest < latest; no_basis records are not scored; hits report
// lead time.

const DAY = 86_400_000;
const HOUR = 3_600_000;
const T0 = Date.parse("2026-10-01T00:00:00.000Z");
const iso = (ms: number): string => new Date(ms).toISOString();
const ISSUED = iso(T0 + 40 * DAY);

function planChange(atMs: number, over: Partial<AnomalyFinding> = {}): AnomalyFinding {
  return {
    detector: "plan-change",
    key: `plan-change:svc:metric:limit:100->200:${iso(atMs)}`,
    at: iso(atMs),
    serviceId: "svc",
    providerId: "pool-a",
    window: "weekly",
    severity: "warning",
    title: "plan change",
    baseline: "100",
    observed: "200",
    detail: {},
    ...over,
  };
}

const baseInput = {
  subject: { pool: "pool-a" },
  issuedAt: ISSUED,
  detectorFindings: [] as AnomalyFinding[],
  vendorCadence: null,
};

describe("no basis at all", () => {
  test("emits exactly one no_basis record with no prediction and both absences named", () => {
    const recs = buildArrivalRecords(baseInput);
    expect(recs).toHaveLength(1);
    const rec = recs[0];
    expect(rec.state).toBe("no_basis");
    expect(rec.prediction).toBeNull();
    expect(rec.basis.name).toBe("none");
    expect(rec.state_reason).toContain("detector_history");
    expect(rec.state_reason).toContain("vendor_cadence");
    expect(rec.kind).toBe("model_plan_arrival");
    expect(rec.id).toContain("c4:model_plan_arrival:pool-a:none:");
  });

  test("findings for other pools and other detectors do not create a basis", () => {
    const recs = buildArrivalRecords({
      ...baseInput,
      detectorFindings: [
        planChange(T0, { providerId: "pool-b" }),
        planChange(T0 + 10 * DAY, { providerId: "pool-b" }),
        planChange(T0 + 20 * DAY, { providerId: "pool-b" }),
        planChange(T0 + 5 * DAY, { detector: "burn-spike" }),
        planChange(T0 + 15 * DAY, { providerId: null }),
      ],
    });
    expect(recs).toHaveLength(1);
    expect(recs[0].state).toBe("no_basis");
  });
});

describe("detector_history basis", () => {
  test("two or more spread intervals bound the next arrival", () => {
    // Arrivals at day 0, 10, 25 -> intervals 10d and 15d.
    const recs = buildArrivalRecords({
      ...baseInput,
      detectorFindings: [planChange(T0), planChange(T0 + 10 * DAY), planChange(T0 + 25 * DAY)],
    });
    expect(recs).toHaveLength(1);
    const rec = recs[0];
    expect(rec.state).toBe("predicted");
    expect(rec.basis.name).toBe("detector_history");
    expect(rec.prediction?.earliest).toBe(iso(T0 + 35 * DAY));
    expect(rec.prediction?.latest).toBe(iso(T0 + 40 * DAY));
    expect(rec.prediction?.likely).toBe(iso(T0 + 37.5 * DAY));
    expect(rec.inputs.samples).toBe(3);
    expect(rec.inputs.span).toEqual({ from: iso(T0), to: iso(T0 + 25 * DAY) });
    // Shared §4 formatter: the range crosses Chicago day boundaries
    // (and its earliest bound precedes issuance), so it renders in
    // calendar form — never a negative or relative-day string.
    expect(rec.prediction?.rendered).toBe("between Wednesday evening and this evening");
  });

  test("several findings at one instant are one arrival", () => {
    const recs = buildArrivalRecords({
      ...baseInput,
      detectorFindings: [
        planChange(T0),
        planChange(T0 + 10 * DAY),
        planChange(T0 + 25 * DAY),
        planChange(T0 + 25 * DAY, { window: "monthly", key: "plan-change:svc:m2:limit:1->2:x" }),
      ],
    });
    expect(recs).toHaveLength(1);
    expect(recs[0].inputs.samples).toBe(3);
    expect(recs[0].prediction?.earliest).toBe(iso(T0 + 35 * DAY));
  });

  test("one interval is insufficient_history, never a range", () => {
    const recs = buildArrivalRecords({
      ...baseInput,
      detectorFindings: [planChange(T0), planChange(T0 + 10 * DAY)],
    });
    expect(recs).toHaveLength(1);
    expect(recs[0].state).toBe("insufficient_history");
    expect(recs[0].prediction).toBeNull();
    expect(recs[0].state_reason).toContain("sample of one");
  });

  test("identical intervals cannot bound a range (earliest == latest is refused)", () => {
    const recs = buildArrivalRecords({
      ...baseInput,
      detectorFindings: [planChange(T0), planChange(T0 + 10 * DAY), planChange(T0 + 20 * DAY)],
    });
    expect(recs).toHaveLength(1);
    expect(recs[0].state).toBe("insufficient_history");
    expect(recs[0].prediction).toBeNull();
    expect(recs[0].state_reason).toContain("no measured spread");
  });
});

describe("vendor_cadence basis", () => {
  const cadence = {
    periodMs: 30 * DAY,
    spreadMs: 3 * DAY,
    lastArrivalAt: iso(T0),
    recordedIn: "issue #13260 test fixture",
    samples: 4,
  };

  test("a recorded cadence with spread bounds the next occurrence", () => {
    const recs = buildArrivalRecords({
      ...baseInput,
      issuedAt: iso(T0 + 10 * DAY),
      vendorCadence: cadence,
    });
    expect(recs).toHaveLength(1);
    const rec = recs[0];
    expect(rec.state).toBe("predicted");
    expect(rec.basis.name).toBe("vendor_cadence");
    expect(rec.basis.detail).toContain("issue #13260 test fixture");
    expect(rec.prediction?.earliest).toBe(iso(T0 + 27 * DAY));
    expect(rec.prediction?.latest).toBe(iso(T0 + 33 * DAY));
    expect(rec.prediction?.likely).toBe(iso(T0 + 30 * DAY));
  });

  test("a stale anchor rolls forward to the first unclosed occurrence", () => {
    const recs = buildArrivalRecords({
      ...baseInput,
      issuedAt: iso(T0 + 70 * DAY),
      vendorCadence: cadence,
    });
    expect(recs).toHaveLength(1);
    // Occurrences at 30d (window 27-33), 60d (57-63, closed by 70d),
    // 90d (87-93) -> the third is the first unclosed one.
    expect(recs[0].prediction?.earliest).toBe(iso(T0 + 87 * DAY));
    expect(recs[0].prediction?.latest).toBe(iso(T0 + 93 * DAY));
  });

  test("a recorded period with no spread is a point date and is refused", () => {
    const recs = buildArrivalRecords({
      ...baseInput,
      vendorCadence: { ...cadence, spreadMs: 0 },
    });
    expect(recs).toHaveLength(1);
    expect(recs[0].state).toBe("insufficient_history");
    expect(recs[0].prediction).toBeNull();
    expect(recs[0].state_reason).toContain("point date");
  });
});

describe("both bases", () => {
  test("one record per basis, with distinct ids — bases are never blended", () => {
    const recs = buildArrivalRecords({
      ...baseInput,
      detectorFindings: [planChange(T0), planChange(T0 + 10 * DAY), planChange(T0 + 25 * DAY)],
      vendorCadence: {
        periodMs: 30 * DAY,
        spreadMs: 3 * DAY,
        lastArrivalAt: iso(T0),
        recordedIn: "fixture",
        samples: 4,
      },
    });
    expect(recs).toHaveLength(2);
    expect(recs.map((r) => r.basis.name).sort()).toEqual(["detector_history", "vendor_cadence"]);
    expect(new Set(recs.map((r) => r.id)).size).toBe(2);
    expect(recs.every((r) => r.state === "predicted")).toBe(true);
  });
});

describe("validateArrivalRecord — the no-speculative-dates guard", () => {
  const predicted = (): ArrivalPredictionRecord =>
    buildArrivalRecords({
      ...baseInput,
      detectorFindings: [planChange(T0), planChange(T0 + 10 * DAY), planChange(T0 + 25 * DAY)],
    })[0];

  test("a predicted record with basis none is refused", () => {
    const rec = predicted();
    rec.basis = { name: "none", detail: "forged" };
    expect(() => validateArrivalRecord(rec)).toThrow("without a basis is never published");
  });

  test("a point date (earliest == latest) is refused", () => {
    const rec = predicted();
    rec.prediction = { ...rec.prediction!, latest: rec.prediction!.earliest };
    expect(() => validateArrivalRecord(rec)).toThrow("earliest < latest");
  });

  test("an honesty-state record carrying a range is refused", () => {
    const rec = buildArrivalRecords(baseInput)[0];
    rec.prediction = { earliest: iso(T0), latest: iso(T0 + DAY), rendered: "forged" };
    expect(() => validateArrivalRecord(rec)).toThrow("speculative date");
  });

  test("every builder-emitted record validates", () => {
    for (const rec of buildArrivalRecords(baseInput)) expect(() => validateArrivalRecord(rec)).not.toThrow();
    expect(() => validateArrivalRecord(predicted())).not.toThrow();
  });
});

describe("renderPredictionRange — the shared §4 formatter (#13408)", () => {
  // The arrival kind stores the one shared formatter's renderings
  // (quota-downtime.ts); these pin the forms for arrival-shaped
  // ranges so the kinds cannot drift apart silently. Oracle:
  // design §4 and quota-downtime.test.ts.
  test("a same-day future range renders in duration form, half-hour rounded", () => {
    expect(renderPredictionRange("2026-10-01T16:15:00.000Z", "2026-10-01T19:45:00.000Z", "2026-10-01T14:00:00.000Z")).toBe(
      "between 2.5 and 6 hours from now",
    );
  });

  test("a multi-day range crossing Chicago day boundaries renders in calendar form", () => {
    expect(renderPredictionRange(iso(T0 + 2.5 * DAY), iso(T0 + 5.7 * DAY), iso(T0))).toBe(
      "between Saturday morning and Tuesday morning",
    );
  });

  test("a sub-48h range crossing a Chicago day boundary renders in calendar form", () => {
    expect(renderPredictionRange(iso(T0 + 4 * HOUR), iso(T0 + 6.4 * HOUR), iso(T0))).toBe(
      "between today 11 pm and tomorrow 1 am",
    );
  });
});

describe("scoreArrivalPrediction (§3)", () => {
  // Issued the day after the last observed arrival (day 25):
  // predicted window is day 35 (earliest) to day 40 (latest).
  const predicted = (): ArrivalPredictionRecord =>
    buildArrivalRecords({
      ...baseInput,
      issuedAt: iso(T0 + 26 * DAY),
      detectorFindings: [planChange(T0), planChange(T0 + 10 * DAY), planChange(T0 + 25 * DAY)],
    })[0];

  test("a realization inside the range is a hit with lead time from issuance", () => {
    const s = scoreArrivalPrediction(predicted(), iso(T0 + 38 * DAY));
    expect(s.scored).toBe(true);
    expect(s.hit).toBe(true);
    expect(s.leadTimeMs).toBe(12 * DAY);
  });

  test("a realization outside the range is a miss with no lead time", () => {
    const s = scoreArrivalPrediction(predicted(), iso(T0 + 45 * DAY));
    expect(s.scored).toBe(true);
    expect(s.hit).toBe(false);
    expect(s.leadTimeMs).toBeNull();
  });

  test("no_basis records are never scored", () => {
    const rec = buildArrivalRecords(baseInput)[0];
    const s = scoreArrivalPrediction(rec, iso(T0 + 50 * DAY));
    expect(s.scored).toBe(false);
    expect(s.reason).toContain("not scored");
  });

  test("open predictions are unscored, never assumed", () => {
    const s = scoreArrivalPrediction(predicted(), null);
    expect(s.scored).toBe(false);
  });
});
