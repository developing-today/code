import { expect, test } from "bun:test";
import { detectCostAnomaly } from "../src/cost-anomaly.ts";
import { loadCostAnomalyEvidence } from "../src/cost-anomaly-producer.ts";
import {
  certifiedBusiestRoot,
  certifiedClosedChildren,
  certifiedSourceTotal,
  type ClosureCensusEntry,
  type RootSessionCensusEntry,
  type SourcePopulationCensusEntry,
} from "../src/cost-anomaly-rollup-sources.ts";
import type { SourceCoverageSpan, SpendRollup } from "../src/spend-rollups.ts";

// Oracle: the #4825 certification contract (COST-ANOMALY.md) — a
// rollup-side fact is `complete` only when an ok coverage span covers
// the whole rollup window; absent, partial, or conflicting census
// data certifies nothing and the detector stays silent. The busiest
// root is the maximum folded root total in the census; a source
// total is the pre-attribution population, never a sum of rollups.
const WEEK_START = "2026-10-05T00:00:00.000Z";
const WEEK_END = "2026-10-12T00:00:00.000Z";
const PREV_START = "2026-09-28T00:00:00.000Z";

const FULL_WEEK: SourceCoverageSpan[] = [{ from: "2026-10-05T00:00:00Z", to: "2026-10-12T00:00:00Z", status: "ok" }];
const FULL_PREV: SourceCoverageSpan[] = [{ from: "2026-09-28T00:00:00Z", to: "2026-10-05T00:00:00Z", status: "ok" }];
const PARTIAL_WEEK: SourceCoverageSpan[] = [{ from: "2026-10-05T00:00:00Z", to: "2026-10-09T00:00:00Z", status: "ok" }];
const FAILED_WEEK: SourceCoverageSpan[] = [
  { from: "2026-10-05T00:00:00Z", to: "2026-10-12T00:00:00Z", status: "failed" },
];

const spend = (over: Partial<SpendRollup> = {}): SpendRollup => ({
  kind: "spend-rollup/v1",
  windowKind: "week",
  windowStart: WEEK_START,
  windowEnd: WEEK_END,
  scopeKind: "epic",
  scopeId: "epic-1",
  unit: "tokens",
  pool: "meta",
  value: 1000,
  coverage: "complete",
  coverageStart: "2026-01-01T00:00:00.000Z",
  attributionVersion: "v1",
  ...over,
});

const closure = (over: Partial<ClosureCensusEntry> = {}): ClosureCensusEntry => ({
  scopeKind: "epic",
  scopeId: "epic-1",
  windowKind: "week",
  windowStart: WEEK_START,
  windowEnd: WEEK_END,
  closedChildren: 0,
  coverage: FULL_WEEK,
  ...over,
});

const root = (over: Partial<RootSessionCensusEntry> = {}): RootSessionCensusEntry => ({
  scopeId: "epic-1",
  windowKind: "week",
  windowStart: WEEK_START,
  windowEnd: WEEK_END,
  pool: "meta",
  rootSessionId: "root-a",
  tokens: 600,
  coverage: FULL_WEEK,
  ...over,
});

const population = (over: Partial<SourcePopulationCensusEntry> = {}): SourcePopulationCensusEntry => ({
  windowKind: "week",
  windowStart: WEEK_START,
  windowEnd: WEEK_END,
  unit: "tokens",
  pool: "meta",
  value: 1000,
  coverage: FULL_WEEK,
  ...over,
});

test("closedChildren seam: absent census entry is uncertified (null), never a certified zero", () => {
  expect(certifiedClosedChildren([])(spend())).toBeNull();
  expect(certifiedClosedChildren([closure({ scopeId: "epic-2" })])(spend())).toBeNull();
  expect(certifiedClosedChildren([closure({ windowStart: PREV_START, windowEnd: WEEK_START })])(spend())).toBeNull();
});

