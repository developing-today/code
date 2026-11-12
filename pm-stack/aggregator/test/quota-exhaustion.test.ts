// #3983: per-pool quota exhaustion intervals over the #3933 live
// contract. Oracle: hand-computed pair rates from the observation
// series below — rates are consecutive same-period deltas per minute;
// P50/P80 are nearest-rank; exhaustion = now + remaining / rate.
import { describe, expect, test } from "bun:test";
import type { PoolWindowObservation } from "../src/burn-history.ts";
import type { QuotaPool } from "../src/model.ts";
import { deriveExhaustionForecasts } from "../src/quota-exhaustion.ts";
import { overlayQuotaCurrent } from "../src/quota-current.ts";
import type { QuotaCurrentFile, QuotaView } from "../src/model.ts";

const NOW = new Date("2026-11-01T02:00:00Z");
const RESET = "2026-11-01T12:00:00Z"; // 10h after NOW

function obs(used: number, at: string, resetsAt: string | null = RESET): PoolWindowObservation {
  return { pool: "p1", provider: "meta", window: "5h", unit: "tokens", used, resetsAt, at };
}

function pool(partial: Partial<QuotaPool> = {}, windowPartial: Record<string, unknown> = {}): QuotaPool {
  return {
    id: "p1",
    provider: "meta",
    plan: "pro",
    state: "ok",
    windows: [
      { name: "5h", unit: "tokens", limit: 10000, used: 4000, resetsAt: RESET, ...windowPartial },
    ],
    freshness: { measuredAt: "2026-11-01T01:59:00Z", source: "test-gatherer", state: "ok" },
    ...partial,
  } as QuotaPool;
}

// Pair rates 100, 200, 300 tokens/min; newest obs 30min before NOW
// (inside RATE_WINDOW_MS). Pool remaining is 10000 - 4000 = 6000.
const clean = [
  obs(1000, "2026-11-01T01:00:00Z"),
  obs(2000, "2026-11-01T01:10:00Z"), // +1000 over 10min = 100/min
  obs(4000, "2026-11-01T01:20:00Z"), // +2000 over 10min = 200/min
  obs(7000, "2026-11-01T01:30:00Z"), // +3000 over 10min = 300/min
];

