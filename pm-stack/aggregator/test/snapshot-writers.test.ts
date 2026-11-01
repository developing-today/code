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
  recordsFromDeepseekBalance,
  recordsFromDns,
  recordsFromPaygLines,
  recordsFromPools,
  recordsFromProviderUsage,
  recordsFromSaasServices,
  freshestDate,
} from "../src/snapshot-records.ts";
import { SnapshotStore, isSnapshotRecord } from "../src/snapshots.ts";

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
      { id: "meta-payg", label: "Meta", accruedUsd: 23.92, unbilledUsd: 0.01, note: "", measuredAt: "2026-10-25", sourceClass: "console" },
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
      windows: [{ name: "weekly", unit: "other", limit: 100, used: 42, percent: 42, resetsAt: "2026-10-28T14:00:00-05:00" }],
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
    const walled = { generated_at: "2026-10-26T10:59:33Z", sessions: [1, 2, 3], codex_usage: { status: "http 403", usage: null } };
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

describe("DeepSeek CNY balance (#1052)", () => {
  // The quota-view DeepSeek section as the backfill reads it: the date
  // rides on the balance line itself.
  const QUOTA_VIEW = `# Quota view

### DeepSeek
- **Plan / price:** API PAYG reserve, list rates; never a routine route.
- **PAYG balance:** ¥44.63 (2026-10-25 read), parked.

### GitHub
- **Last checked:** 2026-10-26
`;

  test("the dated ¥ read becomes a cny record at its own date, never usd", () => {
    const recs = recordsFromDeepseekBalance(QUOTA_VIEW, "backfill:quota-view.md");
    expect(recs).toHaveLength(1);
    expect(recs[0]).toMatchObject({
      ts: dayTs("2026-10-25"),
      serviceId: "deepseek:api",
      metricId: "credits_balance",
      value: 44.63,
      unit: "cny",
      source: "api",
      observedBy: "backfill:quota-view.md",
    });
    expect(recs.every((r) => r.unit !== "usd")).toBe(true);
  });

  test("no ¥ line, no section, or no date yields no record (nothing invented)", () => {
    expect(recordsFromDeepseekBalance("# Nothing\n", "b")).toEqual([]);
    expect(recordsFromDeepseekBalance("### DeepSeek\n- balance: $6.00\n", "b")).toEqual([]);
    expect(recordsFromDeepseekBalance("### DeepSeek\n- balance: ¥44.63\n", "b")).toEqual([]);
  });

  test("a cny record validates, round-trips the store, and no DeepSeek record is usd", async () => {
    const dir = await mkdtemp(join(tmpdir(), "snap-cny-"));
    try {
      const store = new SnapshotStore(dir);
      const recs = recordsFromDeepseekBalance(QUOTA_VIEW, "backfill:quota-view.md");
      expect(isSnapshotRecord(recs[0])).toBe(true);
      expect(await store.append(recs)).toBe(1);
      expect(await store.append(recs)).toBe(0); // re-run dedupes
      const stored = (await store.readAll()).filter((r) => r.serviceId === "deepseek:api");
      expect(stored).toHaveLength(1);
      expect(stored[0].unit).toBe("cny");
      expect(stored[0].value).toBe(44.63);
      expect(stored.filter((r) => r.unit === "usd")).toEqual([]);
    } finally {
      await rm(dir, { recursive: true, force: true });
    }
  });
});

