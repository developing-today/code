import { describe, expect, test } from "bun:test";
import type { QuotaView } from "../src/model.ts";
import {
  STALE_AFTER_MS,
  createQuotaRefresher,
  isQuotaStale,
  newestMeasurement,
  type RefreshTrigger,
} from "../src/quota-refresh.ts";

const NOW = Date.parse("2026-11-03T14:00:00Z");
const iso = (offsetMs: number) => new Date(NOW - offsetMs).toISOString();

function view(measuredAt: string | null, extra: { tickAt?: string | null } = {}): QuotaView {
  return {
    pools: [
      {
        id: "p1",
        provider: "test",
        plan: "test",
        state: "ok" as const,
        windows: [
          {
            name: "week",
            unit: "tokens" as const,
            used: 10,
            limit: 100,
            resetsAt: iso(-86_400_000),
          },
        ],
        ...(measuredAt ? { freshness: { measuredAt, source: "quota-current.json", state: "ok" as const } } : {}),
      },
    ],
    ...(extra.tickAt === undefined
      ? {}
      : {
          gatherer: {
            generatedAt: extra.tickAt ?? "",
            source: "quota-current.json",
            tickAt: extra.tickAt ?? null,
            ok: true,
            providers: [],
          },
        }),
  } as unknown as QuotaView;
}

function harness(nowMs = NOW) {
  const calls: Array<"stale" | "manual"> = [];
  let release: (() => void) | null = null;
  const trigger: RefreshTrigger = (reason) => {
    calls.push(reason);
    return new Promise<void>((resolve) => {
      release = resolve;
    });
  };
  let clock = nowMs;
  const refresher = createQuotaRefresher({
    trigger,
    now: () => clock,
    cooldownMs: 60_000,
  });
  return {
    calls,
    refresher,
    advance: (ms: number) => {
      clock += ms;
    },
    finish: async () => {
      release?.();
      release = null;
      // Let the promise chain clear the in-flight marker.
      await Promise.resolve();
      await Promise.resolve();
    },
  };
}

describe("quota freshness (#1171)", () => {
  test("newest measurement wins across windows and gatherer tick", () => {
    const v = view(iso(10 * 60_000), { tickAt: iso(60_000) });
    expect(newestMeasurement(v, NOW)).toBe(NOW - 60_000);
  });

  test("missing measurements count as stale", () => {
    expect(isQuotaStale(view(null), NOW)).toBe(true);
  });

  test("older than five minutes is stale, newer is fresh", () => {
    expect(isQuotaStale(view(iso(STALE_AFTER_MS + 1_000)), NOW)).toBe(true);
    expect(isQuotaStale(view(iso(STALE_AFTER_MS - 1_000)), NOW)).toBe(false);
  });

  test("future timestamps are treated as fresh, not stale", () => {
    expect(isQuotaStale(view(iso(-60 * 60_000)), NOW)).toBe(false);
  });
});

describe("lazy refresh trigger (#1171)", () => {
  test("fresh data does not trigger", () => {
    const h = harness();
    const status = h.refresher.request(view(iso(30_000)));
    expect(status).toMatchObject({ stale: false, refreshing: false, triggered: false });
    expect(h.calls).toEqual([]);
  });

  test("stale load triggers exactly one gatherer run", () => {
    const h = harness();
    const status = h.refresher.request(view(iso(10 * 60_000)));
    expect(status).toMatchObject({ stale: true, refreshing: true, triggered: true });
    expect(h.calls).toEqual(["stale"]);
  });

  test("concurrent stale loads dedupe to the in-flight run", () => {
    const h = harness();
    h.refresher.request(view(iso(10 * 60_000)));
    const second = h.refresher.request(view(iso(10 * 60_000)));
    expect(second).toMatchObject({ stale: true, refreshing: true, triggered: false });
    expect(h.calls).toEqual(["stale"]);
  });

  test("cooldown suppresses a re-trigger after the run finishes", async () => {
    const h = harness();
    h.refresher.request(view(iso(10 * 60_000)));
    await h.finish();
    const again = h.refresher.request(view(iso(10 * 60_000)));
    expect(again).toMatchObject({ refreshing: true, triggered: false });
    expect(h.calls).toEqual(["stale"]);
  });

  test("a new stale load triggers again once the cooldown elapses", async () => {
    const h = harness();
    h.refresher.request(view(iso(10 * 60_000)));
    await h.finish();
    h.advance(61_000);
    const again = h.refresher.request(view(iso(10 * 60_000)));
    expect(again.triggered).toBe(true);
    expect(h.calls).toEqual(["stale", "stale"]);
  });

  test("manual refresh uses the same path and bypasses freshness", () => {
    const h = harness();
    const status = h.refresher.manual(view(iso(1_000)));
    expect(status).toMatchObject({ stale: false, refreshing: true, triggered: true });
    expect(h.calls).toEqual(["manual"]);
  });

  test("manual refresh is single-flight too", () => {
    const h = harness();
    h.refresher.manual(view(iso(1_000)));
    const second = h.refresher.manual(view(iso(1_000)));
    expect(second.triggered).toBe(false);
    expect(h.calls).toEqual(["manual"]);
  });

  test("a failing trigger fails soft and clears in-flight", async () => {
    const calls: string[] = [];
    const refresher = createQuotaRefresher({
      trigger: () => {
        calls.push("boom");
        throw new Error("no systemd");
      },
      now: () => NOW,
    });
    const originalError = console.error;
    console.error = () => {};
    try {
      refresher.request(view(iso(10 * 60_000)));
      await Promise.resolve();
      await Promise.resolve();
      await Promise.resolve();
      expect(calls).toEqual(["boom"]);
    } finally {
      console.error = originalError;
    }
  });
});
