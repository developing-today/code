import { describe, expect, test } from "bun:test";
import {
  buildResetDurationLedgerRows,
  deriveResetDurationRecords,
  resetDurationSubjectKey,
  type ResetObservation,
} from "./quota-reset-duration.ts";
import { PREDICTION_LEDGER_SOURCE } from "./quota-downtime.ts";
import type { QuotaForecastFile, QuotaForecastPool, QuotaForecastWindow } from "./quota-forecast.ts";

// Oracle: the C4 design (doc/2026-11-13T15-12-00Z_design_reporting-
// architecture-event-driven/c4-forecasting-design.md §1 + §4 + §7,
// PR #13268), #13258's acceptance criterion 2, and Lane WC's
// prepared scope on #13258 (comment 7101533381). A reset_duration
// record bounds the REOPENING instant of a walled/near-wall
// window: anchored on the published resetsAt when there is one,
// ranged by the reset jitter observed over completed reset
// periods (realized minus published, the realized instant
// estimated at the midpoint of the bracketing readings); with
// no published reset, projected one observed cadence interval
// past the last published reset. earliest == latest is refused
// (the range widens to the observation resolution), and a
// subject that cannot be bounded publishes an honesty state.

const AS_OF = "2026-11-23T12:00:00.000Z"; // 06:00 in America/Chicago
const H = 3_600_000;
const M = 60_000;
const at = (hours: number) => new Date(Date.parse(AS_OF) + hours * H).toISOString();
const atMin = (mins: number) => new Date(Date.parse(AS_OF) + mins * M).toISOString();

function fwindow(over: Partial<QuotaForecastWindow> = {}): QuotaForecastWindow {
  return {
    name: "5-hour",
    state: "walled",
    ratePtsPerHour: null,
    samples: 40,
    projectedWallAt: null,
    durationUntilWallMs: 0,
    wallsBeforeReset: true,
    pctNow: 100,
    measuredAt: AS_OF,
    resetsAt: at(6),
    ...over,
  };
}

function fpool(windows: QuotaForecastWindow[], over: Partial<QuotaForecastPool> = {}): QuotaForecastPool {
  return { providerId: "claude", serviceId: "_claude", plan: null, windows, ...over };
}

function fforecast(pools: Record<string, QuotaForecastPool[]>): QuotaForecastFile {
  return { version: 1, generatedAt: AS_OF, source: "gather-quota", pools };
}

/**
 * Synthetic snapshot history: readings every 30 minutes from
 * fromH to toH. Each reading carries the published reset of the
 * period it falls in; period i's TRUE reset (where the readings
 * start carrying the next period's published reset) is
 * published[i] + trueOffsetsMin[i]. The last period is open.
 * With true resets on the quarter hour, the bracketing readings
 * sit exactly 15 minutes either side, so the midpoint estimate
 * recovers the true jitter exactly.
 */
function history(publishedH: number[], trueOffsetsMin: number[], fromH: number, toH: number): ResetObservation[] {
  const obs: ResetObservation[] = [];
  for (let t = fromH; t <= toH; t += 0.5) {
    let idx = 0;
    for (let i = 0; i < publishedH.length - 1; i++) {
      if (Date.parse(at(t)) >= Date.parse(at(publishedH[i])) + trueOffsetsMin[i] * M) idx = i + 1;
    }
    obs.push({ ts: at(t), resetsAt: at(publishedH[idx]) });
  }
  return obs;
}

// Four completed periods (5h cadence), jitter +15/-15/+15/-15
// minutes, current period published at +6h.
const JITTERED = history([-20, -15, -10, -5, 6], [15, -15, 15, -15], -22, -0.5);
const KEY = resetDurationSubjectKey("claude", "5-hour");

