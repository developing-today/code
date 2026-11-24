import { describe, expect, it } from "vitest";
import {
  DEFAULT_ADMISSION_CONFIG,
  HeatTracker,
  decideAdmission,
  planEvictions,
  quotaShares,
  type AdmissionConfig,
  type EvictionEntry,
} from "../src/admission";

// Oracle for every rule below: the #9134 design, §3 (admission) and
// §5 (eviction). Expected values are computed from the design text,
// not from the implementation.

const CFG: AdmissionConfig = DEFAULT_ADMISSION_CONFIG;
const MIN = 60 * 1000;

describe("HeatTracker (§3: eligible after 2 requests / 10 min)", () => {
  it("counts the first request without making the key eligible", () => {
    const heat = new HeatTracker(CFG);
    expect(heat.record("t1", "k", 0)).toBe(1);
    expect(heat.eligible("t1", "k", 0)).toBe(false);
  });

  it("makes the key eligible on the second request inside the window", () => {
    const heat = new HeatTracker(CFG);
    heat.record("t1", "k", 0);
    expect(heat.record("t1", "k", 5 * MIN)).toBe(2);
    expect(heat.eligible("t1", "k", 5 * MIN)).toBe(true);
  });

  it("does not combine requests across the window boundary", () => {
    const heat = new HeatTracker(CFG);
    heat.record("t1", "k", 0);
    // Exactly one window later the first request has aged out
    // (a timestamp counts iff it is younger than the window).
    expect(heat.record("t1", "k", 10 * MIN)).toBe(1);
    expect(heat.eligible("t1", "k", 10 * MIN)).toBe(false);
  });

  it("keeps tenants and keys separate", () => {
    const heat = new HeatTracker(CFG);
    heat.record("t1", "k", 0);
    heat.record("t2", "k", 0);
    heat.record("t1", "other", 0);
    expect(heat.count("t1", "k", 0)).toBe(1);
    expect(heat.eligible("t1", "k", 0)).toBe(false);
  });

  it("reads the threshold and window from config, not constants", () => {
    const heat = new HeatTracker({ ...CFG, heatRequests: 3, heatWindowMs: MIN });
    heat.record("t1", "k", 0);
    heat.record("t1", "k", 1000);
    expect(heat.eligible("t1", "k", 1000)).toBe(false);
    heat.record("t1", "k", 2000);
    expect(heat.eligible("t1", "k", 2000)).toBe(true);
    // The custom 1-minute window ages the t=0 hit out by t=60.5s,
    // leaving the t=1s and t=2s hits inside.
    expect(heat.count("t1", "k", 60_500)).toBe(2);
  });
});

describe("decideAdmission — heavy-query exception (§3)", () => {
  const cold = { payloadBytes: 1024, sourceLatencyMs: 0, rowsRead: 0, isNegative: false, heatCount: 1 };

  it("admits a first request whose source latency exceeds the threshold", () => {
    expect(decideAdmission({ ...cold, sourceLatencyMs: 251 }, CFG)).toEqual({
      admit: true,
      reason: "heavy",
      ttlMs: null,
    });
  });

  it("admits a first request whose rows-read exceeds the threshold", () => {
    expect(decideAdmission({ ...cold, rowsRead: 10_001 }, CFG)).toEqual({
      admit: true,
      reason: "heavy",
      ttlMs: null,
    });
  });

  it("treats the thresholds as strict (at-threshold is not heavy)", () => {
    // Design wording is "latency > 250 ms or rows-read > 10,000".
    expect(decideAdmission({ ...cold, sourceLatencyMs: 250, rowsRead: 10_000 }, CFG)).toEqual({
      admit: false,
      reason: "cold",
      ttlMs: null,
    });
  });

  it("reads the heavy thresholds from config", () => {
    const cfg = { ...CFG, heavyLatencyMs: 100 };
    expect(decideAdmission({ ...cold, sourceLatencyMs: 150 }, cfg).admit).toBe(true);
    expect(decideAdmission({ ...cold, sourceLatencyMs: 150 }, CFG).admit).toBe(false);
  });
});

