import { expect, test } from "bun:test";
import { detectCostAnomaly } from "../src/cost-anomaly.ts";
import { loadCostAnomalyEvidence } from "../src/cost-anomaly-producer.ts";
import {
  certifiedMonthlyCharges,
  certifiedServiceActivity,
  monthlyChargeTotals,
  type ServiceActivityCensusEntry,
} from "../src/cost-anomaly-sources.ts";
import type { MatchedCharge } from "../src/spend-dollars.ts";
import type { SourceCoverageSpan } from "../src/spend-rollups.ts";

// Oracle: the #4825 certification contract (COST-ANOMALY.md) — a
// charges-side fact is `complete` only when an ok coverage span covers
// the whole calendar month; absent, partial, or conflicting census
// data certifies nothing and the detector stays silent. Totals are
// the charges pipeline's matched rows summed exactly, in cents.
const FULL_OCT: SourceCoverageSpan[] = [{ from: "2026-10-01T00:00:00Z", to: "2026-11-01T00:00:00Z", status: "ok" }];
const PARTIAL_OCT: SourceCoverageSpan[] = [
  { from: "2026-10-01T00:00:00Z", to: "2026-10-15T00:00:00Z", status: "ok" },
];
const FAILED_OCT: SourceCoverageSpan[] = [
  { from: "2026-10-01T00:00:00Z", to: "2026-11-01T00:00:00Z", status: "failed" },
];

const CHARGES: MatchedCharge[] = [
  { service: "Modal", at: "2026-10-03", amountUsd: 5.1 },
  { service: "Modal", at: "2026-10-20", amountUsd: 2.9 },
  { service: "Modal", at: "2026-11-02", amountUsd: 4 },
  { service: "Fly.io", at: "2026-10-11", amountUsd: 0.1 },
  { service: "Fly.io", at: "2026-10-12", amountUsd: 0.2 },
];

const census = (over: Partial<ServiceActivityCensusEntry> = {}): ServiceActivityCensusEntry => ({
  service: "Modal",
  month: "2026-10",
  sessions: 0,
  quotaActivity: 0,
  coverage: FULL_OCT,
  ...over,
});

test("monthly totals group by service and month with exact cent sums, sorted", () => {
  const totals = monthlyChargeTotals(CHARGES, FULL_OCT);
  expect(totals.map((t) => [t.service, t.month, t.totalUsd])).toEqual([
    ["Fly.io", "2026-10", 0.3],
    ["Modal", "2026-10", 8],
    ["Modal", "2026-11", 4],
  ]);
});

test("monthly totals certify complete only on a whole-month ok span", () => {
  expect(monthlyChargeTotals(CHARGES, FULL_OCT).find((t) => t.month === "2026-10")!.coverage).toBe("complete");
  // The October span does not cover November.
  expect(monthlyChargeTotals(CHARGES, FULL_OCT).find((t) => t.month === "2026-11")!.coverage).toBe("partial");
  expect(monthlyChargeTotals(CHARGES, PARTIAL_OCT).every((t) => t.coverage === "partial")).toBe(true);
  expect(monthlyChargeTotals(CHARGES, FAILED_OCT).every((t) => t.coverage === "partial")).toBe(true);
  expect(monthlyChargeTotals(CHARGES, []).every((t) => t.coverage === "partial")).toBe(true);
});

test("monthly totals skip rows that certify nothing", () => {
  const totals = monthlyChargeTotals(
    [
      { service: "Modal", at: "not-a-date", amountUsd: 9 },
      { service: "  ", at: "2026-10-03", amountUsd: 9 },
      { service: "Modal", at: "2026-10-03", amountUsd: Number.NaN },
    ],
    FULL_OCT,
  );
  expect(totals).toEqual([]);
});

test("activity seam: absent census entry is uncertified (null), never a certified zero", () => {
  const activity = certifiedServiceActivity([]);
  expect(activity({ service: "Modal", month: "2026-10", totalUsd: 8, coverage: "complete" })).toBeNull();
});

test("activity seam: complete census returns its counts with complete coverage", () => {
  const activity = certifiedServiceActivity([census({ sessions: 3, quotaActivity: 7 })]);
  expect(activity({ service: "Modal", month: "2026-10", totalUsd: 8, coverage: "complete" })).toEqual({
    service: "Modal",
    month: "2026-10",
    sessions: 3,
    quotaActivity: 7,
    coverage: "complete",
  });
});

test("activity seam: partial or failed census coverage stays partial", () => {
  const charge = { service: "Modal", month: "2026-10", totalUsd: 8, coverage: "complete" as const };
  expect(certifiedServiceActivity([census({ coverage: PARTIAL_OCT })])(charge)!.coverage).toBe("partial");
  expect(certifiedServiceActivity([census({ coverage: FAILED_OCT })])(charge)!.coverage).toBe("partial");
});

test("activity seam: conflicting duplicate entries certify nothing; identical duplicates collapse", () => {
  const charge = { service: "Modal", month: "2026-10", totalUsd: 8, coverage: "complete" as const };
  expect(certifiedServiceActivity([census(), census({ sessions: 1 })])(charge)).toBeNull();
  expect(certifiedServiceActivity([census(), census()])(charge)?.sessions).toBe(0);
});

test("activity seam: non-integer or negative counts certify nothing", () => {
  const charge = { service: "Modal", month: "2026-10", totalUsd: 8, coverage: "complete" as const };
  expect(certifiedServiceActivity([census({ sessions: -1 })])(charge)).toBeNull();
  expect(certifiedServiceActivity([census({ quotaActivity: 0.5 })])(charge)).toBeNull();
});

test("end to end: certified charges sources fire exactly the constructed charge anomaly", async () => {
  const now = new Date("2026-11-19T12:00:00Z");
  const deps = {
    readRollups: async () => [],
    monthlyCharges: certifiedMonthlyCharges(CHARGES, FULL_OCT),
    serviceActivity: certifiedServiceActivity([census()]),
  };
  const evidence = await loadCostAnomalyEvidence(deps);
  const chargesEvidence = evidence.filter((e) => e.kind === "charge-without-activity");
  // Modal/October (complete charges, complete zero-activity census)
  // fires; Fly.io/October has no census entry and Modal/November is
  // outside the charges coverage span, so both stay silent.
  const fired = chargesEvidence.map((e) => detectCostAnomaly(e, now)).filter((i) => i !== null);
  expect(fired).toHaveLength(1);
  expect(fired[0]!.title).toContain("Modal");
  // With the census withdrawn, nothing fires: absence is not zero.
  const uncertified = await loadCostAnomalyEvidence({ ...deps, serviceActivity: certifiedServiceActivity([]) });
  expect(
    uncertified.filter((e) => e.kind === "charge-without-activity").map((e) => detectCostAnomaly(e, now)),
  ).toEqual([null, null, null]);
});
