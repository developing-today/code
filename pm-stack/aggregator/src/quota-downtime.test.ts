import { describe, expect, test } from "bun:test";
import {
  buildDowntimeLedgerRows,
  deriveDowntimeWindowRecords,
  renderPredictionRange,
  C4_METHODOLOGY_VERSION,
  PREDICTION_LEDGER_SOURCE,
} from "./quota-downtime.ts";
import type { QuotaForecastFile, QuotaForecastPool, QuotaForecastWindow } from "./quota-forecast.ts";

// Oracle: the C4 design (doc/2026-11-13T15-12-00Z_design_reporting-
// architecture-event-driven/c4-forecasting-design.md §1 + §7, PR
// #13268) over the #12695 forecast product, and #13257 acceptance
// criteria 1 and 3. A downtime_window record is the design's
// prediction record: the all-walled interval intersected from the
// per-pool [wall, reset) intervals, earliest = the last member
// wall, latest = the earliest member reset, basis named, rendered
// stored; a window with no defensible basis is the published
// insufficient_history honesty state, and a defensible no-window
// verdict publishes NO record (absence is the scored signal).

const AS_OF = "2026-11-23T12:00:00.000Z"; // 06:00 in America/Chicago
const H = 3_600_000;
const at = (hours: number) => new Date(Date.parse(AS_OF) + hours * H).toISOString();

function fwindow(over: Partial<QuotaForecastWindow> = {}): QuotaForecastWindow {
  return {
    name: "weekly",
    state: "projected",
    ratePtsPerHour: 5,
    samples: 12,
    projectedWallAt: Date.parse(at(2)),
    durationUntilWallMs: 2 * H,
    wallsBeforeReset: true,
    pctNow: 90,
    measuredAt: AS_OF,
    resetsAt: at(10),
    ...over,
  };
}

function pool(providerId: string, windows: QuotaForecastWindow[], serviceId = `${providerId}-svc`): QuotaForecastPool {
  return { providerId, serviceId, plan: null, windows };
}

function forecast(pools: Record<string, QuotaForecastPool[]>): QuotaForecastFile {
  return { version: 1, generatedAt: AS_OF, source: "gather-quota", pools };
}