describe("decideAdmission — heat, size cap, negatives (§3)", () => {
  const base = { payloadBytes: 1024, sourceLatencyMs: 0, rowsRead: 0, isNegative: false, heatCount: 1 };

  it("refuses a cold, light first request", () => {
    expect(decideAdmission(base, CFG)).toEqual({ admit: false, reason: "cold", ttlMs: null });
  });

  it("admits once the heat count reaches the configured threshold", () => {
    expect(decideAdmission({ ...base, heatCount: 2 }, CFG)).toEqual({
      admit: true,
      reason: "heat",
      ttlMs: null,
    });
  });

  it("refuses an entry larger than the size cap even when heavy and hot", () => {
    const big = { ...base, payloadBytes: CFG.maxEntryBytes + 1, sourceLatencyMs: 9999, heatCount: 99 };
    expect(decideAdmission(big, CFG)).toEqual({ admit: false, reason: "oversized", ttlMs: null });
  });

  it("admits an entry exactly at the size cap (the rule is 'larger than')", () => {
    expect(decideAdmission({ ...base, payloadBytes: CFG.maxEntryBytes, heatCount: 2 }, CFG).admit).toBe(true);
  });

  it("admits a negative result through the heat gate with the negative TTL", () => {
    expect(decideAdmission({ ...base, isNegative: true, heatCount: 2 }, CFG)).toEqual({
      admit: true,
      reason: "heat",
      ttlMs: 30 * 1000,
    });
  });

  it("admits a heavy negative result with the negative TTL", () => {
    expect(decideAdmission({ ...base, isNegative: true, sourceLatencyMs: 500 }, CFG)).toEqual({
      admit: true,
      reason: "heavy",
      ttlMs: 30 * 1000,
    });
  });

  it("does not admit a single cold negative poll (one-hit traffic)", () => {
    expect(decideAdmission({ ...base, isNegative: true }, CFG)).toEqual({
      admit: false,
      reason: "cold",
      ttlMs: null,
    });
  });

  it("reads the negative TTL from config", () => {
    const cfg = { ...CFG, negativeTtlMs: 5000 };
    expect(decideAdmission({ ...base, isNegative: true, heatCount: 2 }, cfg).ttlMs).toBe(5000);
  });
});

describe("quotaShares (§3: tier weights 1:4:16, 5% floor)", () => {
  it("splits purely by weight when the floor is configured off", () => {
    const cfg = { ...CFG, tenantFloorFraction: 0 };
    const shares = quotaShares(
      [
        { tenantId: "a", tier: 0 },
        { tenantId: "b", tier: 1 },
        { tenantId: "c", tier: 2 },
      ],
      2_100_000,
      cfg,
    );
    expect(shares.get("a")).toBe(100_000);
    expect(shares.get("b")).toBe(400_000);
    expect(shares.get("c")).toBe(1_600_000);
  });

  it("reserves the floor per tenant before splitting the remainder by weight", () => {
    // Budget 1,000,000; floor 5% = 50,000 each; remaining 900,000
    // split 1:16 → 52,941 / 847,058 (floored), remainder 1 to first.
    const shares = quotaShares(
      [
        { tenantId: "small", tier: 0 },
        { tenantId: "big", tier: 2 },
      ],
      1_000_000,
      CFG,
    );
    expect(shares.get("small")).toBe(50_000 + 52_941 + 1);
    expect(shares.get("big")).toBe(50_000 + 847_058);
    expect((shares.get("small") ?? 0) + (shares.get("big") ?? 0)).toBe(1_000_000);
  });

  it("guarantees the floor: a lone small tenant among large ones keeps 5%", () => {
    const tenants = [
      { tenantId: "small", tier: 0 },
      { tenantId: "big1", tier: 2 },
      { tenantId: "big2", tier: 2 },
    ];
    const shares = quotaShares(tenants, 1_000_000, CFG);
    // Pure weight would give the small tenant 1/33 ≈ 30,303 < floor.
    expect(shares.get("small")).toBeGreaterThanOrEqual(50_000);
  });

  it("falls back to pure weight when floors alone would exceed the budget", () => {
    // 25 active tenants × 5% floor = 125% of the budget: the floor
    // is impossible, so the budget splits by weight (equal here).
    const tenants = Array.from({ length: 25 }, (_, i) => ({ tenantId: `t${i}`, tier: 0 }));
    const shares = quotaShares(tenants, 1_000_000, CFG);
    for (const t of tenants) expect(shares.get(t.tenantId)).toBe(40_000);
  });

  it("rejects a tier outside the configured weights", () => {
    expect(() => quotaShares([{ tenantId: "x", tier: 7 }], 1000, CFG)).toThrow(RangeError);
  });

  it("returns no shares for no tenants", () => {
    expect(quotaShares([], 1_000_000, CFG).size).toBe(0);
  });
});

