// Usage snapshot writers (hub design §4): record derivation from the
// hub's extracted structures, the provider-usage mirror, and the
// dedupe invariant that makes every writer safe to re-run.
import { describe, expect, test } from "bun:test";
import { mkdtemp, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import type { PaygLine, QuotaPool, SaasService } from "../src/model.ts";
import {
  dayTs,
  recordsFromDns,
  recordsFromPaygLines,
  recordsFromPools,
  recordsFromProviderUsage,
  recordsFromSaasServices,
} from "../src/snapshot-records.ts";
import { SnapshotStore } from "../src/snapshots.ts";

const svc = (partial: Partial<SaasService>): SaasService => ({
  id: "turso",
  name: "Turso",
  group: "database",
  plan: "Starter Free",
  status: "active",
  metrics: [],
  paygEstimate: null,
  measuredAt: "2026-10-20",
  sourceClass: "manual",
  sourceNote: "test",
  cadenceDays: 7,
  conflicts: [],
  reads: [],
  usageNote: "",
  ...partial,
});

describe("recordsFromSaasServices", () => {
  test("measured metrics become records at the read's own date", () => {
    const recs = recordsFromSaasServices(
      [
        svc({
          metrics: [
            { id: "databases", label: "Databases", unit: "count", used: 1, limit: 100, highlight: true },
            { id: "rows-read", label: "Rows read", unit: "rows", used: 42, limit: null, highlight: true },
            { id: "storage", label: "Storage", unit: "gb", used: null, limit: 5, highlight: true },
          ],
        }),
      ],
      "fleet-feed-collector",
    );
    // rows has no snapshot-unit mapping and storage was not measured:
    // only the count metric is recorded.
    expect(recs).toEqual([
      {
        ts: dayTs("2026-10-20"),
        serviceId: "turso",
        metricId: "databases",
        value: 1,
        unit: "count",
        limit: 100,
        source: "manual",
        observedBy: "fleet-feed-collector",
      },
    ]);
  });

  test("a service PAYG estimate becomes a usd-est pair; no read date, no records", () => {
    const recs = recordsFromSaasServices(
      [svc({ id: "cloudflare", paygEstimate: { low: 0, high: 5.2, period: "per month" } })],
      "stewardship-cycle",
    );
    expect(recs).toEqual([
      {
        ts: dayTs("2026-10-20"),
        serviceId: "cloudflare",
        metricId: "platform-excess-est",
        value: { low: 0, high: 5.2 },
        unit: "usd-est",
        source: "estimate",
        observedBy: "stewardship-cycle",
      },
    ]);
    expect(recordsFromSaasServices([svc({ measuredAt: null })], "x")).toEqual([]);
  });
});

describe("recordsFromPaygLines", () => {
  test("backfill ids are reused so the series continues", () => {
    const lines: PaygLine[] = [
      {
        id: "meta-payg",
        label: "Meta",
        accruedUsd: 23.92,
        unbilledUsd: 0.01,
        note: "",
        measuredAt: "2026-10-25",
        sourceClass: "console",
      },
      {
        id: "cloudflare-gateway",
        label: "GW",
        accruedUsd: null,
        estimate: { low: 29.6, high: 45.1, period: "Oct 24–25 accrual" },
        unbilledUsd: 54.12,
        note: "",
        measuredAt: "2026-10-25",
        sourceClass: "estimate",
      },
      {
        id: "cloudflare-platform-excess",
        label: "CF excess",
        accruedUsd: null,
        estimate: { low: 0, high: 5.2, period: "per month" },
        unbilledUsd: 6.24,
        note: "",
        measuredAt: "2026-10-25",
        sourceClass: "estimate",
      },
    ];
    const recs = recordsFromPaygLines(lines, "fleet-feed-collector");
    expect(recs.map((r) => `${r.serviceId}/${r.metricId}`)).toEqual([
      "meta:high-usage/payg-accrued",
      "cloudflare/gateway-accrual-est",
    ]);
    expect(recs[0]).toMatchObject({ value: 23.92, unit: "usd", source: "console" });
    expect(recs[1]).toMatchObject({ value: { low: 29.6, high: 45.1 }, unit: "usd-est" });
  });
});

describe("recordsFromPools", () => {
  test("window percentages are stamped with the section's freshest date", () => {
    const pool: QuotaPool = {
      id: "anthropic:claude-pro",
      provider: "Anthropic",
      plan: "Claude Pro",
      state: "healthy",
      windows: [
        { name: "weekly", unit: "other", limit: 100, used: 42, percent: 42, resetsAt: "2026-10-28T14:00:00-05:00" },
      ],
    };
    // The 2027 renewal date elsewhere in the section is an operational
    // date, not a read — the Last checked bullet wins.
    const doc =
      "### Claude\n- read 2026-10-24 earlier\n- plan renews 2027-10-04\n- **Last checked:** 2026-10-26 refresh\n";
    const recs = recordsFromPools([pool], doc, "stewardship-cycle");
    expect(recs).toEqual([
      {
        ts: dayTs("2026-10-26"),
        serviceId: "anthropic:claude-pro",
        metricId: "weekly",
        value: 42,
        unit: "pct",
        limit: 100,
        resetsAt: "2026-10-28T14:00:00-05:00",
        source: "manual",
        observedBy: "stewardship-cycle",
      },
    ]);
  });
});

describe("recordsFromDns", () => {
  test("register totals ride the register mtime, stable across collections", () => {
    const dns = { generatedAt: "x", source: "test", totalAnnualListUsd: 606.7, entries: new Array(40) };
    const recs = recordsFromDns(dns as never, "2026-10-26T04:11:03.000Z", "fleet-feed-collector");
    expect(recs.map((r) => [r.metricId, r.value, r.ts])).toEqual([
      ["total-annual-usd", 606.7, "2026-10-26T04:11:03.000Z"],
      ["domain-count", 40, "2026-10-26T04:11:03.000Z"],
    ]);
  });
});

describe("recordsFromProviderUsage", () => {
  test("session count always; codex windows only when the endpoint answered", () => {
    const walled = {
      generated_at: "2026-10-26T10:59:33Z",
      sessions: [1, 2, 3],
      codex_usage: { status: "http 403", usage: null },
    };
    expect(recordsFromProviderUsage(walled, "check-usage")).toEqual([
      {
        ts: "2026-10-26T10:59:33Z",
        serviceId: "opencode-server",
        metricId: "sessions-listed",
        value: 3,
        unit: "count",
        source: "api",
        observedBy: "check-usage",
      },
    ]);
    const answered = {
      generated_at: "2026-10-26T10:59:33Z",
      sessions: [],
      codex_usage: {
        status: "ok",
        usage: {
          rate_limit: {
            primary_window: { used_percent: 12, limit_window_seconds: 604800 },
            secondary_window: { used_percent: 3, limit_window_seconds: 18000 },
          },
        },
      },
    };
    const recs = recordsFromProviderUsage(answered, "check-usage");
    expect(recs.map((r) => `${r.serviceId}/${r.metricId}=${r.value}`)).toEqual([
      "opencode-server/sessions-listed=0",
      "openai:codex/7-day=12",
      "openai:codex/5-hour=3",
    ]);
    expect(recordsFromProviderUsage(null, "check-usage")).toEqual([]);
    expect(recordsFromProviderUsage({ sessions: [] }, "check-usage")).toEqual([]);
  });
});

describe("writer dedupe invariant", () => {
  test("re-appending a derived batch appends nothing the second time", async () => {
    const dir = await mkdtemp(join(tmpdir(), "snap-writers-"));
    try {
      const store = new SnapshotStore(dir);
      const batch = [
        ...recordsFromSaasServices(
          [
            svc({
              metrics: [{ id: "databases", label: "Databases", unit: "count", used: 1, limit: 100, highlight: true }],
            }),
          ],
          "fleet-feed-collector",
        ),
        // The same fact derived by another writer (different
        // observedBy) is the same record to the store.
        ...recordsFromSaasServices(
          [
            svc({
              metrics: [{ id: "databases", label: "Databases", unit: "count", used: 1, limit: 100, highlight: true }],
            }),
          ],
          "stewardship-cycle",
        ),
      ];
      expect(await store.append(batch)).toBe(1);
      expect(await store.append(batch)).toBe(0);
      expect((await store.readAll()).length).toBe(1);
    } finally {
      await rm(dir, { recursive: true, force: true });
    }
  });
});
