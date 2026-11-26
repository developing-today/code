import { describe, expect, test } from "bun:test";
import {
  anomalyIncidentInput,
  canonicalWindowForMetric,
  emptyAnomalyState,
  evaluateBurnSpikes,
  observeFieldSet,
  observeLimitRecord,
  poolWindowFields,
  runAnomalyReplay,
  runAnomalyTick,
  shapeFieldsFromEntry,
  shapeObservationFromEntry,
  type FieldSetState,
} from "./quota-anomaly.ts";
import type { PctPoint } from "./quota-analytics.ts";
import type { ForecastPoolDef } from "./quota-forecast-accuracy.ts";
import type { QuotaCurrentEntry, QuotaCurrentWindow, SnapshotRecord } from "./model.ts";

// Oracle: the Stage-3 design §6 / decision D6 (#12699): a burn
// spike is realized burn outside the pool's OWN forecast band
// (forecast rate + the p99 of its realized residuals — the
// S2-measured spread), never a global threshold; schema drift is a
// parsed-shape diff against the recorded shape (additions at first
// occurrence, removals after the observation stream's sustain) or
// a parse falling into the fail-closed unparseable state; a plan
// change is a limit change between consecutive non-null records or
// a canonical window kind appearing/disappearing for a pool. A
// detector that cannot evaluate reports "unevaluable" with a
// reason — absent data is never an all-clear.

const HOUR = 3_600_000;
const POOL: ForecastPoolDef = { serviceId: "meta", providerId: "_muse" };
const RESET_FAR = "2026-11-08T00:00:00.000Z";

/** Percent points every 15 minutes from 2026-11-01, one reset
 * period (resetsAt fixed), pct shaped by pctAt(elapsedHours). */
function series(pctAt: (h: number) => number, hours: number): PctPoint[] {
  const start = Date.parse("2026-11-01T00:00:00.000Z");
  const points: PctPoint[] = [];
  for (let k = 0; k <= hours * 4; k++) {
    const h = k / 4;
    points.push({ ts: start + k * 900_000, pct: pctAt(h), resetsAt: RESET_FAR });
  }
  return points;
}

describe("burn spike (oracle: design §6 — outside the pool's own forecast band)", () => {
  test("a planted spike hour fires exactly once; the episode does not re-fire while it lasts", () => {
    // Steady 0.5 pts/h, then one hour burning at 20 pts/h, then steady again.
    const points = series((h) => {
      if (h < 48) return 5 + 0.5 * h;
      if (h < 49) return 29 + 20 * (h - 48);
      return 49 + 0.5 * (h - 49);
    }, 96);
    const { findings, evaluation } = evaluateBurnSpikes({ pool: POOL, window: "5h", points });
    expect(evaluation.state).toBe("fired");
    expect(findings.length).toBe(1);
    expect(findings[0]?.detector).toBe("burn-spike");
    expect(findings[0]?.detail.realizedBurnPtsPerHour as number).toBeGreaterThan(5);
    expect(findings[0]?.detail.pctAtOrigin as number).toBeLessThan(100);
  });

  test("a clean steady series stays silent — and the silence is a measured one", () => {
    const points = series((h) => 5 + 0.5 * h, 96);
    const { findings, evaluation } = evaluateBurnSpikes({ pool: POOL, window: "weekly", points });
    expect(findings).toEqual([]);
    expect(evaluation.state).toBe("silent");
  });

  test("the pool's normal lumpiness sets its own band: a staircase burn stays silent, one outsized step fires", () => {
    // +2 pts instantly every 2h, flat between: the least-squares
    // forecast is the average slope (1/h) while realized hourly
    // burn alternates 0/2 — residuals of ±1 are this pool's norm,
    // so the band (rate + p99 residual = 2) must absorb them.
    const staircase = (h: number): number => 2 + 2 * Math.floor(h / 2);
    const calm = evaluateBurnSpikes({ pool: POOL, window: "5h", points: series(staircase, 90) });
    expect(calm.findings).toEqual([]);
    expect(calm.evaluation.state).toBe("silent");
    // One step of +14 instead of +2 (at h=60) is outside even that band.
    const spiked = series((h) => (h < 60 ? staircase(h) : staircase(h) + 12), 90);
    const hot = evaluateBurnSpikes({ pool: POOL, window: "5h", points: spiked });
    expect(hot.findings.length).toBe(1);
    expect(hot.evaluation.state).toBe("fired");
  });

  test("a thin series is unevaluable with a named reason — never silent", () => {
    const points = series((h) => 5 + h, 1).slice(0, 3);
    const { findings, evaluation } = evaluateBurnSpikes({ pool: POOL, window: "5h", points });
    expect(findings).toEqual([]);
    expect(evaluation.state).toBe("unevaluable");
    expect(evaluation.reason).toBeTruthy();
  });

  test("readings already at the wall are detections, not spike origins", () => {
    const points = series((h) => Math.min(100, 2 + 2 * h), 60);
    const { findings } = evaluateBurnSpikes({ pool: POOL, window: "5h", points });
    for (const f of findings) expect(f.detail.pctAtOrigin as number).toBeLessThan(100);
  });
});