describe("deriveResetDurationRecords — published reset, ranged by observed jitter", () => {
  const records = deriveResetDurationRecords(
    fforecast({ claude: [fpool([fwindow()])] }),
    new Map([[KEY, JITTERED]]),
    new Map([[KEY, "c4:reset_duration:claude:5-hour:1"]]),
  );

  test("one predicted record with the design's shape", () => {
    expect(records).toHaveLength(1);
    const r = records[0];
    expect(r.record_version).toBe(1);
    expect(r.kind).toBe("reset_duration");
    expect(r.subject).toEqual({ pool: "claude", window: "5-hour" });
    expect(r.id).toBe(`c4:reset_duration:claude:5-hour:${Date.parse(AS_OF)}`);
    expect(r.issued_at).toBe(AS_OF);
    expect(r.basis.name).toBe("reset_cadence");
    expect(r.basis.detail).toContain("reopening anchored on the published resetsAt");
    expect(r.basis.detail).toContain("already walled at issuance");
    expect(r.methodology_version).toBe("c4-meth:3");
    expect(r.state).toBe("predicted");
    expect(r.state_reason).toBeNull();
    expect(r.supersedes).toBe("c4:reset_duration:claude:5-hour:1");
  });

  test("the range is the published reset widened by the jitter spread", () => {
    const p = records[0].prediction;
    expect(p).not.toBeNull();
    expect(p?.earliest).toBe(atMin(360 - 15));
    expect(p?.latest).toBe(atMin(360 + 15));
  });

  test("rendered is the §4 duration form, stored at issuance", () => {
    expect(records[0].prediction?.rendered).toBe("between 6 and 6.5 hours from now");
  });

  test("inputs name the rename-joined series and the history behind the basis", () => {
    const r = records[0];
    expect(r.inputs.series).toBe("claude"); // serviceId "_claude" folded
    expect(r.inputs.samples).toBe(JITTERED.length);
    expect(r.inputs.span).toEqual({ from: at(-22), to: at(-0.5) });
    expect(r.inputs.adjustments).toEqual([]);
  });
});

describe("deriveResetDurationRecords — degenerate jitter widens to the observation resolution", () => {
  test("zero observed spread still yields a strict range", () => {
    // Published resets on the quarter hour, realized exactly on
    // them: every midpoint estimate lands on the published
    // instant, so the raw spread is zero.
    const obs = history([-19.75, -14.75, -9.75, -4.75, 6], [0, 0, 0, 0], -22, -0.5);
    const records = deriveResetDurationRecords(
      fforecast({ claude: [fpool([fwindow()])] }),
      new Map([[KEY, obs]]),
    );
    expect(records).toHaveLength(1);
    const p = records[0].prediction;
    expect(records[0].state).toBe("predicted");
    // Readings every 30 minutes resolve a reset to ±15 minutes.
    expect(p?.earliest).toBe(atMin(360 - 15));
    expect(p?.latest).toBe(atMin(360 + 15));
    expect(p?.earliest).not.toBe(p?.latest);
  });
});

describe("deriveResetDurationRecords — subjects", () => {
  test("only walled and near-wall windows publish; the projected wall rides in the detail", () => {
    const windows = [
      fwindow({ name: "5-hour", state: "projected", projectedWallAt: Date.parse(at(2)), durationUntilWallMs: 2 * H, pctNow: 92 }),
      fwindow({ name: "7d", state: "projected", wallsBeforeReset: false, projectedWallAt: Date.parse(at(30)) }),
      fwindow({ name: "monthly", state: "no-wall-before-reset", wallsBeforeReset: false }),
      fwindow({ name: "api", state: "insufficient-history", wallsBeforeReset: null, pctNow: null }),
    ];
    const records = deriveResetDurationRecords(
      fforecast({ claude: [fpool(windows)] }),
      new Map([[KEY, JITTERED]]),
    );
    expect(records).toHaveLength(1);
    expect(records[0].subject.window).toBe("5-hour");
    expect(records[0].state).toBe("predicted");
    expect(records[0].basis.detail).toContain(`the projected wall is ${at(2)}`);
  });
});

describe("deriveResetDurationRecords — honesty states", () => {
  test("no snapshot history at all is no_basis, never an invented range", () => {
    const records = deriveResetDurationRecords(fforecast({ claude: [fpool([fwindow()])] }), new Map());
    expect(records).toHaveLength(1);
    expect(records[0].state).toBe("no_basis");
    expect(records[0].prediction).toBeNull();
    expect(records[0].state_reason).toContain("no snapshot history");
    expect(records[0].id).toContain("@honesty:");
  });

  test("a single observed period measures no jitter: insufficient_history", () => {
    const obs = history([6], [], -4, -0.5);
    const records = deriveResetDurationRecords(
      fforecast({ claude: [fpool([fwindow()])] }),
      new Map([[KEY, obs]]),
    );
    expect(records).toHaveLength(1);
    expect(records[0].state).toBe("insufficient_history");
    expect(records[0].prediction).toBeNull();
    expect(records[0].state_reason).toContain("no completed reset period");
  });

  test("a published reset already in the past is a stale row, not a prediction", () => {
    const records = deriveResetDurationRecords(
      fforecast({ claude: [fpool([fwindow({ resetsAt: at(-1) })])] }),
      new Map([[KEY, JITTERED]]),
    );
    expect(records).toHaveLength(1);
    expect(records[0].state).toBe("insufficient_history");
    expect(records[0].state_reason).toContain("stale");
  });
});