describe("planEvictions (§5: per-tenant under pressure, B/C before A)", () => {
  const entry = (over: Partial<EvictionEntry>): EvictionEntry => ({
    tenantId: "t1",
    cacheKey: "k",
    entryClass: "B",
    payloadBytes: 100,
    lastHitAt: 0,
    hitCount: 0,
    ...over,
  });

  it("evicts nothing while the database is within budget (quota is borrowable)", () => {
    const entries = [entry({ cacheKey: "a", payloadBytes: 900 })];
    const shares = new Map([["t1", 100]]);
    expect(planEvictions(entries, shares, 1000)).toEqual([]);
  });

  it("sheds only the over-share tenant, down to its share", () => {
    const entries = [
      entry({ tenantId: "over", cacheKey: "o1", payloadBytes: 300, lastHitAt: 1 }),
      entry({ tenantId: "over", cacheKey: "o2", payloadBytes: 300, lastHitAt: 2 }),
      entry({ tenantId: "over", cacheKey: "o3", payloadBytes: 300, lastHitAt: 3 }),
      entry({ tenantId: "under", cacheKey: "u1", payloadBytes: 500, lastHitAt: 1 }),
    ];
    const shares = new Map([
      ["over", 600],
      ["under", 600],
    ]);
    // Total 1400 > budget 1200 → pressure. "over" must shed 300:
    // exactly its coldest entry. "under" is untouched.
    const plan = planEvictions(entries, shares, 1200);
    expect(plan.map((e) => e.cacheKey)).toEqual(["o1"]);
  });

  it("evicts Class B and C before Class A, LRU inside each class group", () => {
    const entries = [
      entry({ cacheKey: "a-coldest", entryClass: "A", lastHitAt: 1 }),
      entry({ cacheKey: "b-mid", entryClass: "B", lastHitAt: 50 }),
      entry({ cacheKey: "c-old", entryClass: "C", lastHitAt: 10 }),
      entry({ cacheKey: "b-new", entryClass: "B", lastHitAt: 90 }),
      entry({ cacheKey: "a-recent", entryClass: "A", lastHitAt: 99 }),
    ];
    const shares = new Map([["t1", 200]]);
    // Total 500 > budget 400 → pressure; t1 must shed 300 of 500.
    // Order: C/B by lastHitAt (c-old 10, b-mid 50, b-new 90), then A
    // — the coldest A goes last despite being the coldest overall.
    const plan = planEvictions(entries, shares, 400);
    expect(plan.map((e) => e.cacheKey)).toEqual(["c-old", "b-mid", "b-new"]);
  });

  it("reaches into Class A only after B and C are exhausted", () => {
    const entries = [
      entry({ cacheKey: "b1", entryClass: "B", lastHitAt: 5 }),
      entry({ cacheKey: "a1", entryClass: "A", lastHitAt: 1 }),
      entry({ cacheKey: "a2", entryClass: "A", lastHitAt: 2 }),
    ];
    const shares = new Map([["t1", 0]]);
    const plan = planEvictions(entries, shares, 200);
    expect(plan.map((e) => e.cacheKey)).toEqual(["b1", "a1", "a2"]);
  });

  it("sheds everything for a tenant with no share entry", () => {
    const entries = [
      entry({ tenantId: "ghost", cacheKey: "g1", payloadBytes: 100 }),
      entry({ tenantId: "t1", cacheKey: "k1", payloadBytes: 950 }),
    ];
    const shares = new Map([["t1", 1000]]);
    const plan = planEvictions(entries, shares, 1000);
    expect(plan.map((e) => e.cacheKey)).toEqual(["g1"]);
  });

  it("breaks lastHitAt ties by hit count, then key, deterministically", () => {
    const entries = [
      entry({ cacheKey: "zz", lastHitAt: 7, hitCount: 1 }),
      entry({ cacheKey: "aa", lastHitAt: 7, hitCount: 1 }),
      entry({ cacheKey: "mm", lastHitAt: 7, hitCount: 5 }),
    ];
    const shares = new Map([["t1", 0]]);
    const plan = planEvictions(entries, shares, 100);
    expect(plan.map((e) => e.cacheKey)).toEqual(["aa", "zz", "mm"]);
  });
});