describe("field-set machine (oracle: design §6 — drift is a finding at first occurrence)", () => {
  const obs = (ts: string, fields: string[] | null, unparseable?: string) => ({ ts, fields, unparseable });

  test("first observation establishes the baseline; it is not a finding and not silence", () => {
    const { events } = observeFieldSet(undefined, obs("2026-11-01T00:00:00Z", ["a", "b"]), 1);
    expect(events).toEqual([{ kind: "baseline-established", fields: ["a", "b"] }]);
  });

  test("an added field fires at first occurrence; a removed field fires after the sustain", () => {
    let state: FieldSetState | undefined;
    state = observeFieldSet(state, obs("t0", ["a", "b"]), 3).state;
    const added = observeFieldSet(state, obs("t1", ["a", "b", "c"]), 3);
    expect(added.events).toEqual([{ kind: "shape-changed", added: ["c"], removed: [], fields: ["a", "b", "c"] }]);
    state = added.state;
    const miss1 = observeFieldSet(state, obs("t2", ["a", "b"]), 3);
    expect(miss1.events).toEqual([]);
    const miss2 = observeFieldSet(miss1.state, obs("t3", ["a", "b"]), 3);
    expect(miss2.events).toEqual([]);
    const miss3 = observeFieldSet(miss2.state, obs("t4", ["a", "b"]), 3);
    expect(miss3.events).toEqual([{ kind: "shape-changed", added: [], removed: ["c"], fields: ["a", "b"] }]);
  });

  test("an unparseable parse fires once per episode; recovery re-arms it", () => {
    let state = observeFieldSet(undefined, obs("t0", ["a"]), 1).state;
    const first = observeFieldSet(state, obs("t1", null, "file-export unparseable: windows missing"), 1);
    expect(first.events).toEqual([{ kind: "unparseable-started", reason: "file-export unparseable: windows missing" }]);
    state = first.state;
    expect(observeFieldSet(state, obs("t2", null, "still bad"), 1).events).toEqual([]);
    state = observeFieldSet(state, obs("t2", null, "still bad"), 1).state;
    state = observeFieldSet(state, obs("t3", ["a"]), 1).state;
    const again = observeFieldSet(state, obs("t4", null, "bad again"), 1);
    expect(again.events.length).toBe(1);
    expect(again.events[0]?.kind).toBe("unparseable-started");
  });

  test("an unparseable first contact is unevaluable; the next parseable read establishes the baseline", () => {
    const first = observeFieldSet(undefined, obs("t0", null, "unparseable"), 1);
    expect(first.events).toEqual([
      { kind: "unevaluable", reason: "first observation is an unparseable parse — no baseline shape recorded yet" },
    ]);
    const second = observeFieldSet(first.state, obs("t1", ["a", "b"]), 1);
    expect(second.events).toEqual([{ kind: "baseline-established", fields: ["a", "b"] }]);
  });
});