describe("deriveResetDurationRecords — cadence projection when no reset is published", () => {
  test("the next reset is projected one cadence interval past the last published reset", () => {
    // Three completed 5h periods, jitter +15/-15/+15; the open
    // period's published reset is the tick instant itself, so no
    // future anchor exists anywhere.
    const obs = history([-15, -10, -5, 0], [15, -15, 15], -17, -0.5);
    const records = deriveResetDurationRecords(
      fforecast({ claude: [fpool([fwindow({ resetsAt: null })])] }),
      new Map([[KEY, obs]]),
    );
    expect(records).toHaveLength(1);
    const r = records[0];
    expect(r.state).toBe("predicted");
    expect(r.basis.detail).toContain("no reset is published for the current window");
    expect(r.prediction?.earliest).toBe(atMin(300 - 15));
    expect(r.prediction?.latest).toBe(atMin(300 + 15));
  });

  test("a cadence that places the next reset in the past is stale history, not a prediction", () => {
    const obs = history([-50, -45, -40], [0, 0], -52, -41);
    const records = deriveResetDurationRecords(
      fforecast({ claude: [fpool([fwindow({ resetsAt: null })])] }),
      new Map([[KEY, obs]]),
    );
    expect(records).toHaveLength(1);
    expect(records[0].state).toBe("insufficient_history");
    expect(records[0].state_reason).toContain("places the next reset in the past");
  });
});

describe("deriveResetDurationRecords — rendered calendar form", () => {
  test("a range straddling the Chicago midnight renders day names and dayparts", () => {
    // Anchor 06:00Z tomorrow = 00:00 Chicago; jitter ±15m puts
    // the bounds at 23:45 Chicago today and 00:15 tomorrow.
    const records = deriveResetDurationRecords(
      fforecast({ claude: [fpool([fwindow({ resetsAt: at(18) })])] }),
      new Map([[KEY, JITTERED]]),
    );
    expect(records).toHaveLength(1);
    expect(records[0].prediction?.earliest).toBe(atMin(18 * 60 - 15));
    expect(records[0].prediction?.latest).toBe(atMin(18 * 60 + 15));
    expect(records[0].prediction?.rendered).toBe("between today 11 pm and tomorrow 12 am");
  });
});

describe("buildResetDurationLedgerRows", () => {
  test("rows mirror under the C4 source with the record verbatim; honesty rows carry no reset", () => {
    const predicted = deriveResetDurationRecords(
      fforecast({ claude: [fpool([fwindow()])] }),
      new Map([[KEY, JITTERED]]),
    );
    const honesty = deriveResetDurationRecords(fforecast({ claude: [fpool([fwindow()])] }), new Map());
    const rows = buildResetDurationLedgerRows([...predicted, ...honesty], AS_OF);
    expect(rows).toHaveLength(2);
    expect(rows[0].source).toBe(PREDICTION_LEDGER_SOURCE);
    expect(rows[0].source).toBe("quota-prediction");
    expect(rows[0].id).toBe(predicted[0].id);
    expect(rows[0].ts).toBe(AS_OF);
    expect(rows[0].resets).toBe(predicted[0].prediction?.latest ?? null);
    expect((rows[0].payload as { kind?: string }).kind).toBe("reset_duration");
    expect(rows[1].resets).toBeNull();
  });
});

describe("deriveResetDurationRecords — null-reset readings across a rollover", () => {
  test("null readings past the published reset do not inflate the jitter estimate", () => {
    // Period 1 (published -10h) truly resets on schedule; the
    // provider then stops publishing resetsAt for two readings
    // and resumes with period 2's reset (-5h) at -7h. The
    // bracketing readings for period 1's rollover are -10.5h
    // (last carrying -10h) and -7h (first carrying -5h):
    // realized estimate -8.75h, jitter +75m. Extending the
    // period with the null readings (-9h, -8h) would read the
    // publication gap as +150m of jitter instead.
    const obs: ResetObservation[] = [
      { ts: at(-11), resetsAt: at(-10) },
      { ts: at(-10.5), resetsAt: at(-10) },
      { ts: at(-9), resetsAt: null },
      { ts: at(-8), resetsAt: null },
      { ts: at(-7), resetsAt: at(-5) },
      { ts: at(-6), resetsAt: at(-5) },
      { ts: at(-5.5), resetsAt: at(-5) },
      { ts: at(-4.5), resetsAt: at(0) },
      { ts: at(-4), resetsAt: at(0) },
      { ts: at(-3), resetsAt: at(0) },
    ];
    const records = deriveResetDurationRecords(
      fforecast({ claude: [fpool([fwindow()])] }),
      new Map([[KEY, obs]]),
    );
    expect(records).toHaveLength(1);
    const r = records[0];
    expect(r.state).toBe("predicted");
    expect(r.basis.detail).toContain("realized minus published: +0m to +75m");
    expect(r.prediction?.earliest).toBe(at(6));
    expect(r.prediction?.latest).toBe(atMin(360 + 75));
  });
});
