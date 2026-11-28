import { describe, expect, test } from "bun:test";
import {
  DEFAULT_METHOD,
  buildPlan,
  cascadeFor,
  checkDue,
  diffPlans,
  estimateRate,
  netBurnSlope,
  pickLargestWindow,
  projectPool,
  renderPlanMarkdown,
  windowSeconds,
  type RawSample,
} from "../src/quota-plan.ts";

// A pool's recorded series is the ledger rows the gatherer wrote; these
// tests build the same shape by hand. Timestamps are UTC instants.

const HOUR = 3_600_000;

function sample(pool: string, tsIso: string, windows: { name: string; percent: number | null; resetsAt: string | null }[]): RawSample {
  return { pool, provider: "Test Provider", plan: null, ts: tsIso, windows };
}

/** A linear series: `startPct` at `startIso`, burning `ptsPerHour` until `endIso`. */
function linearSeries(
  pool: string,
  window: string,
  startIso: string,
  endIso: string,
  startPct: number,
  ptsPerHour: number,
  resetsAt: string | null,
  stepHours = 1,
): RawSample[] {
  const start = Date.parse(startIso);
  const end = Date.parse(endIso);
  const out: RawSample[] = [];
  for (let t = start; t <= end; t += stepHours * HOUR) {
    const percent = startPct + (ptsPerHour * (t - start)) / HOUR;
    out.push(sample(pool, new Date(t).toISOString(), [{ name: window, percent: Math.min(100, Math.round(percent * 100) / 100), resetsAt }]));
  }
  return out;
}

function hoursBefore(iso: string, hours: number): string {
  return new Date(Date.parse(iso) - hours * HOUR).toISOString();
}

const AS_OF = "2026-11-09T00:00:00Z";

describe("window selection", () => {
  test("largest measured window wins: monthly over weekly over 5h", () => {
    expect(pickLargestWindow([
      { name: "5h", percent: 12, resetsAt: null },
      { name: "weekly", percent: 40, resetsAt: null },
      { name: "monthly", percent: 70, resetsAt: null },
    ])).toBe("monthly");
  });

  test("per-model summary windows resolve to their suffix duration", () => {
    expect(windowSeconds("gemini-3.8-flash-high:weekly")).toBe(7 * 24 * 3600);
    expect(windowSeconds("gemini-3.8-flash-high:5h")).toBe(5 * 3600);
    expect(pickLargestWindow([
      { name: "gemini-3.8-flash-high:5h", percent: 2, resetsAt: null },
      { name: "gemini-3.8-flash-high:weekly", percent: 60, resetsAt: null },
    ])).toBe("gemini-3.8-flash-high:weekly");
  });

  test("percent-less windows are not candidates", () => {
    expect(pickLargestWindow([{ name: "credits_balance", percent: null, resetsAt: null }])).toBeNull();
  });
});

