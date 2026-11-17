import { describe, expect, test } from "bun:test";
import { deriveEpicEta, deriveStallRisk, FORECAST_MODEL_VERSION } from "../src/forecast.ts";
import type { ItemHistoryCutoff, ItemHistoryItem } from "../src/item-history.ts";

const REPO = "developing-today/code";
const EPIC = `${REPO}#100`;

// Mondays, weekly.
const W = [
  "2026-09-07T00:00:00Z",
  "2026-09-14T00:00:00Z",
  "2026-09-21T00:00:00Z",
  "2026-09-28T00:00:00Z",
  "2026-10-05T00:00:00Z",
  "2026-10-12T00:00:00Z",
  "2026-10-19T00:00:00Z",
  "2026-10-26T00:00:00Z",
  "2026-11-02T00:00:00Z",
];

function item(number: number, partial: Partial<ItemHistoryItem> = {}): ItemHistoryItem {
  return {
    ref: `${REPO}#${number}`,
    repo: REPO,
    number,
    kind: "issue",
    state: "open",
    status: null,
    effort: null,
    parentRef: EPIC,
    blockedBy: [],
    ...partial,
  };
}

function cutoff(at: string, items: ItemHistoryItem[], complete = true): ItemHistoryCutoff {
  return {
    schemaVersion: 1,
    cutoffAt: at,
    coverage: { source: "projects-pool", fetchedAt: at, complete, unavailable: !complete },
    items,
  };
}

/** Children 1..total; `closesAtWeek[i]` = week index at which child
 * i+1 closes (stays closed). */
function history(total: number, closesAtWeek: (number | null)[]): ItemHistoryCutoff[] {
  return W.map((at, week) =>
    cutoff(
      at,
      Array.from({ length: total }, (_, i) =>
        item(i + 1, {
          state: closesAtWeek[i] !== null && closesAtWeek[i]! <= week ? "closed" : "open",
        }),
      ),
    ),
  );
}


// --- Generated model-based coverage for the design \u00a71 resampling
// estimator (Codex P1 on PR #6169; testing doctrine: PBT/MBT with a
// named oracle). The fixed examples alone cannot detect a wrong
// resampling or percentile calculation (any deterministic ordered
// pair of dates passes them), so the estimator is also checked
// against an exact reference model over generated inputs.
//
// ORACLE (named): the exact completion-time distribution of the
// design \u00a71 model, computed by dynamic programming \u2014 each week
// independently draws one observed bucket uniformly with replacement,
// so the distribution of cumulative completions after w weeks is an
// exact convolution, and the completion-time quantile is the
// smallest w whose exact CDF reaches q. The oracle shares no code
// with the estimator's PRNG, sampling, or percentile code, so it
// decides the expected quantiles independently.
//
// SAMPLING TOLERANCE (explicit): the estimator reports nearest-rank
// quantiles of 4096 resampled trials, so its weeks can differ from
// the exact oracle by sampling error. Calibration over the generated
// range below (buckets 0..4, remaining 1..10) showed a maximum
// deviation of 1 week across 294 forecast cases; the tolerance is 2
// weeks \u2014 twice the observed maximum, and far smaller than the
// error a swapped percentile, a wrong bucket series, or a constant
// date would produce (the constant-bucket cases below are exact,
// tolerance zero).
const SAMPLING_TOLERANCE_WEEKS = 2;

function exactWeeksQuantile(buckets: number[], remaining: number, q: number): number {
  let dist = new Map<number, number>([[0, 1]]);
  for (let w = 1; w <= 100_000; w++) {
    const next = new Map<number, number>();
    for (const [sum, p] of dist) {
      for (const b of buckets) {
        const s = Math.min(remaining, sum + b);
        next.set(s, (next.get(s) ?? 0) + p / buckets.length);
      }
    }
    dist = next;
    if ((dist.get(remaining) ?? 0) >= q) return w;
  }
  throw new Error("oracle did not converge");
}

/** Realize primary buckets b[0..3] as closures at weeks 1..4 plus
 * `remaining` children that never close, over the first 5 cutoffs. */
