// #3933: historical burn segments + measured burn rate derivation.
// Oracle: hand-computed segments/rates from the observation series
// below — a completed reset period's segment is its final observed
// `used`; a rate is the token delta of the two most recent
// same-period observations divided by their minutes apart.
import { describe, expect, test } from "bun:test";
import {
  deriveBurnHistory,
  deriveMeasuredRatesFromPoolTokens,
  mergeObservations,
  mergePoolTokenObservations,
  observationsFromPools,
  MAX_OBSERVATIONS_PER_WINDOW,
  RATE_WINDOW_MS,
  type PoolTokenObservation,
  type PoolWindowObservation,
} from "../src/burn-history.ts";
import { isBurnSegment, sanitizeQuotaView } from "../src/feed.ts";
import type { QuotaPool, QuotaView } from "../src/model.ts";

function obs(pool: string, used: number, at: string, resetsAt: string | null, unit = "tokens"): PoolWindowObservation {
  return { pool, provider: "meta", window: "5h", unit, used, resetsAt, at };
}

const T0 = "2026-11-01T00:00:00Z";
const T1 = "2026-11-01T01:00:00Z";
const T2 = "2026-11-01T02:00:00Z";
const R1 = "2026-11-01T05:00:00Z";
const R2 = "2026-11-01T10:00:00Z";

describe("deriveBurnHistory", () => {
  test("reset boundary closes a segment with the period's final used", () => {
    const { segments } = deriveBurnHistory([
      obs("claude", 100, T0, R1),
      obs("claude", 400, T1, R1),
      obs("claude", 10, T2, R2), // new period: resetsAt moved
    ]);
    expect(segments).toEqual([{ provider: "meta", pool: "claude", from: T0, to: T1, tokens: 400 }]);
  });

  test("a used drop also marks a reset boundary", () => {
    const { segments } = deriveBurnHistory([
      obs("claude", 900, T0, R1),
      obs("claude", 5, T1, R1),
    ]);
    expect(segments).toEqual([{ provider: "meta", pool: "claude", from: T0, to: T0, tokens: 900 }]);
  });

  test("the in-progress period is never emitted as history", () => {
    const { segments } = deriveBurnHistory([obs("claude", 100, T0, R1), obs("claude", 400, T1, R1)]);
    expect(segments).toEqual([]);
  });

  test("measured rate from the most recent same-period pair", () => {
    const { burnTokensPerMin } = deriveBurnHistory([
      obs("claude", 100, T0, R1),
      obs("claude", 700, T1, R1), // +600 tokens over 60 min
    ]);
    expect(burnTokensPerMin.get("claude")).toBe(10);
  });

  test("honest absence: single observation, stale pair, and non-token units yield no rate", () => {
    expect(deriveBurnHistory([obs("a", 100, T0, R1)]).burnTokensPerMin.size).toBe(0);
    const stale = new Date(Date.parse(T0) + RATE_WINDOW_MS + 60000).toISOString();
    expect(deriveBurnHistory([obs("a", 100, T0, R1), obs("a", 200, stale, R1)]).burnTokensPerMin.size).toBe(0);
    expect(
      deriveBurnHistory([obs("a", 100, T0, R1, "usd"), obs("a", 200, T1, R1, "usd")]).burnTokensPerMin.size,
    ).toBe(0);
    expect(deriveBurnHistory([obs("a", 100, T0, R1, "usd"), obs("a", 200, T1, R1, "usd")]).segments).toEqual([]);
    expect(deriveBurnHistory([]).segments).toEqual([]);
  });

  test("a pair straddling a reset is not a rate", () => {
    const { burnTokensPerMin } = deriveBurnHistory([
      obs("claude", 100, T0, R1),
      obs("claude", 400, T1, R1),
      obs("claude", 10, T2, R2),
    ]);
    expect(burnTokensPerMin.has("claude")).toBe(false);
  });
});