describe("burn rate", () => {
  test("net-burn slope trims a single bad tick at either end", () => {
    const base = Date.parse("2026-11-01T00:00:00Z");
    const points = Array.from({ length: 10 }, (_, i) => ({ ts: base + i * HOUR, percent: i * 2 }));
    points[0] = { ...points[0], percent: 40 }; // bad first tick
    points[9] = { ...points[9], percent: 0 }; // bad last tick
    expect(netBurnSlope(points)).toBeCloseTo(2, 1);
  });

  test("a step-function meter is measured by its net burn, not its flat steps", () => {
    // The provider refreshes the percentage in steps: 9 samples flat at
    // 33.8 then a jump to 41.4. A pairwise median would report ~0.
    const base = Date.parse("2026-11-01T00:00:00Z");
    const points = [
      ...Array.from({ length: 9 }, (_, i) => ({ ts: base + i * 6 * 60_000, percent: 33.8 })),
      ...Array.from({ length: 4 }, (_, i) => ({ ts: base + (54 + i * 12) * 60_000, percent: i < 2 ? 34 : 41.4 })),
    ];
    // 7.6 points over ~1.6h, not 0.
    expect(netBurnSlope(points)!).toBeGreaterThan(3);
  });

  test("a reset inside the series is segmented out — no negative burn", () => {
    const base = Date.parse("2026-11-01T00:00:00Z");
    const points = [
      { ts: base, percent: 90 },
      { ts: base + HOUR, percent: 100 },
      { ts: base + 2 * HOUR, percent: 10 }, // reset
      { ts: base + 3 * HOUR, percent: 15 },
      { ts: base + 4 * HOUR, percent: 20 },
    ];
    const { rate, resetDetected } = estimateRate(points, { asOf: base + 4 * HOUR, rateWindowHours: 24 });
    expect(resetDetected).toBe(true);
    expect(rate.perHour).toBeCloseTo(5, 5);
    expect(rate.confidence).toBe("ok");
  });

  test("a small dip clamps to zero rather than reporting negative burn", () => {
    const base = Date.parse("2026-11-01T00:00:00Z");
    const { rate } = estimateRate(
      [
        { ts: base, percent: 41 },
        { ts: base + HOUR, percent: 40 },
        { ts: base + 2 * HOUR, percent: 40 },
      ],
      { asOf: base + 2 * HOUR, rateWindowHours: 24 },
    );
    expect(rate.perHour).toBe(0);
  });
});

describe("projection (oracle: independent recomputation)", () => {
  // Oracle: landing = current + burn * hours-to-reset, computed with
  // plain arithmetic outside the module. The projection must agree
  // within rounding.
  //
  // Fixture A: weekly window resetting 48h after asOf (so the period
  // started 120h before asOf); 75% burned after 50h at 1.5 pts/h.
  const resetsA = new Date(Date.parse(AS_OF) + 48 * HOUR).toISOString();
  const seriesA = linearSeries(
    "opencode-go.drewrypope",
    "weekly",
    hoursBefore(AS_OF, 50),
    AS_OF,
    0,
    1.5,
    resetsA,
  );

  test("landing matches the hand computation", () => {
    const p = projectPool("opencode-go.drewrypope", seriesA, Date.parse(AS_OF))!;
    const current = 1.5 * 50; // 75
    const hoursLeft = 48;
    expect(p.currentPercent).toBeCloseTo(current, 2);
    expect(p.rate.perHour).toBeCloseTo(1.5, 2);
    expect(p.projectedLanding).toBeCloseTo(current + 1.5 * hoursLeft, 1); // 147
    expect(p.requiredPerHour).toBeCloseTo((100 - current) / hoursLeft, 3);
    expect(p.verdict).toBe("over_pace");
    expect(p.exhaustionAt).not.toBeNull();
    expect(p.spillPctPoints).toBeGreaterThan(0);
  });

  test("next-period landing is the rate applied to a whole period", () => {
    const p = projectPool("opencode-go.drewrypope", seriesA, Date.parse(AS_OF))!;
    expect(p.projectedLandingNextPeriod).toBeCloseTo(1.5 * 168, 0); // 252
    expect(p.notes.join(" ")).toContain("next period walls");
  });

  test("under-pace pool lands short and names a catch-up audible", () => {
    const series = linearSeries("meta", "weekly", hoursBefore(AS_OF, 50), AS_OF, 27.5, 0.05, resetsA);
    const p = projectPool("meta", series, Date.parse(AS_OF))!;
    expect(p.currentPercent).toBeCloseTo(30, 1);
    expect(p.verdict).toBe("under_pace");
    expect(p.projectedLanding!).toBeLessThan(90);
    expect(p.audible?.action).toBe("queue-catch-up");
    expect(p.audible?.detail).toContain("points/day");
    expect(p.audible?.deadline).toBeDefined();
  });

  test("on-track pool confirms", () => {
    // 96h to reset: 40% now + 0.6 pts/h lands at 97.6%.
    const resets = new Date(Date.parse(AS_OF) + 96 * HOUR).toISOString();
    const series = linearSeries("meta", "weekly", hoursBefore(AS_OF, 50), AS_OF, 10, 0.6, resets);
    const p = projectPool("meta", series, Date.parse(AS_OF))!;
    expect(p.projectedLanding!).toBeCloseTo(97.6, 0);
    expect(p.verdict).toBe("on_track");
    expect(p.audible?.action).toBe("confirm");
  });

  test("a pool already at 100% is exhausted and confirms its cascade, not a Drew flag", () => {
    const resets = new Date(Date.parse(AS_OF) + 96 * HOUR).toISOString();
    const series = linearSeries("openai.main", "weekly", hoursBefore(AS_OF, 8), AS_OF, 60, 5, resets);
    const p = projectPool("openai.main", series, Date.parse(AS_OF))!;
    expect(p.currentPercent).toBe(100);
    expect(p.verdict).toBe("exhausted");
    expect(p.audible?.action).toBe("confirm");
    expect(p.audible?.detail).toContain("openai.alternate");
  });

  test("a stale reading is not judged on its numbers", () => {
    const series = linearSeries("anthropic:claude-pro", "7d", "2026-11-01T00:00:00Z", "2026-11-02T00:00:00Z", 10, 1, null);
    const p = projectPool("anthropic:claude-pro", series, Date.parse(AS_OF))!;
    expect(p.verdict).toBe("stale");
    expect(p.audible?.action).toBe("repair-reading");
  });

  test("no reset instant yields no landing and a repair audible", () => {
    const series = linearSeries("_commandcode", "monthly", hoursBefore(AS_OF, 48), AS_OF, 30, 1, null);
    const p = projectPool("_commandcode", series, Date.parse(AS_OF))!;
    expect(p.verdict).toBe("no_reset");
    expect(p.projectedLanding).toBeNull();
    expect(p.audible?.action).toBe("repair-reading");
  });
});

