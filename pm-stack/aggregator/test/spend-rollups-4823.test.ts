import { describe, expect, test } from "bun:test";
import { mkdtemp, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import {
  buildSpendRollups,
  selectRollups,
  SpendRollupStore,
  validateSpendRollup,
  windowRange,
  type SpendRollupEvent,
} from "../src/spend-rollups.ts";

// Oracle: the #1259 design invariants — per-unit conservation against the
// source events, complete-vs-partial coverage labelling, coverageStart
// stamping, immutability per attributionVersion, and no merged field.
const COVERAGE_START = "2026-10-05T00:00:00.000Z";
const FULL_NOV: [{ from: string; to: string; status: "ok" }] = [
  { from: "2026-10-05T00:00:00.000Z", to: "2026-12-01T00:00:00.000Z", status: "ok" },
];

const EVENTS: SpendRollupEvent[] = [
  { ts: "2026-11-04T10:00:00Z", projectId: "proj-a", epicRef: "epic-1", tokensByPool: { meta: 100 }, paygUsd: 3, sharePctByPool: { meta: 1.5 } },
  { ts: "2026-11-05T10:00:00Z", projectId: "proj-a", epicRef: "epic-1", tokensByPool: { meta: 50, openai: 20 }, paygUsd: 2, sharePctByPool: { meta: 0.5 } },
  { ts: "2026-11-06T10:00:00Z", projectId: null, epicRef: null, tokensByPool: { meta: 7 }, paygUsd: 1 },
];

describe("windowRange", () => {
  test("week is the ISO Monday-start UTC week; month is the calendar month", () => {
    expect(windowRange("week", "2026-11-04T10:00:00Z")).toEqual({
      start: "2026-11-02T00:00:00.000Z",
      end: "2026-11-09T00:00:00.000Z",
    });
    expect(windowRange("month", "2026-11-04T10:00:00Z")).toEqual({
      start: "2026-11-01T00:00:00.000Z",
      end: "2026-12-01T00:00:00.000Z",
    });
  });
});

describe("buildSpendRollups", () => {
  const built = buildSpendRollups(EVENTS, { attributionVersion: "v1", coverageStart: COVERAGE_START, coverage: FULL_NOV });
  const find = (wk: string, sk: string, sid: string, unit: string, pool: string | null) =>
    built.find((r) => r.windowKind === wk && r.scopeKind === sk && r.scopeId === sid && r.unit === unit && r.pool === pool);

  test("project and epic scopes both roll up, per pool, with units unmerged", () => {
    expect(find("week", "project", "proj-a", "tokens", "meta")?.value).toBe(150);
    expect(find("week", "project", "proj-a", "tokens", "openai")?.value).toBe(20);
    expect(find("week", "epic", "epic-1", "tokens", "meta")?.value).toBe(150);
    expect(find("month", "project", "proj-a", "usd", null)?.value).toBe(5);
    expect(find("week", "project", "proj-a", "share_pct", "meta")?.value).toBe(2);
    // No record merges units: every record carries exactly one unit.
    for (const r of built) expect(["tokens", "usd", "share_pct"]).toContain(r.unit);
  });

  test("unattributed events land in the explicit unattributed scope", () => {
    expect(find("week", "unattributed", "unattributed", "tokens", "meta")?.value).toBe(7);
  });

  test("reconciliation: project + unattributed token totals equal source totals per pool", () => {
    const weekTokens = built.filter((r) => r.windowKind === "week" && r.unit === "tokens" && r.scopeKind !== "epic");
    const byPool = new Map<string, number>();
    for (const r of weekTokens) byPool.set(r.pool!, (byPool.get(r.pool!) ?? 0) + r.value);
    expect(byPool.get("meta")).toBe(157);
    expect(byPool.get("openai")).toBe(20);
  });

  test("complete coverage is labelled complete and coverageStart is stamped", () => {
    for (const r of built) {
      expect(r.coverage).toBe("complete");
      expect(r.coverageStart).toBe(COVERAGE_START);
      expect(r.attributionVersion).toBe("v1");
    }
  });

  test("partial coverage: window not fully covered by an ok span is labelled partial", () => {
    const partial = buildSpendRollups(EVENTS, {
      attributionVersion: "v1",
      coverageStart: COVERAGE_START,
      coverage: [{ from: "2026-11-04T00:00:00.000Z", to: "2026-11-05T00:00:00.000Z", status: "ok" }],
    });
    expect(partial.length).toBeGreaterThan(0);
    for (const r of partial) expect(r.coverage).toBe("partial");
  });

  test("failed/unknown source coverage never emits zero spend: no events, no records", () => {
    const none = buildSpendRollups([], {
      attributionVersion: "v1",
      coverageStart: COVERAGE_START,
      coverage: [{ from: "2026-11-01T00:00:00.000Z", to: "2026-12-01T00:00:00.000Z", status: "failed" }],
    });
    expect(none).toEqual([]);
  });

  test("unknown charge state suppresses the USD record instead of emitting a partial sum or zero", () => {
    const mixed: SpendRollupEvent[] = [
      { ts: "2026-11-04T10:00:00Z", projectId: "proj-a", epicRef: null, tokensByPool: { meta: 1 }, paygUsd: 3 },
      { ts: "2026-11-05T10:00:00Z", projectId: "proj-a", epicRef: null, tokensByPool: { meta: 1 }, paygUsd: null },
    ];
    const out = buildSpendRollups(mixed, { attributionVersion: "v1", coverageStart: COVERAGE_START, coverage: FULL_NOV });
    expect(out.find((r) => r.unit === "usd")).toBeUndefined();
    expect(out.find((r) => r.unit === "tokens")?.value).toBe(2);
  });
});

describe("SpendRollupStore", () => {
  test("append/read round-trip, idempotent re-append, immutability, restatement coexistence", async () => {
    const dir = await mkdtemp(join(tmpdir(), "rollups-4823-"));
    try {
      const store = new SpendRollupStore(dir);
      const v1 = buildSpendRollups(EVENTS, { attributionVersion: "v1", coverageStart: COVERAGE_START, coverage: FULL_NOV });
      expect((await store.append(v1)).appended).toBe(v1.length);
      expect((await store.append(v1)).appended).toBe(0); // identical re-append is a no-op

      const changed = v1.map((r) => ({ ...r }));
      changed[0] = { ...changed[0]!, value: changed[0]!.value + 1 };
      await expect(store.append(changed)).rejects.toThrow(/immutable per attributionVersion/);

      // Restatement: same inputs, new version, different values — stored alongside.
      const v2Events = EVENTS.map((e) => ({ ...e, tokensByPool: { ...e.tokensByPool, meta: (e.tokensByPool?.meta ?? 0) + 1 } }));
      const v2 = buildSpendRollups(v2Events, { attributionVersion: "v2", coverageStart: COVERAGE_START, coverage: FULL_NOV });
      await store.append(v2);
      const all = await store.readAll();
      expect(all.filter((r) => r.attributionVersion === "v1").length).toBe(v1.length);
      expect(all.filter((r) => r.attributionVersion === "v2").length).toBe(v2.length);

      // Reader defaults to the newest version; an explicit version reads the old.
      const latest = selectRollups(all);
      expect(latest.every((r) => r.attributionVersion === "v2")).toBe(true);
      const old = selectRollups(all, "v1");
      expect(old.length).toBe(v1.length);
      expect(old.every((r) => r.attributionVersion === "v1")).toBe(true);
    } finally {
      await rm(dir, { recursive: true, force: true });
    }
  });

  test("a synthetic merged cross-unit field is rejected by the schema", async () => {
    const dir = await mkdtemp(join(tmpdir(), "rollups-4823-"));
    try {
      const store = new SpendRollupStore(dir);
      const [first] = buildSpendRollups(EVENTS, { attributionVersion: "v1", coverageStart: COVERAGE_START, coverage: FULL_NOV });
      const merged = { ...first!, totalSpend: 123 } as unknown;
      expect(() => validateSpendRollup(merged)).toThrow(/unexpected field "totalSpend"/);
      await expect(store.append([merged as never])).rejects.toThrow(/unexpected field/);
    } finally {
      await rm(dir, { recursive: true, force: true });
    }
  });
});