describe("entry shape builders (oracle: the gatherer's parse, quota-gather-map.ts)", () => {
  function win(name: string): QuotaCurrentWindow {
    return {
      name,
      value: 10,
      pctOfLimit: 10,
      valueLabel: null,
      windowSeconds: null,
      resetsAt: RESET_FAR,
      measuredAt: "2026-11-01T00:00:00.000Z",
      source: "opencode-quota",
      state: "ok",
    };
  }
  function ent(over: Partial<QuotaCurrentEntry>): QuotaCurrentEntry {
    return {
      providerId: "_claude",
      providerName: "Claude",
      serviceId: "anthropic:claude-pro",
      state: "ok",
      measuredAt: "2026-11-01T00:00:00.000Z",
      source: "opencode-quota",
      windows: [win("weekly")],
      usage: { windows: { weekly: {} }, plan: "pro" },
      ...over,
    } as QuotaCurrentEntry;
  }

  test("shape fields carry usage keys and collapsed window kinds (models are data, not schema)", () => {
    const fields = shapeFieldsFromEntry(ent({ windows: [win("weekly"), win("some-model:5h"), win("other:5-hour")] }));
    expect(fields).toContain("usage:windows");
    expect(fields).toContain("usage:plan");
    expect(fields).toContain("window:weekly");
    expect(fields).toContain("window:5h");
    // "5-hour" is the same canonical window as "5h" — one field,
    // and the raw alias never appears (a rename is not drift).
    expect(fields.filter((f) => f === "window:5h")).toHaveLength(1);
    expect(fields.some((f) => f.includes("5-hour"))).toBe(false);
    expect(fields.some((f) => f.includes("some-model"))).toBe(false);
  });

  test("retained and merely-errored entries are not observations; an unparseable error is", () => {
    expect(shapeObservationFromEntry(ent({ retainedFromFailedTick: "2026-11-01T00:00:00.000Z" }), "t")).toBeNull();
    expect(shapeObservationFromEntry(ent({ state: "error", error: "HTTP 503" }), "t")).toBeNull();
    const unp = shapeObservationFromEntry(
      ent({ state: "error", error: "file-export unparseable: malformed window element" }),
      "t",
    );
    expect(unp?.fields).toBeNull();
    expect(unp?.unparseable).toContain("unparseable");
  });
});

describe("plan change (oracle: design §6 — the denominator and the window set)", () => {
  const rec = (ts: string, limit: number | null, metricId = "storage", serviceId = "turso"): SnapshotRecord =>
    ({
      ts,
      serviceId,
      metricId,
      value: 3,
      unit: "gb",
      limit,
      resetsAt: null,
      source: "api",
      observedBy: "test",
    }) as SnapshotRecord;

  test("a limit change fires once with from/to; null limits neither fire nor re-baseline", () => {
    const limits: Record<string, number> = {};
    expect(observeLimitRecord(limits, rec("t0", 5))).toBeNull();
    expect(observeLimitRecord(limits, rec("t1", 5))).toBeNull();
    const fired = observeLimitRecord(limits, rec("t2", 24));
    expect(fired?.detector).toBe("plan-change");
    expect(fired?.detail).toMatchObject({ from: 5, to: 24 });
    expect(observeLimitRecord(limits, rec("t3", null))).toBeNull();
    expect(observeLimitRecord(limits, rec("t4", 24))).toBeNull();
    expect(observeLimitRecord(limits, rec("t5", 5))?.detail).toMatchObject({ from: 24, to: 5 });
  });

  test("canonical windows fold renames: 7-day and weekly are the same window kind", () => {
    expect(canonicalWindowForMetric("7-day")).toBe("weekly");
    expect(canonicalWindowForMetric("weekly")).toBe("weekly");
    expect(canonicalWindowForMetric("5-hour")).toBe("5h");
    expect(canonicalWindowForMetric("some-model:weekly")).toBe("weekly");
    expect(canonicalWindowForMetric("credits_balance")).toBeNull();
    expect(canonicalWindowForMetric("rows-read")).toBeNull();
    expect(poolWindowFields(["7-day", "5h"])).toEqual(poolWindowFields(["weekly", "5-hour"]));
  });
});