describe("deriveExhaustionForecasts (#3983)", () => {
  test("measured exhaustion before reset: interval from P50/P80 rates", () => {
    const [f] = deriveExhaustionForecasts([pool()], clean, NOW);
    expect(f.state).toBe("forecast");
    expect(f.remaining).toBe(6000);
    expect(f.ratesPerMin).toEqual({ p50: 200, p80: 300 });
    expect(f.p50ExhaustsAt).toBe("2026-11-01T02:30:00.000Z"); // 6000/200 = 30min
    expect(f.p80ExhaustsAt).toBe("2026-11-01T02:20:00.000Z"); // 6000/300 = 20min
    expect(f.p50VsReset).toBe("before");
    expect(f.p80VsReset).toBe("before");
    expect(f.rateWindowMs).toBe(120 * 60 * 1000);
    expect(f.source).toBe("test-gatherer");
    expect(f.sampleRates).toBe(3);
  });

  test("slow measured burn puts P50 after reset while P80 lands before", () => {
    const slow = [
      obs(3900, "2026-11-01T01:00:00Z"),
      obs(3950, "2026-11-01T01:10:00Z"), // 5/min
      obs(4000, "2026-11-01T01:20:00Z"), // 5/min
      obs(7000, "2026-11-01T01:30:00Z"), // 300/min
    ];
    const [f] = deriveExhaustionForecasts([pool()], slow, NOW);
    expect(f.state).toBe("forecast");
    expect(f.ratesPerMin).toEqual({ p50: 5, p80: 300 });
    expect(f.p50VsReset).toBe("after"); // 6000/5 = 20h > reset in 10h
    expect(f.p80VsReset).toBe("before");
  });

  test("reset boundary: pairs straddling a reset measure nothing for the current window", () => {
    const straddling = [
      obs(9000, "2026-11-01T01:00:00Z", "2026-11-01T01:15:00Z"), // prior period
      obs(100, "2026-11-01T01:20:00Z", RESET), // current period starts over
      obs(400, "2026-11-01T01:30:00Z", RESET), // one current-period pair only
    ];
    const [f] = deriveExhaustionForecasts([pool()], straddling, NOW);
    expect(f.state).toBe("unknown");
    expect(f.reason).toBe("insufficient_observations");
  });

  test("stale source is an explicit unknown, never a forecast", () => {
    const stale = pool({ freshness: { measuredAt: "2026-11-01T00:00:00Z", source: "test-gatherer", state: "stale" } });
    const [f] = deriveExhaustionForecasts([stale], clean, NOW);
    expect(f.state).toBe("unknown");
    expect(f.reason).toBe("stale_source");
  });

  test("missing limit and missing reset are explicit unknowns", () => {
    const [noLimit] = deriveExhaustionForecasts([pool({}, { limit: null })], clean, NOW);
    expect(noLimit).toMatchObject({ state: "unknown", reason: "missing_limit", remaining: null });
    const [noReset] = deriveExhaustionForecasts([pool({}, { resetsAt: null })], clean, NOW);
    expect(noReset).toMatchObject({ state: "unknown", reason: "missing_reset", remaining: 6000 });
  });

  test("no measured burn history is an explicit unknown", () => {
    const [f] = deriveExhaustionForecasts([pool()], [], NOW);
    expect(f).toMatchObject({ state: "unknown", reason: "no_measured_burn" });
    const old = [obs(1000, "2026-10-31T20:00:00Z"), obs(2000, "2026-10-31T20:10:00Z")];
    const [staleObs] = deriveExhaustionForecasts([pool()], old, NOW);
    expect(staleObs).toMatchObject({ state: "unknown", reason: "no_measured_burn" });
  });

  test("zero measured rate is an explicit unknown, not a fabricated date", () => {
    const flat = [obs(4000, "2026-11-01T01:00:00Z"), obs(4000, "2026-11-01T01:10:00Z"), obs(4000, "2026-11-01T01:20:00Z")];
    const [f] = deriveExhaustionForecasts([pool()], flat, NOW);
    expect(f).toMatchObject({ state: "unknown", reason: "zero_rate" });
  });

  test("limit already reached reports exhausted at the collection instant", () => {
    const [f] = deriveExhaustionForecasts([pool({}, { used: 10000 })], clean, NOW);
    expect(f.state).toBe("exhausted");
    expect(f.remaining).toBe(0);
    expect(f.exhaustsAt).toBe(NOW.toISOString());
  });

  test("a passed reset is unknown even when the dated window is at its limit", () => {
    const [f] = deriveExhaustionForecasts(
      [pool({}, { used: 10000, resetsAt: "2026-11-01T01:59:00Z" })],
      clean,
      NOW,
    );
    expect(f).toMatchObject({ state: "unknown", reason: "reset_passed", remaining: 0 });
    expect(f.exhaustsAt).toBeUndefined();
  });

  test("the pool carries its forecasts for view consumers", () => {
    const p = pool();
    deriveExhaustionForecasts([p], clean, NOW);
    expect(p.exhaustion).toHaveLength(1);
    expect(p.exhaustion?.[0].state).toBe("forecast");
  });

  test("stale or errored source at the limit is unknown, never exhausted (Codex P2, PR #8522)", () => {
    for (const state of ["stale", "error"] as const) {
      const p = pool({}, { used: 10000 });
      p.freshness = { measuredAt: "2026-11-01T00:00:00Z", source: "test-gatherer", state };
      const [f] = deriveExhaustionForecasts([p], clean, NOW);
      expect(f).toMatchObject({ state: "unknown", reason: "stale_source", remaining: 0 });
    }
  });

  test("overlay replacing windows drops forecasts derived from the old windows (Codex P2, PR #8522)", () => {
    const p = pool();
    deriveExhaustionForecasts([p], clean, NOW);
    expect(p.exhaustion).toHaveLength(1);
    const view: QuotaView = {
      pools: [{ ...p, id: "p1svc" }],
      burn: [],
      choices: { hasHeadroom: [], burnBeforeReset: [], doNotStart: [] },
    };
    const file: QuotaCurrentFile = {
      version: 1,
      generatedAt: NOW.toISOString(),
      source: "test-gatherer",
      heartbeat: { tickAt: NOW.toISOString(), ok: true, durationMs: 1, perProvider: {} },
      providers: [
        {
          providerId: "meta",
          providerName: "Meta",
          serviceId: "p1svc",
          state: "ok",
          measuredAt: NOW.toISOString(),
          source: "test-gatherer",
          windows: [{ name: "5h", pctOfLimit: 10, resetsAt: RESET }],
        },
      ],
    };
    const out = overlayQuotaCurrent(view, file, NOW);
    expect(out.pools[0].windows[0].used).toBe(10);
    expect(out.pools[0].exhaustion).toBeUndefined();
  });

  // Property coverage (Codex P1, PR #8522). Oracle: the invariants
  // themselves over generated histories from a seeded PRNG (no new
  // dependency; the seed makes every failure reproducible).
  test("properties: P80 never exhausts later than P50; foreign/reset-straddling obs never change a forecast", () => {
    let seed = 0x3983;
    const rnd = () => {
      seed = (seed * 1103515245 + 12345) & 0x7fffffff;
      return seed / 0x7fffffff;
    };
    for (let iter = 0; iter < 200; iter++) {
      const n = 3 + Math.floor(rnd() * 6); // 3..8 observations -> 2..7 pair rates
      const series: PoolWindowObservation[] = [];
      let used = Math.floor(rnd() * 2000);
      let t = Date.parse("2026-11-01T00:30:00Z");
      for (let i = 0; i < n; i++) {
        series.push(obs(used, new Date(t).toISOString()));
        used += Math.floor(rnd() * 900);
        t += (5 + Math.floor(rnd() * 10)) * 60000;
      }
      const newestMs = Date.parse(series[series.length - 1].at);
      const now = new Date(newestMs + 30 * 60000);
      const mkPool = () => pool({}, { used: Math.min(used, 9000) });
      const [f] = deriveExhaustionForecasts([mkPool()], series, now);
      if (f.state === "forecast") {
        expect(f.ratesPerMin!.p80).toBeGreaterThanOrEqual(f.ratesPerMin!.p50);
        expect(Date.parse(f.p80ExhaustsAt!)).toBeLessThanOrEqual(Date.parse(f.p50ExhaustsAt!));
      }
      // Foreign-pool observations and previous-reset observations are noise:
      const noisy = [
        ...series,
        { pool: "other", provider: "meta", window: "5h", unit: "tokens", used: 99999, resetsAt: RESET, at: series[0].at },
        obs(99999, series[0].at, "2026-10-31T12:00:00Z"),
      ];
      const [g] = deriveExhaustionForecasts([mkPool()], noisy, now);
      expect(g).toEqual(f);
    }
  });
});