describe("deriveDowntimeWindowRecords", () => {
  test("overlapping walls publish one design-shaped predicted record", () => {
    const records = deriveDowntimeWindowRecords(
      forecast({
        a: [pool("a", [fwindow({ projectedWallAt: Date.parse(at(2)), resetsAt: at(10) })])],
        b: [pool("b", [fwindow({ projectedWallAt: Date.parse(at(4)), resetsAt: at(8) })])],
      }),
    );
    expect(records).toHaveLength(1);
    const r = records[0];
    expect(r.record_version).toBe(1);
    expect(r.kind).toBe("downtime_window");
    expect(r.id).toBe(`c4:downtime_window:a+b@${Date.parse(at(4))}:${Date.parse(AS_OF)}`);
    expect(r.subject.pools).toEqual(["a", "b"]);
    expect(r.issued_at).toBe(AS_OF);
    expect(r.basis.name).toBe("since_last_reset");
    expect(r.inputs.samples).toBe(24);
    expect(r.inputs.adjustments).toEqual([]);
    expect(r.methodology_version).toBe(C4_METHODOLOGY_VERSION);
    expect(r.state).toBe("predicted");
    expect(r.state_reason).toBeNull();
    expect(r.supersedes).toBeNull();
    // earliest = the last member wall; latest = the earliest
    // member reset (the first reopening ends the window).
    expect(r.prediction?.earliest).toBe(at(4));
    expect(r.prediction?.latest).toBe(at(8));
    // 10:00 -> 14:00 Chicago, same day: duration form (§4).
    expect(r.prediction?.rendered).toBe("between 4 and 8 hours from now");
  });

  test("supersedes links the previous record for the same pool set", () => {
    const prev = new Map([["a+b", "c4:downtime_window:a+b@111:222"]]);
    const records = deriveDowntimeWindowRecords(
      forecast({
        a: [pool("a", [fwindow()])],
        b: [pool("b", [fwindow()])],
      }),
      prev,
    );
    expect(records[0].supersedes).toBe("c4:downtime_window:a+b@111:222");
  });

  test("pools already walled at the tick: the window is in progress, rendered in calendar form", () => {
    const records = deriveDowntimeWindowRecords(
      forecast({
        a: [
          pool("a", [
            fwindow({ state: "walled", projectedWallAt: null, durationUntilWallMs: 0, pctNow: 100, resetsAt: at(6) }),
          ]),
        ],
        b: [
          pool("b", [
            fwindow({ state: "walled", projectedWallAt: null, durationUntilWallMs: 0, pctNow: 100, resetsAt: at(9) }),
          ]),
        ],
      }),
    );
    expect(records).toHaveLength(1);
    expect(records[0].prediction?.earliest).toBe(AS_OF);
    expect(records[0].prediction?.latest).toBe(at(6));
    // 06:00 -> 12:00 Chicago, start not in the future: dayparts.
    expect(records[0].prediction?.rendered).toBe("between this morning and this afternoon");
  });

  test("a pool that never walls: the defensible no-window verdict publishes NO record", () => {
    const records = deriveDowntimeWindowRecords(
      forecast({
        a: [pool("a", [fwindow()])],
        b: [
          pool("b", [
            fwindow({ state: "no-wall-before-reset", projectedWallAt: null, wallsBeforeReset: false, pctNow: 40 }),
          ]),
        ],
      }),
    );
    expect(records).toEqual([]);
  });

  test("walls that never coincide publish no record either", () => {
    const records = deriveDowntimeWindowRecords(
      forecast({
        a: [pool("a", [fwindow({ projectedWallAt: Date.parse(at(1)), resetsAt: at(3) })])],
        b: [pool("b", [fwindow({ projectedWallAt: Date.parse(at(5)), resetsAt: at(9) })])],
      }),
    );
    expect(records).toEqual([]);
  });

  test("an unprojectable pool with no known overlap publishes the honesty state, never a range", () => {
    const records = deriveDowntimeWindowRecords(
      forecast({
        a: [pool("a", [fwindow({ projectedWallAt: Date.parse(at(1)), resetsAt: at(3) })])],
        b: [
          pool("b", [
            fwindow({
              state: "insufficient-history",
              projectedWallAt: null,
              wallsBeforeReset: null,
              pctNow: null,
              reason: "fewer than two same-period readings spanning 10 minutes",
            }),
          ]),
        ],
      }),
    );
    expect(records).toHaveLength(1);
    expect(records[0].state).toBe("insufficient_history");
    expect(records[0].prediction).toBeNull();
    expect(records[0].state_reason).toContain("b");
  });

  test("unknown windows cannot remove a known overlap: predicted, unknowns named in the basis detail", () => {
    const records = deriveDowntimeWindowRecords(
      forecast({
        a: [
          pool("a", [
            fwindow({ projectedWallAt: Date.parse(at(2)), resetsAt: at(10) }),
            fwindow({
              name: "5-hour",
              state: "insufficient-history",
              projectedWallAt: null,
              wallsBeforeReset: null,
              pctNow: null,
              resetsAt: at(5),
              reason: "fewer than two same-period readings spanning 10 minutes",
            }),
          ]),
        ],
        b: [pool("b", [fwindow({ projectedWallAt: Date.parse(at(3)), resetsAt: at(9) })])],
      }),
    );
    expect(records).toHaveLength(1);
    expect(records[0].state).toBe("predicted");
    expect(records[0].prediction?.earliest).toBe(at(3));
    expect(records[0].basis.detail).toContain("a");
  });

  test("a provider is down while ANY of its pool rows' windows is down", () => {
    const records = deriveDowntimeWindowRecords(
      forecast({
        a: [
          pool("a", [fwindow({ name: "5-hour", projectedWallAt: Date.parse(at(1)), resetsAt: at(2) })], "a-one"),
          pool(
            "a",
            [fwindow({ state: "no-wall-before-reset", projectedWallAt: null, wallsBeforeReset: false, pctNow: 30 })],
            "a-two",
          ),
        ],
        b: [pool("b", [fwindow({ projectedWallAt: Date.parse(at(1.5)), resetsAt: at(5) })])],
      }),
    );
    expect(records).toHaveLength(1);
    expect(records[0].prediction?.earliest).toBe(at(1.5));
    expect(records[0].prediction?.latest).toBe(at(2));
  });

  test("two disjoint overlaps publish two records with distinct ids", () => {
    const records = deriveDowntimeWindowRecords(
      forecast({
        a: [pool("a", [fwindow({ projectedWallAt: Date.parse(at(1)), resetsAt: at(9) })])],
        b: [
          pool("b", [
            fwindow({ name: "5-hour", projectedWallAt: Date.parse(at(2)), resetsAt: at(3) }),
            fwindow({ name: "weekly", projectedWallAt: Date.parse(at(5)), resetsAt: at(6) }),
          ]),
        ],
      }),
    );
    expect(records.map((r) => [r.prediction?.earliest, r.prediction?.latest])).toEqual([
      [at(2), at(3)],
      [at(5), at(6)],
    ]);
    expect(new Set(records.map((r) => r.id)).size).toBe(2);
  });

  test("no pools at all publishes the honesty state, never silence", () => {
    const records = deriveDowntimeWindowRecords(forecast({}));
    expect(records).toHaveLength(1);
    expect(records[0].state).toBe("insufficient_history");
    expect(records[0].state_reason).toContain("no pools");
  });

  test("an all-walled overlap no reset bounds is an honesty state, not an unbounded prediction", () => {
    const records = deriveDowntimeWindowRecords(
      forecast({
        a: [
          pool("a", [
            fwindow({ state: "walled", projectedWallAt: null, durationUntilWallMs: 0, pctNow: 100, resetsAt: null }),
          ]),
        ],
        b: [
          pool("b", [
            fwindow({ state: "walled", projectedWallAt: null, durationUntilWallMs: 0, pctNow: 100, resetsAt: null }),
          ]),
        ],
      }),
    );
    expect(records).toHaveLength(1);
    expect(records[0].state).toBe("insufficient_history");
    expect(records[0].prediction).toBeNull();
    expect(records[0].state_reason).toContain("cannot be bounded");
  });
});