describe("replay + tick runners", () => {
  function pctRec(
    ts: string,
    serviceId: string,
    metricId: string,
    value: number,
    limit: number | null = 100,
  ): SnapshotRecord {
    return {
      ts,
      serviceId,
      metricId,
      value,
      unit: "pct",
      limit,
      resetsAt: RESET_FAR,
      source: "api",
      observedBy: "test",
    } as SnapshotRecord;
  }

  test("replay: a planted limit change and a steady pool — the change fires, the pool's burn evaluation is a measured silence", () => {
    const records: SnapshotRecord[] = [
      {
        ts: "2026-11-01T00:00:00.000Z",
        serviceId: "turso",
        metricId: "storage",
        value: 3,
        unit: "gb",
        limit: 5,
        resetsAt: null,
        source: "api",
        observedBy: "test",
      } as SnapshotRecord,
      {
        ts: "2026-11-02T00:00:00.000Z",
        serviceId: "turso",
        metricId: "storage",
        value: 4,
        unit: "gb",
        limit: 24,
        resetsAt: null,
        source: "api",
        observedBy: "test",
      } as SnapshotRecord,
    ];
    const start = Date.parse("2026-11-01T00:00:00.000Z");
    for (let k = 0; k <= 96 * 4; k++) {
      records.push(pctRec(new Date(start + k * 900_000).toISOString(), "meta", "5h", 5 + 0.5 * (k / 4)));
    }
    const { findings, evaluations } = runAnomalyReplay(records);
    const plan = findings.filter((f) => f.detector === "plan-change");
    expect(plan.length).toBe(1);
    expect(plan[0]?.detail).toMatchObject({ from: 5, to: 24 });
    const burn = evaluations.find((e) => e.detector === "burn-spike" && e.serviceId === "meta" && e.window === "5h");
    expect(burn?.state).toBe("silent");
    // A pool with no percent readings anywhere gets no burn row at
    // all — absence is reported by omission, not as an all-clear.
    expect(evaluations.some((e) => e.detector === "burn-spike" && e.serviceId === "deepseek")).toBe(false);
  });

  test("tick: baselines establish silently, a shape change fires once, firedKeys dedupe a repost", () => {
    const now = new Date("2026-11-22T12:00:00.000Z");
    const entry = (usage: unknown): QuotaCurrentEntry =>
      ({
        providerId: "_claude",
        providerName: "Claude",
        serviceId: "anthropic:claude-pro",
        state: "ok",
        measuredAt: now.toISOString(),
        source: "opencode-quota",
        windows: [
          {
            name: "weekly",
            value: 10,
            pctOfLimit: 10,
            valueLabel: null,
            windowSeconds: null,
            resetsAt: RESET_FAR,
            measuredAt: now.toISOString(),
            source: "opencode-quota",
            state: "ok",
          },
        ],
        usage,
      }) as QuotaCurrentEntry;
    const tick1 = runAnomalyTick({
      state: emptyAnomalyState(now),
      entries: [entry({ windows: {} })],
      freshRecords: [],
      history: [],
      now,
    });
    expect(tick1.findings).toEqual([]);
    expect(tick1.evaluations.some((e) => e.state === "baseline-established")).toBe(true);
    const tick2 = runAnomalyTick({
      state: tick1.state,
      entries: [entry({ windows: {}, plan: "pro" })],
      freshRecords: [],
      history: [],
      now: new Date(now.getTime() + 900_000),
    });
    expect(tick2.findings.length).toBe(1);
    expect(tick2.findings[0]?.detector).toBe("schema-drift");
    // The incident the tick files for the finding:
    const f0 = tick2.findings[0];
    if (!f0) throw new Error("expected the schema-drift finding");
    const incident = anomalyIncidentInput(f0);
    expect(incident.source).toBe("quota-anomaly");
    expect(incident.key).toBe(f0.key);
    // Same transition re-observed from the post-tick2 state: the
    // recorded shape already carries the field, so nothing re-fires.
    const tick3 = runAnomalyTick({
      state: tick2.state,
      entries: [entry({ windows: {}, plan: "pro" })],
      freshRecords: [],
      history: [],
      now: new Date(now.getTime() + 1_800_000),
    });
    expect(tick3.findings).toEqual([]);
  });
});