function historyForBuckets(buckets: number[], remaining: number): ItemHistoryCutoff[] {
  const closes: (number | null)[] = [];
  buckets.forEach((count, i) => {
    for (let k = 0; k < count; k++) closes.push(i + 1);
  });
  for (let k = 0; k < remaining; k++) closes.push(null);
  return history(closes.length, closes).slice(0, 5);
}

function etaWeeks(eta: { p50: string | null; p80: string | null; asOf: string }): { p50: number; p80: number } {
  const toWeeks = (d: string | null) => Math.round((Date.parse(d!) - Date.parse(eta.asOf)) / (7 * 24 * 3600 * 1000));
  return { p50: toWeeks(eta.p50), p80: toWeeks(eta.p80) };
}

/** Deterministic case generator (fixed seed: the suite is a
 * regression pin, not a fuzz run). */
function generatedEstimatorCases(count: number): { buckets: number[]; remaining: number }[] {
  let s = 0x5eed_5274;
  const rnd = () => {
    s = (Math.imul(s, 1664525) + 1013904223) >>> 0;
    return s / 4294967296;
  };
  return Array.from({ length: count }, () => ({
    buckets: Array.from({ length: 4 }, () => Math.floor(rnd() * 5)),
    remaining: 1 + Math.floor(rnd() * 10),
  }));
}