describe("unreadable records are rejected (#1540)", () => {
  const base = {
    serviceId: "cloudflare",
    metricId: "gateway-accrual-est",
    unit: "usd-est",
    source: "estimate",
    observedBy: "stewardship-cycle",
    ts: "2026-10-26T12:00:00Z",
  } as const;

  test("non-finite and inverted pairs never validate", () => {
    expect(isSnapshotRecord({ ...base, value: { low: NaN, high: 10 } })).toBe(false);
    expect(isSnapshotRecord({ ...base, value: { low: Number.POSITIVE_INFINITY, high: 10 } })).toBe(false);
    expect(isSnapshotRecord({ ...base, value: { low: 1, high: Number.NEGATIVE_INFINITY } })).toBe(false);
    expect(isSnapshotRecord({ ...base, value: { low: 50, high: 10 } })).toBe(false);
  });

  test("a ts outside the usage-YYYY-MM routing never validates", () => {
    // Date.parse accepts it, but snapshotFileFor routes it to
    // usage-October.jsonl, which readAll never reads.
    expect(isSnapshotRecord({ ...base, ts: "October 20, 2026", value: { low: 1, high: 2 } })).toBe(false);
  });

  test("unreadable records append nothing and write no files", async () => {
    const dir = await mkdtemp(join(tmpdir(), "snap-unreadable-"));
    try {
      const store = new SnapshotStore(dir);
      const bad = [
        { ...base, value: { low: NaN, high: 10 } },
        { ...base, value: { low: Number.POSITIVE_INFINITY, high: 10 } },
        { ...base, value: { low: 50, high: 10 } },
        { ...base, ts: "October 20, 2026", value: { low: 1, high: 2 } },
      ];
      expect(await store.append(bad as never)).toBe(0);
      expect(await store.readAll()).toEqual([]);
    } finally {
      await rm(dir, { recursive: true, force: true });
    }
  });

  test("the finite-pair ISO control still appends once and dedupes", async () => {
    const dir = await mkdtemp(join(tmpdir(), "snap-pair-control-"));
    try {
      const store = new SnapshotStore(dir);
      const rec = { ...base, value: { low: 29.6, high: 45.1 } };
      expect(isSnapshotRecord(rec)).toBe(true);
      expect(await store.append([rec] as never)).toBe(1);
      expect(await store.append([rec] as never)).toBe(0);
      expect(await store.readAll()).toHaveLength(1);
    } finally {
      await rm(dir, { recursive: true, force: true });
    }
  });
});

describe("writer dedupe invariant", () => {
  test("re-appending a derived batch appends nothing the second time", async () => {
    const dir = await mkdtemp(join(tmpdir(), "snap-writers-"));
    try {
      const store = new SnapshotStore(dir);
      const batch = [
        ...recordsFromSaasServices(
          [svc({ metrics: [{ id: "databases", label: "Databases", unit: "count", used: 1, limit: 100, highlight: true }] })],
          "fleet-feed-collector",
        ),
        // The same fact derived by another writer (different
        // observedBy) is the same record to the store.
        ...recordsFromSaasServices(
          [svc({ metrics: [{ id: "databases", label: "Databases", unit: "count", used: 1, limit: 100, highlight: true }] })],
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

describe("snapshot boundary on validate.ts primitives (#2578 Phase 2)", () => {
  const base = {
    serviceId: "cloudflare",
    metricId: "gateway-accrual-est",
    unit: "usd-est",
    source: "estimate",
    observedBy: "stewardship-cycle",
    ts: "2026-10-26T12:00:00Z",
    value: 5,
  } as const;

  test("limit/resetsAt are validated when present (#2483)", () => {
    expect(isSnapshotRecord({ ...base, limit: 100 })).toBe(true);
    expect(isSnapshotRecord({ ...base, limit: 0 })).toBe(true);
    expect(isSnapshotRecord({ ...base, limit: null })).toBe(true);
    expect(isSnapshotRecord({ ...base, limit: Number.NaN })).toBe(false);
    expect(isSnapshotRecord({ ...base, limit: Number.POSITIVE_INFINITY })).toBe(false);
    expect(isSnapshotRecord({ ...base, limit: -1 })).toBe(false);
    expect(isSnapshotRecord({ ...base, resetsAt: "2026-11-01T00:00:00Z" })).toBe(true);
    expect(isSnapshotRecord({ ...base, resetsAt: null })).toBe(true);
    expect(isSnapshotRecord({ ...base, resetsAt: "2026-02-30" })).toBe(false);
    expect(isSnapshotRecord({ ...base, resetsAt: "2026-13-01T00:00:00Z" })).toBe(false);
    expect(isSnapshotRecord({ ...base, resetsAt: "soon" })).toBe(false);
  });

  test("impossible ts dates never validate (#2264)", () => {
    expect(isSnapshotRecord({ ...base, ts: "2026-02-30T12:00:00Z" })).toBe(false);
    expect(isSnapshotRecord({ ...base, ts: "2026-13-01T12:00:00Z" })).toBe(false);
  });

  test("freshestDate ignores impossible dates (#2264)", () => {
    expect(freshestDate("checked 2026-10-01; renews 2026-02-30")).toBe("2026-10-01");
    expect(freshestDate("only 2026-13-01 here")).toBe(null);
    expect(freshestDate("2026-10-01 and 2026-10-03")).toBe("2026-10-03");
    expect(freshestDate("no dates")).toBe(null);
  });
});