describe("renderPredictionRange", () => {
  test("a range crossing a Chicago day boundary renders in calendar form", () => {
    // Mon 06:00 CST -> Tue 12:00 CST / Wed 04:00 CST.
    const rendered = renderPredictionRange(at(30), at(46), AS_OF);
    expect(rendered.startsWith("between ")).toBe(true);
    expect(rendered).toContain("tomorrow");
  });

  test("a same-day future range renders in duration form, half-hour rounded", () => {
    expect(renderPredictionRange(at(2.25), at(5.75), AS_OF)).toBe("between 2.5 and 6 hours from now");
  });

  test("a 48h-or-wider future range renders in day form, not calendar form (design §4; #13357)", () => {
    // Wed 18:00 CST -> Sat 06:00 CST: 60h wide, crosses day boundaries.
    // The day-form branch was unreachable: the calendar check ran first.
    expect(renderPredictionRange(at(60), at(120), AS_OF)).toBe("2.5 to 5 days");
  });

  test("a 48h-wide range on the boundary renders in day form (design §4; #13357)", () => {
    // Wed 06:00 CST -> Fri 06:00 CST: exactly 48h wide, both bounds exact tenths of a day.
    expect(renderPredictionRange(at(48), at(96), AS_OF)).toBe("2 to 4 days");
  });

  test("a 48h-or-wider range with non-tenth-day bounds renders compound (design §4; #13357)", () => {
    // 52h = 2 days 4 hours, 124h = 5 days 4 hours; neither is an exact tenth of a day.
    expect(renderPredictionRange(at(52), at(124), AS_OF)).toBe("2 days 4 hours to up to 5 days 4 hours");
  });

  test("a 48h-or-wider range already in progress still renders calendar form (#13357)", () => {
    // The day form has no "from now" anchor, so a past start stays calendar.
    const rendered = renderPredictionRange(at(-2), at(60), AS_OF);
    expect(rendered.startsWith("between ")).toBe(true);
  });

  test("a far-out same-weekday range names both calendar dates (#13536)", () => {
    // The issue's example: a vendor-cadence arrival Sep 22 → Oct 6,
    // both Tuesdays, issued Sep 1 — "between Tuesday afternoon and
    // Tuesday afternoon" names no week.
    expect(
      renderPredictionRange("2026-09-22T19:00:00.000Z", "2026-10-06T19:00:00.000Z", "2026-09-01T17:00:00.000Z"),
    ).toBe("between Tuesday afternoon, Sep 22 and Tuesday afternoon, Oct 6");
  });

  test("a near-horizon same-weekday pair keeps its short form", () => {
    // T0+2.5d → T0+5.7d crosses Chicago day boundaries but stays
    // inside one week: no date suffix.
    expect(renderPredictionRange(at(2.5 * 24), at(5.7 * 24), AS_OF)).not.toContain(",");
  });

  test("a bound exactly seven days out is dated, a bound just inside is not", () => {
    const issued = "2026-10-01T00:00:00.000Z";
    const weekOut = "2026-10-08T19:00:00.000Z"; // Thursday afternoon, 7.8d out
    const justInside = "2026-10-07T19:00:00.000Z"; // Wednesday afternoon, 6.8d out
    expect(renderPredictionRange(justInside, weekOut, issued)).toBe(
      "between Wednesday afternoon and Thursday afternoon, Oct 8",
    );
    expect(renderPredictionRange(at(30), at(46), AS_OF)).not.toContain(",");
  });
});

describe("buildDowntimeLedgerRows", () => {
  test("rows mirror the records under the C4 source, payload verbatim, resets = window end", () => {
    const records = deriveDowntimeWindowRecords(
      forecast({
        a: [pool("a", [fwindow({ projectedWallAt: Date.parse(at(2)), resetsAt: at(10) })])],
        b: [pool("b", [fwindow({ projectedWallAt: Date.parse(at(4)), resetsAt: at(8) })])],
      }),
    );
    const rows = buildDowntimeLedgerRows(records, AS_OF);
    expect(rows).toHaveLength(1);
    expect(rows[0].source).toBe(PREDICTION_LEDGER_SOURCE);
    expect(rows[0].source).toBe("quota-prediction");
    expect(rows[0].id).toBe(records[0].id);
    expect(rows[0].resets).toBe(at(8));
    expect(rows[0].payload).toEqual({ ...records[0] });
  });

  test("honesty records mirror too, with no resets", () => {
    const records = deriveDowntimeWindowRecords(forecast({}));
    const rows = buildDowntimeLedgerRows(records, AS_OF);
    expect(rows).toHaveLength(1);
    expect(rows[0].resets).toBeNull();
    expect((rows[0].payload as { state: string }).state).toBe("insufficient_history");
  });
});