describe("deriveEpicEta", () => {
  test("missing history: not_enough_history, no dates, unavailable confidence", () => {
    const eta = deriveEpicEta({ subjectRef: EPIC, cutoffs: [] });
    expect(eta.status).toBe("not_enough_history");
    expect(eta.p50).toBeNull();
    expect(eta.p80).toBeNull();
    expect(eta.confidence).toBe("unavailable");
    expect(eta.modelVersion).toBe(FORECAST_MODEL_VERSION);
  });

  test("explicit childRefs absent from the latest cutoff are unknown, not complete", () => {
    // Requested child #2 is never observed in any cutoff; requested
    // child #1 is closed throughout. The epic must not be declared
    // complete (no p50 = p80 = asOf) while #2's state is unknown.
    const cutoffs = history(1, [0]);
    const eta = deriveEpicEta({
      subjectRef: EPIC,
      cutoffs,
      childRefs: [`${REPO}#1`, `${REPO}#2`],
    });
    expect(eta.remainingUnits).toBe(0);
    expect(eta.completedUnits).toBe(1);
    expect(eta.status).toBe("not_enough_history");
    expect(eta.p50).toBeNull();
    expect(eta.p80).toBeNull();
    expect(eta.limitations.join(" ")).toContain("state unknown");
    expect(eta.limitations.join(" ")).toContain(`${REPO}#2`);
    // Control: with only the observed child requested, nothing-left
    // still returns the as-of instant itself.
    const complete = deriveEpicEta({ subjectRef: EPIC, cutoffs, childRefs: [`${REPO}#1`] });
    expect(complete.p50).toBe(complete.asOf);
    expect(complete.p80).toBe(complete.asOf);
  });

  test("incomplete cutoffs are excluded, never read as zero weeks", () => {
    const cutoffs = history(6, [null, null, null, null, null, null]).map((c, i) =>
      i % 2 === 1 ? { ...c, coverage: { ...c.coverage, complete: false, unavailable: true } } : c,
    );
    const eta = deriveEpicEta({ subjectRef: EPIC, cutoffs });
    expect(eta.inputCoverage.unavailableCutoffs).toBeGreaterThan(0);
    expect(eta.inputCoverage.completeWeeks).toBeLessThan(cutoffs.length);
  });

  test("steady pace over 8 weeks: forecast with P50 <= P80 and a real sample", () => {
    // 2 closures per week for 8 weeks (16 units), 8 children remain.
    const closes: (number | null)[] = Array.from({ length: 24 }, (_, i) => (i < 16 ? Math.floor(i / 2) + 1 : null));
    const eta = deriveEpicEta({ subjectRef: EPIC, cutoffs: history(24, closes) });
    expect(eta.status).toBe("forecast");
    expect(eta.completedUnits).toBe(16);
    expect(eta.remainingUnits).toBe(8);
    expect(eta.sample.completionsPrimary).toBe(8);
    expect(eta.p50).not.toBeNull();
    expect(eta.p80).not.toBeNull();
    expect(eta.p80! >= eta.p50!).toBe(true);
    expect(eta.confidence).toBe("high");
    expect(eta.baselineSource).toBe(EPIC);
  });

  test("zero pace across a full primary window: no_measurable_pace, no dates", () => {
    const eta = deriveEpicEta({
      subjectRef: EPIC,
      cutoffs: history(5, [null, null, null, null, null]).slice(0, 6),
    });
    expect(eta.status).toBe("no_measurable_pace");
    expect(eta.p50).toBeNull();
    expect(eta.remainingUnits).toBe(5);
  });

  test("sparse sample without a baseline: not_enough_history, no dates", () => {
    const closes: (number | null)[] = [3, null, null, null, null, null];
    const eta = deriveEpicEta({ subjectRef: EPIC, cutoffs: history(6, closes).slice(0, 6) });
    expect(eta.status).toBe("not_enough_history");
    expect(eta.p50).toBeNull();
    expect(eta.p80).toBeNull();
  });

  test("sparse sample with a named baseline: borrowed_baseline, low confidence, dates present", () => {
    const closes: (number | null)[] = [3, null, null, null, null, null];
    const eta = deriveEpicEta({
      subjectRef: EPIC,
      cutoffs: history(6, closes).slice(0, 6),
      borrowedBaseline: { scopeRef: `${REPO}#200`, weeklyCompletions: [2, 3, 2, 3] },
    });
    expect(eta.status).toBe("borrowed_baseline");
    expect(eta.baselineSource).toBe(`${REPO}#200`);
    expect(eta.confidence).toBe("low");
    expect(eta.p50).not.toBeNull();
  });

  test("a reopened child is remaining work, not a completion", () => {
    const cutoffs = history(6, [null, null, null, null, null, null]);
    // Child 1 closes at week 2, reopens at week 4, stays open.
    for (let w = 0; w < cutoffs.length; w++) {
      const it = cutoffs[w]!.items[0]!;
      it.state = w >= 2 && w < 4 ? "closed" : "open";
    }
    const eta = deriveEpicEta({ subjectRef: EPIC, cutoffs });
    expect(eta.completedUnits).toBe(0);
    expect(eta.remainingUnits).toBe(6);
  });

  test("merged PR children never count as units", () => {
    const cutoffs = history(4, [1, 2, null, null]).slice(0, 5);
    for (const c of cutoffs) {
      c.items.push(item(900, { kind: "pr", state: "merged" }));
    }
    const eta = deriveEpicEta({ subjectRef: EPIC, cutoffs });
    expect(eta.completedUnits).toBe(2);
    expect(eta.remainingUnits).toBe(2);
  });

  test("zero lower-quartile pace, re-pinned for the resampling estimator (#5274): P50 and P80 both present", () => {
    // Primary weeks [0, 1, 1, 1]. Under forecast/v1 the median pace
    // was 1 and the lower-quartile pace 0, so P80 was explicitly
    // unmeasurable (the previous pin). The design \u00a71 resampling
    // estimator (forecast/v2) draws the observed buckets themselves:
    // a zero week is a draw, not a denominator, so the
    // weeks-to-complete distribution has a finite conservative tail
    // and P80 is defensible. This pin replaces the v1 pin with that
    // rationale, per #5274's acceptance criteria.
    const eta = deriveEpicEta({
      subjectRef: EPIC,
      cutoffs: history(5, [2, 3, 4, null, null]).slice(0, 5),
    });
    expect(eta.status).toBe("forecast");
    expect(eta.sample.completionsPrimary).toBe(3);
    expect(eta.p50).not.toBeNull();
    expect(eta.p80).not.toBeNull();
    expect(eta.p80! >= eta.p50!).toBe(true);
    expect(eta.limitations.join(" ")).not.toContain("lower-quartile");
  });

  test("bursty zero-median buckets [0, 0, 0, 4]: forecast with finite P50 <= P80, not no_measurable_pace (#5274)", () => {
    // 4 children close in the last observed week; 4 remain. The
    // median bucket is 0, so forecast/v1 returned
    // no_measurable_pace despite 4 completions of evidence; the
    // resampling estimator forecasts from the bucket distribution.
    const closes: (number | null)[] = [4, 4, 4, 4, null, null, null, null];
    const eta = deriveEpicEta({ subjectRef: EPIC, cutoffs: history(8, closes).slice(0, 5) });
    expect(eta.status).toBe("forecast");
    expect(eta.sample.completionsPrimary).toBe(4);
    expect(eta.remainingUnits).toBe(4);
    expect(eta.p50).not.toBeNull();
    expect(eta.p80).not.toBeNull();
    expect(Number.isFinite(Date.parse(eta.p50!))).toBe(true);
    expect(Number.isFinite(Date.parse(eta.p80!))).toBe(true);
    expect(eta.p80! >= eta.p50!).toBe(true);
  });

  test("resampling estimator is deterministic: identical inputs produce identical outputs (#5274)", () => {
    const closes: (number | null)[] = [4, 4, 4, 4, null, null, null, null];
    const cutoffs = history(8, closes).slice(0, 5);
    const a = deriveEpicEta({ subjectRef: EPIC, cutoffs });
    const b = deriveEpicEta({ subjectRef: EPIC, cutoffs });
    expect(b).toEqual(a);
  });


  test("generated cases match the exact DP oracle within the stated sampling tolerance (MBT, #5274 / PR #6169 Codex P1)", () => {
    let forecastCases = 0;
    for (const { buckets, remaining } of generatedEstimatorCases(240)) {
      const eta = deriveEpicEta({ subjectRef: EPIC, cutoffs: historyForBuckets(buckets, remaining) });
      const total = buckets.reduce((a, b) => a + b, 0);
      if (total === 0) {
        expect(eta.status).toBe("no_measurable_pace");
        expect(eta.p50).toBeNull();
        expect(eta.p80).toBeNull();
        continue;
      }
      if (total < 3) {
        expect(eta.status).toBe("not_enough_history");
        expect(eta.p50).toBeNull();
        expect(eta.p80).toBeNull();
        continue;
      }
      forecastCases++;
      expect(eta.status).toBe("forecast");
      expect(eta.p50).not.toBeNull();
      expect(eta.p80).not.toBeNull();
      const w = etaWeeks(eta);
      expect(w.p50).toBeGreaterThanOrEqual(1);
      expect(w.p80).toBeGreaterThanOrEqual(w.p50);
      expect(Math.abs(w.p50 - exactWeeksQuantile(buckets, remaining, 0.5))).toBeLessThanOrEqual(
        SAMPLING_TOLERANCE_WEEKS,
      );
      expect(Math.abs(w.p80 - exactWeeksQuantile(buckets, remaining, 0.8))).toBeLessThanOrEqual(
        SAMPLING_TOLERANCE_WEEKS,
      );
    }
    // Guard the generator itself: most generated cases must exercise
    // the forecast path, or the oracle comparison above is vacuous.
    expect(forecastCases).toBeGreaterThan(150);
  });

  test("generated cases are deterministic: identical generated inputs produce identical outputs (PBT, #5274)", () => {
    generatedEstimatorCases(240).forEach(({ buckets, remaining }, i) => {
      if (i % 10 !== 0) return;
      const cutoffs = historyForBuckets(buckets, remaining);
      expect(deriveEpicEta({ subjectRef: EPIC, cutoffs })).toEqual(deriveEpicEta({ subjectRef: EPIC, cutoffs }));
    });
  });

  test("constant buckets are exact (tolerance zero): every resample draws the same pace (MBT, #5274)", () => {
    for (const c of [1, 2, 3, 4]) {
      for (const remaining of [1, 2, 5, 9, 12]) {
        const buckets = [c, c, c, c];
        const eta = deriveEpicEta({ subjectRef: EPIC, cutoffs: historyForBuckets(buckets, remaining) });
        expect(eta.status).toBe("forecast");
        const exact = Math.ceil(remaining / c);
        expect(exactWeeksQuantile(buckets, remaining, 0.5)).toBe(exact);
        expect(exactWeeksQuantile(buckets, remaining, 0.8)).toBe(exact);
        const w = etaWeeks(eta);
        expect(w.p50).toBe(exact);
        expect(w.p80).toBe(exact);
      }
    }
  });

  test("capped trials are censored, not completions: buckets [1, 1, 1, 1] with 100001 remaining publish no date (#6386)", () => {
    // The Codex P2 example from PR #6169, verbatim. ORACLE (exact
    // arithmetic, no sampling): a constant pace of 1 unit/week needs
    // exactly 100001 weeks for 100001 units, so EVERY resampled trial
    // exceeds the 100000-week cap and is censored — neither
    // percentile is inside the simulation horizon. Before the fix
    // each capped trial was recorded as a completion at the cap and
    // both dates were published at 100000 weeks, one week before
    // the work can finish.
    const eta = deriveEpicEta({ subjectRef: EPIC, cutoffs: historyForBuckets([1, 1, 1, 1], 100_001) });
    expect(eta.status).toBe("forecast");
    expect(eta.remainingUnits).toBe(100_001);
    expect(eta.p50).toBeNull();
    expect(eta.p80).toBeNull();
    expect(eta.confidence).toBe("unavailable");
    expect(eta.limitations.some((l) => l.includes("P50") && l.includes("censored"))).toBe(true);
    expect(eta.limitations.some((l) => l.includes("P80") && l.includes("censored"))).toBe(true);
  });

  test("partially censored distribution: P50 inside the horizon is dated, P80 beyond it is not (#6386)", () => {
    // Buckets [0, 0, 0, 4] with 99600 remaining: a trial finishes
    // within the cap iff 4 * Binomial(100000, 1/4) >= 99600, which
    // the exact binomial CDF puts at ~77% of trials — so the median
    // trial completes (just inside the horizon) while the 80th
    // percentile lies beyond it and must return no date. (Sampling
    // margin: the censored share sits ~5 sigma from the 20% that
    // would move the P80 rank onto a finite sample.)
    const eta = deriveEpicEta({ subjectRef: EPIC, cutoffs: historyForBuckets([0, 0, 0, 4], 99_600) });
    expect(eta.status).toBe("forecast");
    expect(eta.remainingUnits).toBe(99_600);
    expect(eta.p50).not.toBeNull();
    const p50Weeks = Math.round((Date.parse(eta.p50!) - Date.parse(eta.asOf)) / (7 * 24 * 3600 * 1000));
    expect(p50Weeks).toBeGreaterThan(98_000);
    expect(p50Weeks).toBeLessThanOrEqual(100_000);
    expect(eta.p80).toBeNull();
    expect(eta.confidence).not.toBe("unavailable");
    expect(eta.limitations.some((l) => l.includes("P80") && l.includes("censored"))).toBe(true);
    expect(eta.limitations.some((l) => l.includes("P50 exceeds"))).toBe(false);
  });

  test("all-zero borrowed baseline buckets: still no_measurable_pace, no dates (#5274)", () => {
    // Local evidence is below the floor (1 completion in the primary
    // window) and the borrowed baseline's buckets are all zero: the
    // total observed pace is zero, and resampling zero buckets can
    // only draw zero \u2014 no date is defensible.
    const closes: (number | null)[] = [3, null, null, null, null, null];
    const eta = deriveEpicEta({
      subjectRef: EPIC,
      cutoffs: history(6, closes).slice(0, 6),
      borrowedBaseline: { scopeRef: `${REPO}#200`, weeklyCompletions: [0, 0, 0, 0] },
    });
    expect(eta.status).toBe("no_measurable_pace");
    expect(eta.p50).toBeNull();
    expect(eta.p80).toBeNull();
  });

  test("inferred scope counts direct children only, not grandchildren via epicRef", () => {
    const cutoffs = [
      cutoff(W[0]!, [item(1), item(2, { parentRef: `${REPO}#1`, epicRef: EPIC }), item(3, { state: "closed" })]),
    ];
    const eta = deriveEpicEta({ subjectRef: EPIC, cutoffs });
    expect(eta.remainingUnits).toBe(1);
    expect(eta.completedUnits).toBe(1);
  });

  test("asOf excludes later cutoffs: a closure after asOf is not yet a completion", () => {
    const eta = deriveEpicEta({
      subjectRef: EPIC,
      cutoffs: history(3, [4, null, null]),
      asOf: W[2]!,
    });
    expect(eta.asOf).toBe(W[2]!);
    expect(eta.completedUnits).toBe(0);
    expect(eta.remainingUnits).toBe(3);
  });

  test("asOf compares as an instant: equal instant in another representation is included", () => {
    // W[8] is "2026-11-02T00:00:00Z"; asOf writes the same instant with
    // fractional seconds. String order would exclude the W[8] cutoff
    // ("Z" > "."), instant order includes it.
    const eta = deriveEpicEta({
      subjectRef: EPIC,
      cutoffs: history(3, [4, null, null]),
      asOf: "2026-11-02T00:00:00.000Z",
    });
    expect(eta.inputCoverage.completeWeeks).toBe(9);
    expect(eta.completedUnits).toBe(1);
  });

  test("asOf compares as an instant: an offset cutoff later than asOf is excluded", () => {
    // The last cutoff written with a -05:00 offset is 05:00Z — after
    // asOf at 00:00Z — although its string sorts before "Z".
    const cutoffs = history(3, [4, null, null]);
    cutoffs[cutoffs.length - 1] = {
      ...cutoffs[cutoffs.length - 1]!,
      cutoffAt: "2026-11-02T00:00:00-05:00",
    };
    const eta = deriveEpicEta({
      subjectRef: EPIC,
      cutoffs,
      asOf: "2026-11-02T00:00:00Z",
    });
    expect(eta.inputCoverage.completeWeeks).toBe(8);
  });

  test("effort stays explicit: unsized and open-ended remaining work are named, not sized", () => {
    const cutoffs = history(4, [null, null, null, null]).slice(0, 5);
    const latest = cutoffs[cutoffs.length - 1]!;
    latest.items[0]!.effort = "l";
    latest.items[1]!.effort = "s";
    const eta = deriveEpicEta({ subjectRef: EPIC, cutoffs });
    expect(eta.remainingEffort.unsized).toBe(2);
    expect(eta.remainingEffort.openEnded).toBe(1);
    expect(eta.remainingEffort.byEffort).toEqual({ l: 1, s: 1 });
    expect(eta.limitations.join(" ")).toContain("unsized");
  });
});