describe("cascade", () => {
  test("position and successor come from the routing fallback map", () => {
    expect(cascadeFor("opencode-go.drewrypope")).toMatchObject({
      group: "deepseek-class",
      position: 1,
      of: 3,
      next: "opencode-go.developing-today-0",
    });
    expect(cascadeFor("opencode-go.developing-today-0")).toMatchObject({
      position: 2,
      next: "deepseek",
    });
  });

  test("an ungrouped pool is its own last resort", () => {
    expect(cascadeFor("some-new-pool")).toEqual({ group: "ungrouped", position: 1, of: 1, next: null });
  });
});

describe("multi-credential pools", () => {
  test("two rows per tick collapse to the binding credential, with a note", () => {
    const resetsA = new Date(Date.parse(AS_OF) + 6 * HOUR).toISOString(); // exhausted credential
    const resetsB = new Date(Date.parse(AS_OF) + 96 * HOUR).toISOString(); // second credential
    const seriesA = linearSeries("openai.main", "weekly", hoursBefore(AS_OF, 8), AS_OF, 60, 5, resetsA);
    const seriesB = linearSeries("openai.main", "weekly", hoursBefore(AS_OF, 48), AS_OF, 20, 1.08, resetsB);
    // Interleave as the gatherer does: one row per credential per tick.
    const merged: RawSample[] = [];
    const n = Math.max(seriesA.length, seriesB.length);
    for (let i = 0; i < n; i += 1) {
      if (seriesA[i]) merged.push(seriesA[i]);
      if (seriesB[i]) merged.push(seriesB[i]);
    }
    const plan = buildPlan(merged, AS_OF);
    const rows = plan.pools.filter((p) => p.pool === "openai.main");
    expect(rows).toHaveLength(1);
    expect(rows[0].currentPercent).toBe(100); // the binding credential, not the comfortable one
    expect(rows[0].verdict).toBe("exhausted");
    expect(rows[0].notes.join(" ")).toContain("multi-credential");
  });
});

