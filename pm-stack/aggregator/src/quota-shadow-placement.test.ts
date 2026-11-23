import { describe, expect, test } from "bun:test";
import { buildShadowPlacement, liveOrderingFor } from "./quota-shadow-placement.ts";
import type { QuotaForecastFile, QuotaForecastPool, QuotaForecastWindow } from "./quota-forecast.ts";
import type { QuotaCurrentEntry, QuotaCurrentFile, QuotaCurrentWindow } from "./model.ts";

// Oracle: the Stage-3 design D3 (doc/2026-11-22T22-35-00Z_design_
// quota-plane-stage-3, #12697): the forecast-informed ordering —
// burn-by-expiry adjusted so a pool projected to wall before its
// reset is demoted BEFORE it reaches the caution zone (NEAR_PCT,
// derive.ts) — is computed beside the live ordering and logged as a
// per-decision diff record. The live ordering is a function of the
// measured data (quota-current) ONLY: no forecast input, however
// alarming or however broken, may change it. S2's named findings
// bind consumption: the bare opencode-go pool's monthly forecast is
// a cross-workspace mixing artifact and is never consumed (the
// per-workspace rows are the real ones); a series the forecast
// itself marks insufficient-history is logged, never consumed.

const NOW = new Date("2026-11-22T22:00:00.000Z");
const NOW_MS = NOW.getTime();
const FRESH = "2026-11-22T21:55:00.000Z"; // 5 min before NOW
const STALE = "2026-11-22T19:00:00.000Z"; // 3 h before NOW
const HOUR = 3_600_000;
const DAY = 24 * HOUR;
const iso = (ms: number) => new Date(ms).toISOString();

