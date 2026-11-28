import { describe, expect, test } from "bun:test";
import {
  buildForecastLedgerRows,
  buildQuotaForecast,
  forecastSeriesKey,
  FORECAST_LEDGER_SOURCE,
} from "./quota-forecast.ts";
import { projectWindow, type PctPoint } from "./quota-analytics.ts";
import type { QuotaCurrentEntry, QuotaCurrentFile, QuotaCurrentWindow, SnapshotRecord } from "./model.ts";

// Oracle: the Stage-3 design D1/D2 (doc/2026-11-22T22-35-00Z_design_
// quota-plane-stage-3, #12695): the forecast product IS
// quota-analytics.ts's projectWindow promoted per tick — same inputs
// -> same projection — keyed by the canonical provider id
// (pool-state.ts keying), freshness by quota-current.ts's overlay
// entryState rule (fresh ok only; retained is stale by definition),
// series joined across the recorded serviceId renames (design §2)
// with the analytics-store join semantics.

const NOW = new Date("2026-11-22T22:00:00.000Z");
const NOW_MS = NOW.getTime();
const FRESH = "2026-11-22T21:55:00.000Z"; // 5 min before NOW
const STALE = "2026-11-22T19:00:00.000Z"; // 3 h before NOW
const RESET = "2026-11-23T08:00:00.000Z"; // 10 h after NOW
const HOUR = 3_600_000;