describe("observationsFromPools + mergeObservations (collector mapping)", () => {
  const pool: QuotaPool = {
    id: "claude",
    provider: "meta",
    plan: "High Usage",
    state: "healthy",
    windows: [{ name: "5h", unit: "tokens", limit: 100000, used: 500, resetsAt: R1 }],
  };

  test("live pools map to observations at the collection instant", () => {
    expect(observationsFromPools([pool], T0)).toEqual([
      { pool: "claude", provider: "meta", window: "5h", unit: "tokens", used: 500, resetsAt: R1, at: T0 },
    ]);
  });

  test("merge dedupes identical points and sorts by time", () => {
    const a = obs("claude", 100, T0, R1);
    const b = obs("claude", 200, T1, R1);
    const merged = mergeObservations([b], [a, b], Date.parse(T2));
    expect(merged).toEqual([a, b]);
  });
});

describe("measured rates from live session-token attribution (PR #7354 Codex P2)", () => {
  const tok = (pool: string, activeTokens: number, at: string): PoolTokenObservation => ({
    pool,
    provider: "meta",
    activeTokens,
    at,
  });

  test("rate is the delta of consecutive pool token totals over their minutes", () => {
    const rates = deriveMeasuredRatesFromPoolTokens([tok("claude", 1000, T0), tok("claude", 1600, T1)]);
    expect(rates.get("claude")).toBe(10);
  });

  test("shrinking active set, stale pair, and single read yield no rate", () => {
    expect(deriveMeasuredRatesFromPoolTokens([tok("a", 900, T0), tok("a", 100, T1)]).size).toBe(0);
    const stale = new Date(Date.parse(T0) + RATE_WINDOW_MS + 60000).toISOString();
    expect(deriveMeasuredRatesFromPoolTokens([tok("a", 100, T0), tok("a", 200, stale)]).size).toBe(0);
    expect(deriveMeasuredRatesFromPoolTokens([tok("a", 100, T0)]).size).toBe(0);
    const merged = mergePoolTokenObservations([tok("a", 100, T0)], [tok("a", 100, T0)], Date.parse(T2));
    expect(merged).toHaveLength(1);
  });
});