function window(pctOfLimit: number | null, over: Partial<QuotaCurrentWindow> = {}): QuotaCurrentWindow {
  return {
    name: "weekly",
    value: pctOfLimit,
    pctOfLimit,
    valueLabel: null,
    windowSeconds: null,
    resetsAt: iso(NOW_MS + 10 * HOUR),
    measuredAt: FRESH,
    source: "opencode-quota",
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
    windows: [window(40)],
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

function forecastWindow(over: Partial<QuotaForecastWindow> = {}): QuotaForecastWindow {
  return {
    name: "weekly",
    state: "no-wall-before-reset",
    ratePtsPerHour: 1,
    samples: 12,
    projectedWallAt: null,
    durationUntilWallMs: null,
    wallsBeforeReset: false,
    pctNow: 40,
    measuredAt: FRESH,
    resetsAt: iso(NOW_MS + 10 * HOUR),
    ...over,
  };
}

function forecastPool(providerId: string, serviceId: string, windows: QuotaForecastWindow[]): QuotaForecastPool {
  return { providerId, serviceId, plan: null, windows };
}

function forecast(pools: QuotaForecastPool[]): QuotaForecastFile {
  const byProvider: Record<string, QuotaForecastPool[]> = {};
  for (const p of pools) (byProvider[p.providerId] ??= []).push(p);
  return { version: 1, generatedAt: FRESH, source: "gather-quota", pools: byProvider };
}

/** The five-pool measured fixture: two open pools (B expires
 * sooner), one unknown (errored read), one caution (90%), one
 * walled (100%). */
function fivePools(): QuotaCurrentFile {
  return file([
    entry({ providerId: "_claude", windows: [window(40, { resetsAt: iso(NOW_MS + 10 * HOUR) })] }),
    entry({
      providerId: "_openai.main",
      providerName: "OpenAI",
      serviceId: "_openai.main",
      windows: [window(30, { resetsAt: iso(NOW_MS + 2 * HOUR) })],
    }),
    entry({
      providerId: "deepseek",
      providerName: "DeepSeek",
      serviceId: "deepseek",
      state: "error",
      windows: [window(10, { measuredAt: STALE })],
    }),
    entry({
      providerId: "cline-pass",
      providerName: "Cline",
      serviceId: "cline-pass",
      windows: [window(90, { resetsAt: iso(NOW_MS + 30 * HOUR) })],
    }),
    entry({
      providerId: "opendesign",
      providerName: "OpenDesign",
      serviceId: "opendesign",
      windows: [window(100, { resetsAt: iso(NOW_MS + 5 * HOUR) })],
    }),
  ]);
}

describe("live ordering (measured data only)", () => {
  test("burn-by-expiry with measured demotions: soonest reset first, caution below open, unknown above caution, walled last", () => {
    const record = buildShadowPlacement(fivePools(), null, NOW);
    expect(record.liveOrdering).toEqual(["_openai.main", "_claude", "deepseek", "cline-pass", "opendesign"]);
    // With no forecast the shadow ordering IS the live ordering.
    expect(record.shadowOrdering).toEqual(record.liveOrdering);
    expect(record.summary.identical).toBe(true);
    const tiers = Object.fromEntries(record.pools.map((p) => [p.providerId, p.liveTier]));
    expect(tiers).toEqual({
      "_openai.main": "open",
      _claude: "open",
      deepseek: "unknown",
      "cline-pass": "caution",
      opendesign: "walled",
    });
  });

  test("liveOrderingFor is the same ordering the record carries", () => {
    const current = fivePools();
    expect(liveOrderingFor(current, NOW)).toEqual(buildShadowPlacement(current, null, NOW).liveOrdering);
  });
});

describe("shadow ordering (forecast-adjusted, logged only)", () => {
  test("a pool projected to wall before its reset is demoted before the caution zone", () => {
    const current = fivePools();
    const fc = forecast([
      forecastPool(
        "_claude",
        "anthropic:claude-pro",
        [
          forecastWindow({
            state: "projected",
            wallsBeforeReset: true,
            projectedWallAt: NOW_MS + 5 * HOUR,
            durationUntilWallMs: 5 * HOUR,
            pctNow: 40,
          }),
        ],
      ),
    ]);
    const record = buildShadowPlacement(current, fc, NOW);
    // Live ordering untouched by the forecast.
    expect(record.liveOrdering).toEqual(["_openai.main", "_claude", "deepseek", "cline-pass", "opendesign"]);
    // Shadow: _claude (40%, nowhere near the 85% caution zone) drops
    // into the caution tier, ahead of cline-pass there by expiry.
    expect(record.shadowOrdering).toEqual(["_openai.main", "deepseek", "_claude", "cline-pass", "opendesign"]);
    const claude = record.pools.find((p) => p.providerId === "_claude")!;
    expect(claude.liveTier).toBe("open");
    expect(claude.shadowTier).toBe("caution");
    expect(claude.demotedByForecast).toBe(true);
    expect(claude.moved).toBe("demoted");
    expect(claude.reasons.join(" ")).toContain("weekly");
    expect(record.summary.demotedByForecast).toBe(1);
    expect(record.summary.identical).toBe(false);
    // deepseek only moved relatively.
    const deepseek = record.pools.find((p) => p.providerId === "deepseek")!;
    expect(deepseek.moved).toBe("promoted");
    expect(deepseek.demotedByForecast).toBe(false);
  });

  test("the forecast never promotes a measured demotion and never closes a pool", () => {
    const current = fivePools();
    const fc = forecast([
      // cline-pass is measured-caution at 90%; a benign forecast
      // must not lift it back into the open tier.
      forecastPool("cline-pass", "cline-pass", [forecastWindow({ state: "no-wall-before-reset", pctNow: 90 })]),
      // opendesign is measured-walled; the shadow ordering is a
      // permutation of the live one — same pools, nobody removed.
      forecastPool("opendesign", "opendesign", [forecastWindow({ state: "no-wall-before-reset", pctNow: 100 })]),
    ]);
    const record = buildShadowPlacement(current, fc, NOW);
    const cline = record.pools.find((p) => p.providerId === "cline-pass")!;
    expect(cline.shadowTier).toBe("caution");
    expect([...record.shadowOrdering].sort()).toEqual([...record.liveOrdering].sort());
    expect(record.shadowOrdering[record.shadowOrdering.length - 1]).toBe("opendesign");
  });
});

describe("no live effect, whatever the forecast says or fails to say", () => {
  const variants: [string, QuotaForecastFile | null][] = [
    ["null forecast", null],
    ["empty forecast", forecast([])],
    [
      "every pool projected to wall",
      forecast([
        forecastPool("_claude", "anthropic:claude-pro", [
          forecastWindow({ state: "projected", wallsBeforeReset: true, projectedWallAt: NOW_MS + HOUR }),
        ]),
        forecastPool("_openai.main", "_openai.main", [
          forecastWindow({ state: "projected", wallsBeforeReset: true, projectedWallAt: NOW_MS + HOUR }),
        ]),
        forecastPool("cline-pass", "cline-pass", [
          forecastWindow({ state: "projected", wallsBeforeReset: true, projectedWallAt: NOW_MS + HOUR }),
        ]),
      ]),
    ],
    [
      "honesty rows only",
      forecast([
        forecastPool("_claude", "anthropic:claude-pro", [
          forecastWindow({
            state: "insufficient-history",
            ratePtsPerHour: null,
            samples: 0,
            wallsBeforeReset: null,
            pctNow: null,
            reason: "fewer than two same-period readings spanning 10 minutes",
          }),
        ]),
      ]),
    ],
    [
      "stale forecast readings",
      forecast([
        forecastPool("_claude", "anthropic:claude-pro", [
          forecastWindow({
            state: "projected",
            wallsBeforeReset: true,
            projectedWallAt: NOW_MS + HOUR,
            measuredAt: STALE,
          }),
        ]),
      ]),
    ],
  ];
  for (const [name, fc] of variants) {
    test(`live ordering identical with ${name}`, () => {
      const current = fivePools();
      const record = buildShadowPlacement(current, fc, NOW);
      expect(record.liveOrdering).toEqual(liveOrderingFor(current, NOW));
    });
  }

  test("a stale forecast reading degrades to no forecast: no demotion, exclusion logged", () => {
    const record = buildShadowPlacement(fivePools(), variants[4][1], NOW);
    const claude = record.pools.find((p) => p.providerId === "_claude")!;
    expect(claude.demotedByForecast).toBe(false);
    expect(claude.shadowTier).toBe("open");
    const input = claude.forecastInputs[0];
    expect(input.consumed).toBe(false);
    expect(input.exclusion).toContain("stale");
  });

  test("an insufficient-history forecast is logged with its reason, never consumed", () => {
    const record = buildShadowPlacement(fivePools(), variants[3][1], NOW);
    const claude = record.pools.find((p) => p.providerId === "_claude")!;
    expect(claude.demotedByForecast).toBe(false);
    const input = claude.forecastInputs[0];
    expect(input.consumed).toBe(false);
    expect(input.state).toBe("insufficient-history");
    expect(input.reason).toContain("fewer than two same-period readings");
    expect(record.shadowOrdering).toEqual(record.liveOrdering);
  });
});

describe("S2 findings bind consumption", () => {
  function goPools(): QuotaCurrentFile {
    return file([
      entry({
        providerId: "opencode-go",
        providerName: "OpenCode Go",
        serviceId: "opencode-go",
        windows: [window(20, { name: "monthly", resetsAt: iso(NOW_MS + 20 * DAY) })],
      }),
      entry({
        providerId: "opencode-go.drewrypope",
        providerName: "OpenCode Go",
        serviceId: "opencode-go.drewrypope",
        windows: [window(20, { name: "monthly", resetsAt: iso(NOW_MS + 20 * DAY) })],
      }),
    ]);
  }
  const monthlyWall = (pct: number) =>
    forecastWindow({
      name: "monthly",
      state: "projected",
      wallsBeforeReset: true,
      projectedWallAt: NOW_MS + 5 * DAY,
      durationUntilWallMs: 5 * DAY,
      pctNow: pct,
      resetsAt: iso(NOW_MS + 20 * DAY),
    });

  test("the bare opencode-go pool's monthly forecast is NOT consumed (cross-workspace mixing artifact)", () => {
    const fc = forecast([
      forecastPool("opencode-go", "opencode-go", [monthlyWall(20)]),
      forecastPool("opencode-go.drewrypope", "opencode-go.drewrypope", [monthlyWall(20)]),
    ]);
    const record = buildShadowPlacement(goPools(), fc, NOW);
    const bare = record.pools.find((p) => p.providerId === "opencode-go")!;
    expect(bare.demotedByForecast).toBe(false);
    expect(bare.shadowTier).toBe("open");
    expect(bare.forecastInputs[0].consumed).toBe(false);
    expect(bare.forecastInputs[0].exclusion).toContain("opencode-go");
    // The per-workspace row is the real one: consumed, demoted.
    const ws = record.pools.find((p) => p.providerId === "opencode-go.drewrypope")!;
    expect(ws.demotedByForecast).toBe(true);
    expect(ws.shadowTier).toBe("caution");
    expect(ws.forecastInputs[0].consumed).toBe(true);
  });
});

describe("the diff record S7 scores", () => {
  test("carries both orderings, the decision point, per-pool forecast inputs with honesty states, and ranks", () => {
    const fc = forecast([
      forecastPool("_claude", "anthropic:claude-pro", [
        forecastWindow({ state: "projected", wallsBeforeReset: true, projectedWallAt: NOW_MS + 5 * HOUR }),
      ]),
    ]);
    const record = buildShadowPlacement(fivePools(), fc, NOW);
    expect(record.version).toBe(1);
    expect(record.kind).toBe("quota-shadow-placement");
    expect(record.generatedAt).toBe(NOW.toISOString());
    expect(record.decisionPoint.kind).toBe("gatherer-tick");
    expect(record.decisionPoint.at).toBe(NOW.toISOString());
    expect(record.decisionPoint.forecastGeneratedAt).toBe(FRESH);
    expect(record.decisionPoint.currentGeneratedAt).toBe(FRESH);
    const claude = record.pools.find((p) => p.providerId === "_claude")!;
    expect(claude.liveRank).toBe(2);
    expect(claude.shadowRank).toBe(3);
    expect(claude.measured.percent).toBe(40);
    expect(claude.measured.bindingWindow).toBe("weekly");
    expect(claude.forecastInputs).toHaveLength(1);
    expect(claude.forecastInputs[0].state).toBe("projected");
    expect(claude.forecastInputs[0].wallsBeforeReset).toBe(true);
    expect(claude.forecastInputs[0].projectedWallAt).toBe(NOW_MS + 5 * HOUR);
    expect(record.summary.pools).toBe(5);
    expect(record.summary.moved).toBe(2);
  });

  test("degenerate inputs produce an empty record, never a throw", () => {
    const empty = buildShadowPlacement(file([]), null, NOW);
    expect(empty.liveOrdering).toEqual([]);
    expect(empty.shadowOrdering).toEqual([]);
    expect(empty.summary.pools).toBe(0);
    const noWindows = buildShadowPlacement(file([entry({ windows: [] })]), forecast([]), NOW);
    expect(noWindows.pools[0].liveTier).toBe("unknown");
    // A forecast for a pool the current read does not carry is not
    // placed: placement orders pools that exist in the measured read.
    const ghost = buildShadowPlacement(
      fivePools(),
      forecast([forecastPool("mistral", "mistral", [forecastWindow({ state: "projected", wallsBeforeReset: true })])]),
      NOW,
    );
    expect(ghost.liveOrdering).not.toContain("mistral");
    expect(ghost.shadowOrdering).not.toContain("mistral");
  });
});