describe("deriveStallRisk", () => {
  const REF = `${REPO}#7`;
  const BLOCKER = `${REPO}#9`;

  function blockedHistory(): ItemHistoryCutoff[] {
    return [cutoff(W[0]!, [item(7, { blockedBy: [BLOCKER] })]), cutoff(W[2]!, [item(7, { blockedBy: [BLOCKER] })])];
  }

  test("blocked edge unresolved 14 days (lower bound): +30 with the bound labelled", () => {
    const risk = deriveStallRisk({ ref: REF, cutoffs: blockedHistory(), asOf: W[2]! });
    const signal = risk.signals.find((s) => s.code === "blocked_edge_age")!;
    expect(signal.contribution).toBe(30);
    expect(signal.detail).toContain("lower bound");
    expect(risk.score).toBe(30);
    expect(risk.atRisk).toBe(false);
  });

  test("no-activity proxy signal: +20 when updatedAt is 7+ days old", () => {
    const risk = deriveStallRisk({
      ref: REF,
      cutoffs: blockedHistory(),
      asOf: W[2]!,
      updatedAt: "2026-09-10T00:00:00Z",
    });
    const signal = risk.signals.find((s) => s.code === "no_meaningful_activity")!;
    expect(signal.contribution).toBe(20);
    expect(signal.detail).toContain("proxy");
  });

  test("queue heartbeat signal: +30 at two missed intervals", () => {
    const risk = deriveStallRisk({
      ref: REF,
      cutoffs: blockedHistory(),
      asOf: W[2]!,
      queue: { missedHeartbeatIntervals: 2 },
    });
    expect(risk.signals.find((s) => s.code === "queue_heartbeat")!.contribution).toBe(30);
  });

  test("queue requeue signal: +20 at two requeues in 14 days", () => {
    const risk = deriveStallRisk({
      ref: REF,
      cutoffs: blockedHistory(),
      asOf: W[2]!,
      queue: { requeues14d: 3 },
    });
    expect(risk.signals.find((s) => s.code === "queue_requeues")!.contribution).toBe(20);
  });

  test("blocked + heartbeat = 60: at risk, informational only, never escalates", () => {
    const risk = deriveStallRisk({
      ref: REF,
      cutoffs: blockedHistory(),
      asOf: W[2]!,
      queue: { missedHeartbeatIntervals: 3 },
    });
    expect(risk.score).toBe(60);
    expect(risk.atRisk).toBe(true);
    expect(risk.escalatesToNeedsYou).toBe(false);
  });

  test("unsupplied signals stay unknown and reduce evidence coverage", () => {
    const risk = deriveStallRisk({ ref: REF, cutoffs: blockedHistory(), asOf: W[2]! });
    expect(risk.signals.find((s) => s.code === "no_meaningful_activity")!.known).toBe(false);
    expect(risk.signals.find((s) => s.code === "queue_heartbeat")!.known).toBe(false);
    expect(risk.signals.find((s) => s.code === "queue_requeues")!.known).toBe(false);
    expect(risk.evidenceCoverage).toBe(0.25);
    // Unknown signals contribute nothing.
    expect(risk.score).toBe(30);
  });

  test("a blocker closed at the latest cutoff is resolved: no blocked-edge points", () => {
    const cutoffs = [
      cutoff(W[0]!, [item(7, { blockedBy: [BLOCKER] }), item(9, { state: "open" })]),
      cutoff(W[2]!, [item(7, { blockedBy: [BLOCKER] }), item(9, { state: "closed" })]),
    ];
    const risk = deriveStallRisk({ ref: REF, cutoffs, asOf: W[2]! });
    const signal = risk.signals.find((s) => s.code === "blocked_edge_age")!;
    expect(signal.contribution).toBe(0);
    expect(signal.detail).toContain("resolved");
    expect(risk.score).toBe(0);
  });

  test("default asOf bounds blocker age to the latest complete cutoff", () => {
    // The edge is first observed at the complete Sep 7 cutoff (W[0]);
    // a later UNAVAILABLE Sep 14 cutoff (W[1]) also lists it. With no
    // explicit asOf the result is dated at the latest complete cutoff
    // (Sep 7), where the edge's lower-bound age is 0 days: the later
    // unavailable observation must not fire blocked-edge points.
    const cutoffs = [
      cutoff(W[0]!, [item(7, { blockedBy: [BLOCKER] })]),
      cutoff(W[1]!, [item(7, { blockedBy: [BLOCKER] })], false),
    ];
    const risk = deriveStallRisk({ ref: REF, cutoffs });
    expect(risk.asOf).toBe(W[0]!);
    const signal = risk.signals.find((s) => s.code === "blocked_edge_age")!;
    expect(signal.contribution).toBe(0);
    expect(risk.score).toBe(0);
  });

  test("asOf compares as an instant: same instant, fractional-second representation", () => {
    // W[2] is "2026-09-21T00:00:00Z"; asOf writes the same instant as
    // "2026-09-21T00:00:00.000Z". String order would scope the history
    // to W[0] only (blocked age 0, no points); instant order keeps
    // W[2], where the lower-bound age is 14 days.
    const risk = deriveStallRisk({
      ref: REF,
      cutoffs: blockedHistory(),
      asOf: "2026-09-21T00:00:00.000Z",
    });
    expect(risk.signals.find((s) => s.code === "blocked_edge_age")!.contribution).toBe(30);
  });

  test("all signals firing caps at 100", () => {
    const risk = deriveStallRisk({
      ref: REF,
      cutoffs: blockedHistory(),
      asOf: W[2]!,
      updatedAt: "2026-09-01T00:00:00Z",
      queue: { missedHeartbeatIntervals: 2, requeues14d: 5 },
    });
    expect(risk.score).toBe(100);
    expect(risk.evidenceCoverage).toBe(1);
  });
});
