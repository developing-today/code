// GitHub billing collector (#1051): aggregation of the unified
// billing usage endpoint's items into per-day totals and §4 snapshot
// records, plus the snapshot unit extension (minutes/hours/gb-hours)
// the collector's records depend on.
import { describe, expect, test } from "bun:test";
import { isSnapshotRecord } from "../src/snapshots.ts";
import {
  aggregateBillingUsage,
  recordsFromBillingUsage,
  summarizeBillingDays,
  type BillingUsageItem,
} from "../src/sources/github-billing.ts";

const ITEMS: BillingUsageItem[] = [
  // 2026-10-01: Linux + macOS Actions minutes, Actions storage, money.
  {
    date: "2026-10-01",
    product: "actions",
    sku: "Actions Linux",
    unitType: "Minutes",
    quantity: 100,
    grossAmount: 0.8,
    discountAmount: 0.8,
    netAmount: 0,
  },
  {
    date: "2026-10-01",
    product: "actions",
    sku: "Actions macOS 3-core",
    unitType: "Minutes",
    quantity: 9,
    grossAmount: 0.54,
    discountAmount: 0.54,
    netAmount: 0,
  },
  {
    date: "2026-10-01",
    product: "actions",
    sku: "Actions storage",
    unitType: "GigabyteHours",
    quantity: 19.4,
    grossAmount: 0.13,
    discountAmount: 0.13,
    netAmount: 0,
  },
  // 2026-10-02: Codespaces compute + storage; an unknown product row
  // counts toward gross/net only.
  {
    date: "2026-10-02",
    product: "codespaces",
    sku: "Codespaces 2-core",
    unitType: "Hours",
    quantity: 2.7,
    grossAmount: 0.49,
    discountAmount: 0.49,
    netAmount: 0,
  },
  {
    date: "2026-10-02",
    product: "codespaces",
    sku: "Codespaces storage",
    unitType: "GigabyteHours",
    quantity: 1.4,
    grossAmount: 0.01,
    discountAmount: 0.01,
    netAmount: 0,
  },
  {
    date: "2026-10-02",
    product: "packages",
    sku: "Packages storage",
    unitType: "GigabyteHours",
    quantity: 3,
    grossAmount: 0.02,
    discountAmount: 0,
    netAmount: 0.02,
  },
  // A row with no usable date is skipped, not crashed on.
  { product: "actions", unitType: "Minutes", quantity: 5 },
];

describe("aggregateBillingUsage", () => {
  test("groups by date with endpoint-native units", () => {
    const days = aggregateBillingUsage(ITEMS);
    expect(days.length).toBe(2);
    expect(days[0]).toEqual({
      date: "2026-10-01",
      actionsMinutes: 109,
      codespacesHours: 0,
      actionsStorageGbHours: 19.4,
      codespacesStorageGbHours: 0,
      grossUsd: 1.47,
      netUsd: 0,
    });
    expect(days[1].actionsMinutes).toBe(0);
    expect(days[1].codespacesHours).toBe(5.4);
    expect(days[1].codespacesStorageGbHours).toBe(1.4);
    expect(days[1].grossUsd).toBe(0.52);
    expect(days[1].netUsd).toBe(0.02);
  });

  test("month totals sum the days", () => {
    const totals = summarizeBillingDays(aggregateBillingUsage(ITEMS));
    expect(totals.actionsMinutes).toBe(109);
    expect(totals.codespacesHours).toBe(5.4);
    expect(totals.grossUsd).toBe(1.99);
    expect(totals.netUsd).toBe(0.02);
  });
});

describe("aggregateBillingUsage validation (#2493)", () => {
  test("impossible dates never create a day bucket", () => {
    const days = aggregateBillingUsage([
      { date: "2026-13-01", product: "actions", unitType: "Minutes", quantity: 10, grossAmount: 1, netAmount: 1 },
      { date: "2026-02-30", product: "actions", unitType: "Minutes", quantity: 10, grossAmount: 1, netAmount: 1 },
      { date: "2026-10-03", product: "actions", unitType: "Minutes", quantity: 7, grossAmount: 0.1, netAmount: 0.1 },
    ]);
    expect(days.length).toBe(1);
    expect(days[0].date).toBe("2026-10-03");
    expect(days[0].actionsMinutes).toBe(7);
  });

  test("negative or non-finite numbers contribute 0, never usage", () => {
    const days = aggregateBillingUsage([
      { date: "2026-10-04", product: "actions", unitType: "Minutes", quantity: -50, grossAmount: -2, netAmount: -2 },
      {
        date: "2026-10-04",
        product: "actions",
        unitType: "Minutes",
        quantity: Number.NaN,
        grossAmount: Number.POSITIVE_INFINITY,
        netAmount: 0.5,
      },
      { date: "2026-10-04", product: "actions", unitType: "Minutes", quantity: 30, grossAmount: 0.25, netAmount: 0 },
    ]);
    expect(days.length).toBe(1);
    expect(days[0].actionsMinutes).toBe(30);
    expect(days[0].grossUsd).toBe(0.25);
    expect(days[0].netUsd).toBe(0.5);
  });

  test("records derived from hostile items all pass isSnapshotRecord", () => {
    const records = recordsFromBillingUsage(
      [
        { date: "2026-13-40", product: "actions", unitType: "Minutes", quantity: 10, grossAmount: 1, netAmount: 1 },
        { date: "2026-10-05", product: "actions", unitType: "Minutes", quantity: -10, grossAmount: -1, netAmount: -1 },
        {
          date: "2026-10-05",
          product: "codespaces",
          sku: "Codespaces 4-core",
          unitType: "Hours",
          quantity: 1.5,
          grossAmount: 0.4,
          netAmount: 0.4,
        },
      ],
      "test",
    );
    expect(records.length).toBeGreaterThan(0);
    for (const r of records) expect(isSnapshotRecord(r)).toBe(true);
    expect(records.every((r) => r.value > 0)).toBe(true);
  });
});