test("closedChildren seam: complete census returns its count stamped on the rollup's window and scope", () => {
  expect(certifiedClosedChildren([closure({ closedChildren: 3 })])(spend())).toEqual({
    windowKind: "week",
    windowStart: WEEK_START,
    windowEnd: WEEK_END,
    coverage: "complete",
    attributionVersion: "v1",
    scopeKind: "epic",
    scopeId: "epic-1",
    count: 3,
  });
});

test("closedChildren seam: partial or failed census coverage stays partial", () => {
  expect(certifiedClosedChildren([closure({ coverage: PARTIAL_WEEK })])(spend())!.coverage).toBe("partial");
  expect(certifiedClosedChildren([closure({ coverage: FAILED_WEEK })])(spend())!.coverage).toBe("partial");
  expect(certifiedClosedChildren([closure({ coverage: [] })])(spend())!.coverage).toBe("partial");
});

test("closedChildren seam: conflicting duplicates certify nothing; identical duplicates collapse", () => {
  expect(certifiedClosedChildren([closure(), closure({ closedChildren: 1 })])(spend())).toBeNull();
  expect(certifiedClosedChildren([closure(), closure()])(spend())?.count).toBe(0);
});

test("closedChildren seam: non-integer or negative counts certify nothing", () => {
  expect(certifiedClosedChildren([closure({ closedChildren: -1 })])(spend())).toBeNull();
  expect(certifiedClosedChildren([closure({ closedChildren: 0.5 })])(spend())).toBeNull();
});

test("busiestRoot seam: returns the maximum folded root total for the epic, window, and pool", () => {
  const seam = certifiedBusiestRoot([root(), root({ rootSessionId: "root-b", tokens: 400 })]);
  const record = seam(spend());
  expect(record?.tokens).toBe(600);
  expect(record?.pool).toBe("meta");
  expect(record?.coverage).toBe("complete");
});

test("busiestRoot seam: absent census, wrong pool, and non-epic-token rollups are uncertified", () => {
  expect(certifiedBusiestRoot([])(spend())).toBeNull();
  expect(certifiedBusiestRoot([root({ pool: "other" })])(spend())).toBeNull();
  expect(certifiedBusiestRoot([root()])(spend({ scopeKind: "project" }))).toBeNull();
  expect(certifiedBusiestRoot([root()])(spend({ unit: "usd", pool: null }))).toBeNull();
});

test("busiestRoot seam: identical duplicates of one root collapse; conflicting totals certify nothing", () => {
  expect(certifiedBusiestRoot([root(), root()])(spend())?.tokens).toBe(600);
  expect(certifiedBusiestRoot([root(), root({ tokens: 700 })])(spend())).toBeNull();
});

test("busiestRoot seam: invalid entries certify nothing", () => {
  expect(certifiedBusiestRoot([root({ tokens: Number.NaN })])(spend())).toBeNull();
  expect(certifiedBusiestRoot([root({ tokens: -5 })])(spend())).toBeNull();
  expect(certifiedBusiestRoot([root({ rootSessionId: "  " })])(spend())).toBeNull();
});

test("busiestRoot seam: one partially covered root leaves the maximum uncertified (partial)", () => {
  const seam = certifiedBusiestRoot([root(), root({ rootSessionId: "root-b", tokens: 400, coverage: PARTIAL_WEEK })]);
  expect(seam(spend())?.coverage).toBe("partial");
});

test("sourceTotal seam: returns the population value for the rollup's window, unit, and pool", () => {
  const record = certifiedSourceTotal([population({ value: 4000 })])(
    spend({ scopeKind: "unattributed", scopeId: "unattributed" }),
  );
  expect(record?.value).toBe(4000);
  expect(record?.coverage).toBe("complete");
  expect(record?.unit).toBe("tokens");
  expect(record?.pool).toBe("meta");
});

