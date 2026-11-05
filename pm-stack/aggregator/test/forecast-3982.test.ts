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

  test("zero lower-quartile pace: P50 present, P80 explicitly unmeasurable, no crash", () => {
    // Primary weeks [0, 1, 1, 1]: median pace 1, lower quartile 0.
    const eta = deriveEpicEta({
      subjectRef: EPIC,
      cutoffs: history(5, [2, 3, 4, null, null]).slice(0, 5),
    });
    expect(eta.status).toBe("forecast");
    expect(eta.sample.completionsPrimary).toBe(3);
    expect(eta.p50).not.toBeNull();
    expect(eta.p80).toBeNull();
    expect(eta.limitations.join(" ")).toContain("lower-quartile");
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
