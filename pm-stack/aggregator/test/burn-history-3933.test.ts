// #3933: historical burn segments + measured burn rate derivation.
// Oracle: hand-computed segments/rates from the observation series
// below — a completed reset period's segment is its final observed
// `used`; a rate is the token delta of the two most recent
// same-period observations divided by their minutes apart.
import { describe, expect, test } from "bun:test";
import {
  deriveBurnHistory,
  mergeObservations,
  observationsFromPools,
  RATE_WINDOW_MS,
  type PoolWindowObservation,
} from "../src/burn-history.ts";
import { sanitizeQuotaView } from "../src/feed.ts";
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