// Property-based coverage (PR #7354 Codex P1; testing doctrine:
// invariants + a generator + a decidable oracle). Generator: a
// seeded PRNG (mulberry32) producing arbitrary observation series;
// oracles: the feed's own isBurnSegment validator, hand recomputation
// of the last-pair rate, and the merge/retention bounds themselves.
describe("burn-history invariants over generated series", () => {
  function mulberry32(seed: number) {
    return () => {
      seed |= 0;
      seed = (seed + 0x6d2b79f5) | 0;
      let t = Math.imul(seed ^ (seed >>> 15), 1 | seed);
      t = (t + Math.imul(t ^ (t >>> 7), 61 | t)) ^ t;
      return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
    };
  }

  function genSeries(rand: () => number, pool: string, n: number): PoolWindowObservation[] {
    const out: PoolWindowObservation[] = [];
    let t = Date.parse("2026-10-01T00:00:00Z");
    let used = 0;
    let reset = "2026-10-01T05:00:00Z";
    for (let i = 0; i < n; i++) {
      t += Math.floor(rand() * 90 + 1) * 60000;
      if (rand() < 0.25) {
        // reset: schedule moves and usage restarts low
        reset = new Date(t + 5 * 3600000).toISOString();
        used = Math.floor(rand() * 50);
      } else {
        used += Math.floor(rand() * 500);
      }
      out.push(obs(pool, used, new Date(t).toISOString(), reset, rand() < 0.8 ? "tokens" : "usd"));
    }
    return out;
  }

  test("every derived segment passes the feed validator and respects period bounds", () => {
    for (let seed = 1; seed <= 40; seed++) {
      const rand = mulberry32(seed);
      const series = genSeries(rand, "claude", 30);
      const { segments } = deriveBurnHistory(series);
      for (const seg of segments) {
        expect(isBurnSegment(seg)).toBe(true);
        const inPeriod = series.filter((o) => o.at >= seg.from && o.at <= seg.to);
        expect(seg.tokens <= Math.max(...inPeriod.map((o) => o.used))).toBe(true);
        expect(inPeriod.some((o) => o.used === seg.tokens && o.at === seg.to)).toBe(true);
      }
    }
  });

  test("per-window isolation: another pool's series never changes a pool's derivation", () => {
    for (let seed = 100; seed < 120; seed++) {
      const rand = mulberry32(seed);
      const a = genSeries(rand, "a", 25);
      const b = genSeries(rand, "b", 25);
      const solo = deriveBurnHistory(a);
      const mixed = deriveBurnHistory([...b, ...a]);
      expect(mixed.segments.filter((s) => s.pool === "a")).toEqual(solo.segments);
      expect(mixed.burnTokensPerMin.get("a")).toBe(solo.burnTokensPerMin.get("a"));
    }
  });

  test("merge is idempotent, capped, and prunes past retention", () => {
    const rand = mulberry32(7);
    const now = Date.parse("2026-11-05T00:00:00Z");
    const fresh = genSeries(rand, "claude", 40);
    const once = mergeObservations([], fresh, now);
    const twice = mergeObservations(once, fresh, now);
    expect(twice).toEqual(once);
    const many: PoolWindowObservation[] = [];
    for (let i = 0; i < MAX_OBSERVATIONS_PER_WINDOW + 50; i++) {
      many.push(obs("claude", i, new Date(Date.parse("2026-11-01T00:00:00Z") + i * 60000).toISOString(), R1));
    }
    expect(mergeObservations([], many, Date.parse("2026-11-02T00:00:00Z")).length).toBe(MAX_OBSERVATIONS_PER_WINDOW);
    const old = obs("claude", 1, "2025-01-01T00:00:00Z", R1);
    expect(mergeObservations([], [old], now)).toEqual([]);
  });

  test("a reset between the last pair always excludes the rate", () => {
    for (let seed = 200; seed < 230; seed++) {
      const rand = mulberry32(seed);
      const series = genSeries(rand, "claude", 12).filter((o) => o.unit === "tokens");
      if (series.length < 2) continue;
      const last = series[series.length - 1]!;
      const withReset: PoolWindowObservation[] = [
        ...series.slice(0, -1),
        { ...last, used: 1, resetsAt: "2027-01-01T00:00:00Z" },
      ];
      expect(deriveBurnHistory(withReset).burnTokensPerMin.has("claude")).toBe(false);
    }
  });
});

describe("measured rates survive feed sanitization (#2429/#3921/#4546 reconciliation)", () => {
  test("a valid measured rate is kept; only invalid rates drop", () => {
    const view: QuotaView = {
      pools: [
        {
          id: "claude",
          provider: "meta",
          plan: "High Usage",
          state: "healthy",
          windows: [{ name: "5h", unit: "tokens", limit: 100000, used: 500, resetsAt: R1 }],
          burnTokensPerMin: 10,
        },
        {
          id: "broken",
          provider: "meta",
          plan: "x",
          state: "healthy",
          windows: [],
          burnTokensPerMin: -3,
        },
      ],
      burn: [{ provider: "meta", pool: "claude", from: T0, to: T1, tokens: 400 }],
      choices: { hasHeadroom: [], burnBeforeReset: [], doNotStart: [] },
    };
    const s = sanitizeQuotaView(view);
    expect(s.view.pools[0]?.burnTokensPerMin).toBe(10);
    expect(s.view.pools[1]?.burnTokensPerMin).toBeUndefined();
    expect(s.view.burn).toHaveLength(1);
    expect(s.droppedBurnTokensPerMin).toBe(1);
  });
});