test("sourceTotal seam: absent entry, pool or unit mismatch, and conflicts are uncertified", () => {
  const unattr = spend({ scopeKind: "unattributed", scopeId: "unattributed" });
  expect(certifiedSourceTotal([])(unattr)).toBeNull();
  expect(certifiedSourceTotal([population({ pool: "other" })])(unattr)).toBeNull();
  expect(certifiedSourceTotal([population({ unit: "share_pct" })])(unattr)).toBeNull();
  expect(certifiedSourceTotal([population(), population({ value: 2000 })])(unattr)).toBeNull();
  expect(certifiedSourceTotal([population({ value: -1 })])(unattr)).toBeNull();
  expect(certifiedSourceTotal([population({ value: Number.NaN })])(unattr)).toBeNull();
});

test("sourceTotal seam: partial census coverage stays partial", () => {
  const unattr = spend({ scopeKind: "unattributed", scopeId: "unattributed" });
  expect(certifiedSourceTotal([population({ coverage: PARTIAL_WEEK })])(unattr)?.coverage).toBe("partial");
});

test("end to end: certified rollup sources fire exactly the constructed burn and dominance anomalies", async () => {
  const now = new Date("2026-10-20T12:00:00Z");
  const deps = {
    readRollups: async () => [spend()],
    closedChildren: certifiedClosedChildren([closure()]),
    busiestRoot: certifiedBusiestRoot([root(), root({ rootSessionId: "root-b", tokens: 400 })]),
  };
  const evidence = await loadCostAnomalyEvidence(deps);
  const fired = evidence.map((e) => detectCostAnomaly(e, now)).filter((i) => i !== null);
  expect(fired.map((i) => i.title)).toEqual([
    "burn without progress: epic:epic-1",
    "single session dominance: epic:epic-1",
  ]);
  // Real closures suppress burn; a busiest root at exactly 50% is not
  // strictly above the dominance threshold.
  const suppressed = await loadCostAnomalyEvidence({
    ...deps,
    closedChildren: certifiedClosedChildren([closure({ closedChildren: 2 })]),
    busiestRoot: certifiedBusiestRoot([root({ tokens: 500 })]),
  });
  expect(suppressed.map((e) => detectCostAnomaly(e, now))).toEqual([null, null]);
  // With the censuses withdrawn, nothing fires: absence is not zero.
  const uncertified = await loadCostAnomalyEvidence({
    ...deps,
    closedChildren: certifiedClosedChildren([]),
    busiestRoot: certifiedBusiestRoot([]),
  });
  expect(uncertified.map((e) => detectCostAnomaly(e, now))).toEqual([null, null]);
});

test("end to end: certified source totals fire exactly the constructed unattributed-growth anomaly", async () => {
  const now = new Date("2026-10-20T12:00:00Z");
  const previous = spend({
    scopeKind: "unattributed",
    scopeId: "unattributed",
    windowStart: PREV_START,
    windowEnd: WEEK_START,
    value: 100,
  });
  const current = spend({ scopeKind: "unattributed", scopeId: "unattributed", value: 300 });
  const totals = certifiedSourceTotal([
    population({ windowStart: PREV_START, windowEnd: WEEK_START, coverage: FULL_PREV }),
    population(),
  ]);
  const evidence = await loadCostAnomalyEvidence({
    readRollups: async () => [previous, current],
    sourceTotal: totals,
  });
  const growth = evidence.filter((e) => e.kind === "unattributed-growth");
  expect(growth).toHaveLength(1);
  // 10% -> 30% unattributed is a 20-point rise, above the 10-point threshold.
  expect(detectCostAnomaly(growth[0]!, now)?.title).toContain("unattributed growth");
  // With the population census withdrawn, both totals are null and
  // the comparison is suppressed: no denominator, no anomaly.
  const uncertified = await loadCostAnomalyEvidence({
    readRollups: async () => [previous, current],
    sourceTotal: certifiedSourceTotal([]),
  });
  expect(uncertified.filter((e) => e.kind === "unattributed-growth").map((e) => detectCostAnomaly(e, now))).toEqual([
    null,
  ]);
});