describe("recordsFromBillingUsage", () => {
  const records = recordsFromBillingUsage(ITEMS, "test");

  test("emits dated github records with the new units, skipping zeros", () => {
    const day1 = records.filter((r) => r.ts === "2026-10-01T12:00:00Z");
    expect(day1.map((r) => `${r.metricId}:${r.unit}:${r.value}`).sort()).toEqual([
      "actions-minutes:minutes:109",
      "actions-storage-gb-hours:gb-hours:19.4",
      "billing-gross:usd:1.47",
    ]);
    // billing-net is 0 on day 1 (fully discounted) — no record.
    expect(day1.some((r) => r.metricId === "billing-net")).toBe(false);
    const cs = records.find((r) => r.metricId === "codespaces-hours");
    expect(cs).toMatchObject({
      ts: "2026-10-02T12:00:00Z",
      serviceId: "github",
      unit: "hours",
      value: 5.4,
      source: "api",
      observedBy: "test",
    });
  });

  test("every derived record passes the store's validation", () => {
    expect(records.length).toBeGreaterThan(0);
    for (const r of records) expect(isSnapshotRecord(r)).toBe(true);
  });
});

describe("snapshot unit extension", () => {
  test("minutes, hours and gb-hours records validate; unknown units do not", () => {
    const base = {
      ts: "2026-10-01T12:00:00Z",
      serviceId: "github",
      metricId: "m",
      value: 1,
      source: "api",
      observedBy: "test",
    };
    expect(isSnapshotRecord({ ...base, unit: "minutes" })).toBe(true);
    expect(isSnapshotRecord({ ...base, unit: "hours" })).toBe(true);
    expect(isSnapshotRecord({ ...base, unit: "gb-hours" })).toBe(true);
    expect(isSnapshotRecord({ ...base, unit: "parsecs" })).toBe(false);
  });
});

describe("codex regression cases (PR #2236 review)", () => {
  test("documented casing (Actions/minutes) classifies, not just the live casing", () => {
    // The exact spelling in GitHub's billing usage API example.
    const days = aggregateBillingUsage([
      {
        date: "2026-10-03",
        product: "Actions",
        sku: "Actions Linux",
        unitType: "minutes",
        quantity: 100,
        grossAmount: 0.8,
        netAmount: 0,
      },
    ]);
    expect(days[0].actionsMinutes).toBe(100);
    const records = recordsFromBillingUsage(
      [
        {
          date: "2026-10-03",
          product: "Actions",
          sku: "Actions Linux",
          unitType: "minutes",
          quantity: 100,
          grossAmount: 0.8,
          netAmount: 0,
        },
      ],
      "test",
    );
    expect(records.some((r) => r.metricId === "actions-minutes" && r.value === 100)).toBe(true);
  });

  test("codespaces machine-hours convert to core-hours by the SKU core count", () => {
    const days = aggregateBillingUsage([
      {
        date: "2026-10-03",
        product: "codespaces",
        sku: "Codespaces 4-core",
        unitType: "Hours",
        quantity: 1.5,
        grossAmount: 0,
        netAmount: 0,
      },
      {
        date: "2026-10-03",
        product: "codespaces",
        sku: "Codespaces prebuild",
        unitType: "Hours",
        quantity: 2,
        grossAmount: 0,
        netAmount: 0,
      },
    ]);
    // 1.5 machine-hours x 4 cores = 6 core-hours; unparseable SKU counts at face value.
    expect(days[0].codespacesHours).toBe(8);
  });
});

describe("github-billing malformed items and SKU multipliers (#3739)", () => {
  test("non-object items are skipped and valid neighbors still aggregate", () => {
    const days = aggregateBillingUsage([
      null,
      42,
      ["not", "an", "item"],
      {
        date: "2026-10-04",
        product: "actions",
        sku: "Actions Linux",
        unitType: "Minutes",
        quantity: 30,
        grossAmount: 0.24,
        netAmount: 0.24,
      },
    ] as unknown as BillingUsageItem[]);
    expect(days).toHaveLength(1);
    expect(days[0].actionsMinutes).toBe(30);
    expect(days[0].grossUsd).toBe(0.24);
  });

  test("0-core SKU falls back to face value instead of erasing hours", () => {
    const days = aggregateBillingUsage([
      {
        date: "2026-10-04",
        product: "codespaces",
        sku: "Codespaces 0-core",
        unitType: "Hours",
        quantity: 3,
        grossAmount: 0,
        netAmount: 0,
      },
    ]);
    expect(days[0].codespacesHours).toBe(3);
  });

  test("huge-core SKU falls back to face value instead of an unbounded multiplier", () => {
    const days = aggregateBillingUsage([
      {
        date: "2026-10-04",
        product: "codespaces",
        sku: "Codespaces 999999999999999999999999999999-core",
        unitType: "Hours",
        quantity: 2,
        grossAmount: 0,
        netAmount: 0,
      },
    ]);
    expect(days[0].codespacesHours).toBe(2);
  });

  test("control: 4-core x 2 hours still converts to 8 core-hours", () => {
    const days = aggregateBillingUsage([
      {
        date: "2026-10-04",
        product: "codespaces",
        sku: "Codespaces 4-core",
        unitType: "Hours",
        quantity: 2,
        grossAmount: 0,
        netAmount: 0,
      },
    ]);
    expect(days[0].codespacesHours).toBe(8);
  });
});