describe("plan, diff and cadence", () => {
  const resets = new Date(Date.parse(AS_OF) + 48 * HOUR).toISOString();

  function baselineSamples(): RawSample[] {
    return [
      // under pace: 30% now, 0.05 pts/h
      ...linearSeries("meta", "weekly", hoursBefore(AS_OF, 50), AS_OF, 27.5, 0.05, resets),
      // over pace: 75% now, 1.5 pts/h
      ...linearSeries("openai.main", "weekly", hoursBefore(AS_OF, 50), AS_OF, 0, 1.5, resets),
      // on track on the monthly window: ~52% now, 0.08 pts/h, 24 days left
      ...linearSeries("cline-pass", "monthly", "2026-11-03T00:00:00Z", AS_OF, 40.38, 0.08, "2026-12-03T00:00:00Z"),
    ];
  }

  test("a slowly burning weekly pool also reports the next-period projection", () => {
    const plan = buildPlan(baselineSamples(), AS_OF);
    const meta = plan.pools.find((p) => p.pool === "meta")!;
    expect(meta.projectedLandingNextPeriod).toBeCloseTo(0.05 * 168, 0); // 8% next period
    expect(meta.notes.join(" ")).toContain("next period strands");
  });

  test("plan covers every pool's largest window with a cascade and an audible", () => {
    const plan = buildPlan(baselineSamples(), AS_OF);
    expect(plan.pools.map((p) => p.pool).sort()).toEqual(["cline-pass", "meta", "openai.main"]);
    for (const p of plan.pools) {
      expect(p.cascade.of).toBeGreaterThanOrEqual(1);
      expect(p.audible).not.toBeNull();
    }
    expect(plan.pools.find((p) => p.pool === "cline-pass")!.window).toBe("monthly");
    expect(plan.summary.underPace).toBe(1);
    expect(plan.summary.overPace).toBe(1);
  });

  test("markdown names every pool and every non-confirm audible", () => {
    const plan = buildPlan(baselineSamples(), AS_OF);
    const md = renderPlanMarkdown(plan);
    for (const p of plan.pools) expect(md).toContain(`\`${p.pool}\``);
    expect(md).toContain("queue-catch-up");
    expect(md).toContain("flag-drew");
    expect(md).toContain("Dated audibles");
  });

  test("a first run has nothing to diff", () => {
    const plan = buildPlan(baselineSamples(), AS_OF);
    expect(diffPlans(null, plan).firstRun).toBe(true);
  });

  test("a newly under-pace pool escalates; a recovered one is named", () => {
    const baseline = buildPlan(baselineSamples(), AS_OF);
    const nextAsOf = new Date(Date.parse(AS_OF) + 6 * HOUR).toISOString();
    const nextResets = new Date(Date.parse(nextAsOf) + 42 * HOUR).toISOString();
    const next = buildPlan(
      [
        // meta stays under pace
        ...linearSeries("meta", "weekly", hoursBefore(nextAsOf, 50), nextAsOf, 27.5, 0.05, nextResets),
        // openai.main recovers: 90% now with only 6h left lands ~96%
        ...linearSeries(
          "openai.main",
          "weekly",
          hoursBefore(nextAsOf, 48),
          nextAsOf,
          45,
          0.94,
          new Date(Date.parse(nextAsOf) + 6 * HOUR).toISOString(),
        ),
        // cline-pass turns over-pace: 82.5% now at 1.65 pts/h with 42h left
        ...linearSeries("cline-pass", "monthly", hoursBefore(nextAsOf, 50), nextAsOf, 0, 1.65, "2026-12-03T00:00:00Z"),
      ],
      nextAsOf,
    );
    const diff = diffPlans(baseline, next);
    expect(diff.escalations.map((e) => e.pool)).toContain("cline-pass");
    expect(diff.recovered).toContain("openai.main");
  });

  test("the 4–6h due guard", () => {
    const now = Date.parse("2026-11-09T12:00:00Z");
    expect(checkDue(null, now, 6)).toBe(true);
    expect(checkDue("2026-11-09T09:00:00Z", now, 6)).toBe(false); // 3h ago
    expect(checkDue("2026-11-09T05:00:00Z", now, 6)).toBe(true); // 7h ago
    expect(checkDue("2026-11-09T12:00:00Z", now, 6)).toBe(false);
    expect(DEFAULT_METHOD.checkIntervalHours).toBe(6);
  });
});
