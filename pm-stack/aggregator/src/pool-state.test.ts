import { describe, expect, test } from "bun:test";
import { projectPoolState } from "./pool-state.ts";
import type { QuotaCurrentEntry, QuotaCurrentFile, QuotaCurrentWindow } from "./model.ts";

// Oracle: the babysitter's pool-state contract
// (pm-stack/opencode-claimant/babysitter.py — pools keyed by the
// OpenCode config provider id; {walled, percent, state} per pool),
// with the wall rule from derive.ts poolState (any window >= 100%),
// the freshness rule from quota-current.ts (fresh ok within
// QUOTA_CURRENT_FRESH_MS, future skew bounded), and the honesty
// rule from quota-exhaustion.ts (a stale/errored source never
// asserts exhaustion from retained windows).

const NOW = new Date("2026-11-22T22:00:00.000Z");
const FRESH = "2026-11-22T21:55:00.000Z"; // 5 min before NOW
const STALE = "2026-11-22T20:00:00.000Z"; // 2 h before NOW

function window(pctOfLimit: number | null, over: Partial<QuotaCurrentWindow> = {}): QuotaCurrentWindow {
  return {
    name: "weekly",
    value: pctOfLimit,
    pctOfLimit,
    valueLabel: null,
    windowSeconds: null,
    resetsAt: null,
    measuredAt: FRESH,
    source: "test",
    state: "ok",
    ...over,
  };
}

function entry(over: Partial<QuotaCurrentEntry> = {}): QuotaCurrentEntry {
  return {
    providerId: "_claude",
    providerName: "Claude",
    serviceId: "anthropic:claude-pro",
    state: "ok",
    measuredAt: FRESH,
    source: "opencode-quota",
    windows: [window(42)],
    ...over,
  } as QuotaCurrentEntry;
}

function file(providers: QuotaCurrentEntry[]): QuotaCurrentFile {
  return {
    version: 1,
    generatedAt: FRESH,
    source: "opencode-quota",
    heartbeat: { tickAt: FRESH, ok: true, durationMs: 1, perProvider: {} },
    providers,
  };
}

describe("projectPoolState (#1338 pool-state feed)", () => {
  test("keys pools by providerId, never by the aggregator serviceId", () => {
    const out = projectPoolState(file([entry()]), NOW);
    expect(Object.keys(out.pools)).toEqual(["_claude"]);
    expect(out.pools["_claude"]).toEqual({ walled: false, percent: 42, state: "healthy" });
    expect(out.version).toBe(1);
    expect(out.source).toBe("gather-quota");
    expect(out.generatedAt).toBe(FRESH);
  });

  test("a fresh window at 100% walls the pool (derive.ts poolState rule)", () => {
    const out = projectPoolState(file([entry({ windows: [window(42), window(100)] })]), NOW);
    expect(out.pools["_claude"]).toEqual({ walled: true, percent: 100, state: "walled" });
  });

  test("percent is the highest window percent; >= NEAR_PCT reads near", () => {
    const out = projectPoolState(file([entry({ windows: [window(30), window(85)] })]), NOW);
    expect(out.pools["_claude"]).toEqual({ walled: false, percent: 85, state: "near" });
  });

  test("a stale ok entry at its limit asserts no wall and no percent (stale_source honesty)", () => {
    const out = projectPoolState(file([entry({ measuredAt: STALE, windows: [window(100)] })]), NOW);
    expect(out.pools["_claude"]).toEqual({ walled: false, percent: null, state: "stale" });
  });

  test("an error entry surfaces as state, never as a wall", () => {
    const out = projectPoolState(file([entry({ state: "error", windows: [window(100)] })]), NOW);
    expect(out.pools["_claude"]).toEqual({ walled: false, percent: null, state: "error" });
  });

  test("several credentials share a providerId: any fresh ok one at its limit walls the pool", () => {
    const out = projectPoolState(
      file([entry({ credentialId: "a", windows: [window(10)] }), entry({ credentialId: "b", windows: [window(100)] })]),
      NOW,
    );
    expect(out.pools["_claude"]).toEqual({ walled: true, percent: 100, state: "walled" });
  });

  test("a fresh ok sibling decides over a stale walled-looking sibling", () => {
    const out = projectPoolState(
      file([
        entry({ credentialId: "old", measuredAt: STALE, windows: [window(100)] }),
        entry({ credentialId: "new", windows: [window(12)] }),
      ]),
      NOW,
    );
    expect(out.pools["_claude"]).toEqual({ walled: false, percent: 12, state: "healthy" });
  });

  test("an entry measured beyond the future-skew bound is not fresh", () => {
    const future = new Date(NOW.getTime() + 10 * 60_000).toISOString();
    const out = projectPoolState(file([entry({ measuredAt: future, windows: [window(100)] })]), NOW);
    expect(out.pools["_claude"]).toEqual({ walled: false, percent: null, state: "stale" });
  });

  test("windows without a percent contribute nothing; percent stays null", () => {
    const out = projectPoolState(file([entry({ windows: [window(null)] })]), NOW);
    expect(out.pools["_claude"]).toEqual({ walled: false, percent: null, state: "healthy" });
  });

  test("unconfigured and lapsed entries ride as their own state", () => {
    const out = projectPoolState(
      file([
        entry({ providerId: "deepseek", serviceId: "deepseek", state: "unconfigured", windows: [] }),
        entry({ providerId: "_muse", serviceId: "meta", state: "lapsed", windows: [] }),
      ]),
      NOW,
    );
    expect(out.pools["deepseek"]).toEqual({ walled: false, percent: null, state: "unconfigured" });
    expect(out.pools["_muse"]).toEqual({ walled: false, percent: null, state: "lapsed" });
  });

  test("no providers projects to an empty pools record", () => {
    const out = projectPoolState(file([]), NOW);
    expect(out.pools).toEqual({});
  });
});