function window(pctOfLimit: number | null, over: Partial<QuotaCurrentWindow> = {}): QuotaCurrentWindow {
  return {
    name: "weekly",
    value: pctOfLimit,
    pctOfLimit,
    valueLabel: null,
    windowSeconds: null,
    resetsAt: RESET,
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
    windows: [window(70)],
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

function record(
  serviceId: string,
  metricId: string,
  hoursAgo: number,
  value: number,
  over: Partial<SnapshotRecord> = {},
): SnapshotRecord {
  return {
    ts: new Date(NOW_MS - hoursAgo * HOUR).toISOString(),
    serviceId,
    metricId,
    value,
    unit: "pct",
    limit: 100,
    resetsAt: RESET,
    source: "api",
    observedBy: "quota-gatherer",
    ...over,
  };
}

/** A clean 10 pts/h burn: 40/50/60 at 3/2/1 h ago, 70 now. */
const BURN: SnapshotRecord[] = [
  record("anthropic:claude-pro", "weekly", 3, 40),
  record("anthropic:claude-pro", "weekly", 2, 50),
  record("anthropic:claude-pro", "weekly", 1, 60),
];

function weeklyOf(out: ReturnType<typeof buildQuotaForecast>, providerId: string, serviceId: string) {
  const pool = out.pools[providerId]?.find((p) => p.serviceId === serviceId);
  return pool?.windows.find((w) => w.name === "weekly");
}

describe("buildQuotaForecast (#12695 forecast product, D1/D2)", () => {
  test("promotes the analytics projection verbatim (oracle: projectWindow, same inputs)", () => {
    const out = buildQuotaForecast(file([entry()]), BURN, NOW);
    const w = weeklyOf(out, "_claude", "anthropic:claude-pro");
    const points: PctPoint[] = BURN.map((r) => ({ ts: Date.parse(r.ts), pct: r.value as number, resetsAt: RESET }));
    const oracle = projectWindow({ points, pctNow: 70, resetsAt: RESET, now: NOW_MS });
    expect(oracle.state).toBe("projected"); // fixture sanity: the burn really does project
    expect(w).toMatchObject({
      state: oracle.state,
      ratePtsPerHour: oracle.ratePtsPerHour,
      samples: oracle.samples,
      projectedWallAt: oracle.projectedWallAt,
      durationUntilWallMs: oracle.durationUntilWallMs,
      wallsBeforeReset: oracle.wallsBeforeReset,
    });
    expect(w?.pctNow).toBe(70);
    expect(w?.measuredAt).toBe(FRESH);
    expect(w?.resetsAt).toBe(RESET);
    expect(out.version).toBe(1);
    expect(out.source).toBe("gather-quota");
    expect(out.generatedAt).toBe(FRESH);
  });

  test("property: forecast projection equals the projectWindow oracle on generated series", () => {
    // Seeded generator (no new dependency): for each case the
    // builder's window must equal projectWindow run on the same
    // series — the promotion adds keying/freshness, never new math.
    let s = 123456789;
    const rnd = (): number => {
      s = (s * 1664525 + 1013904223) >>> 0;
      return s / 2 ** 32;
    };
    for (let caseNo = 0; caseNo < 60; caseNo++) {
      const n = Math.floor(rnd() * 9); // 0..8 points
      const resetsAt = rnd() < 0.25 ? null : new Date(NOW_MS + Math.floor(rnd() * 48) * HOUR).toISOString();
      const records: SnapshotRecord[] = [];
      const points: PctPoint[] = [];
      for (let i = 0; i < n; i++) {
        const ts = NOW_MS - Math.floor(rnd() * 6 * HOUR) - i; // distinct, <= now, inside the fit window
        const pct = Math.floor(rnd() * 100);
        records.push({
          ts: new Date(ts).toISOString(),
          serviceId: "anthropic:claude-pro",
          metricId: "weekly",
          value: pct,
          unit: "pct",
          limit: 100,
          resetsAt,
          source: "api",
          observedBy: "quota-gatherer",
        });
        points.push({ ts, pct, resetsAt });
      }
      const pctNow = Math.floor(rnd() * 101);
      const out = buildQuotaForecast(file([entry({ windows: [window(pctNow, { resetsAt })] })]), records, NOW);
      const w = weeklyOf(out, "_claude", "anthropic:claude-pro");
      const oracle = projectWindow({ points, pctNow, resetsAt, now: NOW_MS });
      expect(w?.state).toBe(oracle.state);
      expect(w?.ratePtsPerHour).toBe(oracle.ratePtsPerHour);
      expect(w?.samples).toBe(oracle.samples);
      expect(w?.projectedWallAt).toBe(oracle.projectedWallAt);
      expect(w?.wallsBeforeReset).toBe(oracle.wallsBeforeReset);
    }
  });

  test("keys pools by the canonical provider id; service ids ride on the pools", () => {
    const gemini = entry({
      providerId: "_antigravity",
      providerName: "Antigravity",
      serviceId: "_antigravity.gemini",
      windows: [window(30, { name: "5h" })],
    });
    const out = buildQuotaForecast(file([gemini]), [], NOW);
    expect(Object.keys(out.pools)).toEqual(["_antigravity"]);
    expect(out.pools["_antigravity"][0].serviceId).toBe("_antigravity.gemini");
    expect(out.pools["_antigravity"][0].windows[0].name).toBe("5h");
  });

  test("joins the series across the recorded serviceId renames (design §2)", () => {
    // Antigravity: colon-era + pre-underscore spellings are the same
    // series as the current split id.
    const gemini = entry({
      providerId: "_antigravity",
      providerName: "Antigravity",
      serviceId: "_antigravity.gemini",
      windows: [window(70, { name: "5h" })],
    });
    const history = [
      record("antigravity:gemini", "5h", 3, 40),
      record("antigravity.gemini", "5h", 2, 50),
      record("_antigravity.gemini", "5h", 1, 60),
    ];
    const out = buildQuotaForecast(file([gemini]), history, NOW);
    const w = out.pools["_antigravity"][0].windows[0];
    expect(w.samples).toBe(3);
    expect(w.state).toBe("projected");

    // Command Code: command-code / commandcode / _commandcode.
    const cc = entry({
      providerId: "_commandcode",
      providerName: "Command Code",
      serviceId: "_commandcode",
      windows: [window(70)],
    });
    const ccHistory = [
      record("command-code", "weekly", 3, 40),
      record("commandcode", "weekly", 2, 50),
      record("_commandcode", "weekly", 1, 60),
    ];
    const ccOut = buildQuotaForecast(file([cc]), ccHistory, NOW);
    const ccw = weeklyOf(ccOut, "_commandcode", "_commandcode");
    expect(ccw?.samples).toBe(3);
    expect(ccw?.state).toBe("projected");
  });

  test("duplicate store lines count once (analytics-store dedupe rule)", () => {
    const dup = [...BURN, BURN[0], BURN[0]];
    const out = buildQuotaForecast(file([entry()]), dup, NOW);
    const w = weeklyOf(out, "_claude", "anthropic:claude-pro");
    expect(w?.samples).toBe(3);
  });

  test("a stale reading at 100% forecasts nothing: insufficient-history, never walled", () => {
    const stale = entry({ measuredAt: STALE, windows: [window(100, { measuredAt: STALE })] });
    const out = buildQuotaForecast(file([stale]), BURN, NOW);
    const w = weeklyOf(out, "_claude", "anthropic:claude-pro");
    expect(w?.state).toBe("insufficient-history");
    expect(w?.projectedWallAt).toBeNull();
    expect(w?.ratePtsPerHour).toBeNull();
    expect(w?.pctNow).toBeNull();
    expect(w?.measuredAt).toBe(STALE); // provenance still says when it was read
    expect(w?.reason).toContain("stale");
  });

  test("a retained last-known reading is stale by definition (quota-current overlay rule)", () => {
    const retained = entry({ retainedFromFailedTick: FRESH, windows: [window(100)] });
    const out = buildQuotaForecast(file([retained]), BURN, NOW);
    const w = weeklyOf(out, "_claude", "anthropic:claude-pro");
    expect(w?.state).toBe("insufficient-history");
    expect(w?.reason).toContain("retained");
  });

  test("an errored provider yields an explicit insufficient-history row naming the state", () => {
    const errored = entry({
      state: "error",
      error: "token expired",
      windows: [window(55, { state: "error", measuredAt: STALE })],
    });
    const out = buildQuotaForecast(file([errored]), BURN, NOW);
    const w = weeklyOf(out, "_claude", "anthropic:claude-pro");
    expect(w?.state).toBe("insufficient-history");
    expect(w?.reason).toContain("error");
    expect(w?.projectedWallAt).toBeNull();
  });

  test("thin history is the oracle's own insufficient-history, with its reason", () => {
    const out = buildQuotaForecast(file([entry()]), [record("anthropic:claude-pro", "weekly", 0.05, 65)], NOW);
    const w = weeklyOf(out, "_claude", "anthropic:claude-pro");
    expect(w?.state).toBe("insufficient-history");
    expect(w?.samples).toBe(1);
    expect(w?.reason).toContain("fewer than two same-period readings");
  });

  test("a fresh reading at 100% is walled, verbatim from the oracle", () => {
    const out = buildQuotaForecast(file([entry({ windows: [window(100)] })]), BURN, NOW);
    const w = weeklyOf(out, "_claude", "anthropic:claude-pro");
    expect(w?.state).toBe("walled");
    expect(w?.durationUntilWallMs).toBe(0);
    expect(w?.wallsBeforeReset).toBe(true);
  });

  test("label-only windows (no percentage anywhere) get no forecast row", () => {
    const balance = entry({
      providerId: "deepseek",
      providerName: "DeepSeek",
      serviceId: "deepseek",
      windows: [window(null, { name: "credits_balance", valueLabel: "¥44.29" })],
    });
    const out = buildQuotaForecast(file([balance]), [], NOW);
    expect(out.pools["deepseek"]?.[0]?.windows ?? []).toEqual([]);
  });

  test("deterministic: input order does not change the product", () => {
    const a = buildQuotaForecast(file([entry()]), BURN, NOW);
    const b = buildQuotaForecast(file([entry()]), [...BURN].reverse(), NOW);
    expect(JSON.stringify(b)).toBe(JSON.stringify(a));
  });
});

describe("forecastSeriesKey (the explicit rename mapping, design §2)", () => {
  test("folds the recorded spellings of one pool onto one key", () => {
    expect(forecastSeriesKey("_antigravity")).toBe(forecastSeriesKey("antigravity"));
    expect(forecastSeriesKey("command-code")).toBe(forecastSeriesKey("commandcode"));
    expect(forecastSeriesKey("_commandcode")).toBe(forecastSeriesKey("commandcode"));
    expect(forecastSeriesKey("_openai.main")).toBe(forecastSeriesKey("openai.main"));
    expect(forecastSeriesKey("antigravity:gemini")).toBe(forecastSeriesKey("_antigravity.gemini"));
    // Distinct pools stay distinct.
    expect(forecastSeriesKey("_antigravity.gemini")).not.toBe(forecastSeriesKey("_antigravity.claude-gpt"));
    expect(forecastSeriesKey("meta")).not.toBe(forecastSeriesKey("cline-pass"));
  });
});

describe("buildForecastLedgerRows (#12695 ledger half of D1)", () => {
  test("one row per pool/window, forecast source, payload round-trips the forecast", () => {
    const out = buildQuotaForecast(file([entry()]), BURN, NOW);
    const rows = buildForecastLedgerRows(out, FRESH);
    expect(rows.length).toBe(1);
    const row = rows[0];
    expect(row.source).toBe(FORECAST_LEDGER_SOURCE);
    expect(row.id).toBe(`anthropic:claude-pro:weekly:${FRESH}`);
    expect(row.ts).toBe(FRESH);
    expect(row.weeklyPercent).toBe(70); // the weekly window's own current percent
    expect(row.resets).toBe(RESET);
    const payload = row.payload as Record<string, unknown>;
    expect(payload.kind).toBe("quota-forecast");
    expect(payload.state).toBe("projected");
    expect(payload.providerId).toBe("_claude");
    expect(payload.serviceId).toBe("anthropic:claude-pro");
    expect(payload.window).toBe("weekly");
    expect(payload.pctNow).toBe(70);
    expect(payload.measuredAt).toBe(FRESH);
  });

  test("non-weekly windows carry no weekly_percent; ids are unique per window", () => {
    const two = entry({
      windows: [window(70), window(30, { name: "5h" })],
    });
    const out = buildQuotaForecast(file([two]), BURN, NOW);
    const rows = buildForecastLedgerRows(out, FRESH);
    expect(rows.length).toBe(2);
    expect(new Set(rows.map((r) => r.id)).size).toBe(2);
    expect(rows.find((r) => r.id.includes(":5h:"))?.weeklyPercent).toBeNull();
  });
});
