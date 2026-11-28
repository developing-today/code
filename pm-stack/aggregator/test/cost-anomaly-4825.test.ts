import { expect, test } from "bun:test";
import { Database } from "bun:sqlite";
import { AlertsStore } from "../src/alerts.ts";
import { detectCostAnomaly, postCostAnomalyTick, type CostEvidence } from "../src/cost-anomaly.ts";
import type { SpendRollup } from "../src/spend-rollups.ts";
import { decodeStoreRow, storeStatement } from "../src/turso-stores.ts";

const now = new Date("2026-11-19T12:00:00Z");
const spend: SpendRollup = {
  kind: "spend-rollup/v1",
  windowKind: "week",
  windowStart: "2026-11-09T00:00:00.000Z",
  windowEnd: "2026-11-16T00:00:00.000Z",
  scopeKind: "epic",
  scopeId: "example/code#4825",
  unit: "tokens",
  pool: "eco",
  value: 100,
  coverage: "complete",
  coverageStart: "2026-11-01",
  attributionVersion: "v1",
};
const window = {
  windowKind: spend.windowKind,
  windowStart: spend.windowStart,
  windowEnd: spend.windowEnd,
  attributionVersion: spend.attributionVersion,
  coverage: spend.coverage,
};
const burn = {
  kind: "burn-without-progress",
  spend,
  closedChildren: { ...window, scopeKind: "epic", scopeId: spend.scopeId, count: 0 },
} as const;
const dominant = {
  kind: "single-session-dominance",
  spend,
  busiestRoot: { ...window, scopeKind: "epic", scopeId: spend.scopeId, pool: "eco", tokens: 51 },
} as const;
const growth = {
  kind: "unattributed-growth",
  current: { ...spend, scopeKind: "unattributed", scopeId: "unattributed", value: 21 },
  previous: {
    ...spend,
    scopeKind: "unattributed",
    scopeId: "unattributed",
    value: 10,
    windowStart: "2026-11-02T00:00:00.000Z",
    windowEnd: spend.windowStart,
  },
  currentTotal: { ...window, unit: "tokens", pool: "eco", value: 100 },
  previousTotal: {
    ...window,
    unit: "tokens",
    pool: "eco",
    value: 100,
    windowStart: "2026-11-02T00:00:00.000Z",
    windowEnd: spend.windowStart,
  },
} as const;
const charge = {
  kind: "charge-without-activity",
  charges: { service: "Example Service", month: "2026-10", totalUsd: 8, coverage: "complete" },
  activity: { service: "Example Service", month: "2026-10", sessions: 0, quotaActivity: 0, coverage: "complete" },
} as const;

// Oracles: design #1259 thresholds and certified zero against independent fixtures.
test("burn requires positive spend and certified zero closures for the same scope and window", () => {
  expect(detectCostAnomaly(burn, now)?.body).toContain("100 tokens");
  expect(detectCostAnomaly({ ...burn, spend: { ...spend, value: 0 } }, now)).toBeNull();
  expect(detectCostAnomaly({ ...burn, closedChildren: { ...burn.closedChildren, count: 1 } }, now)).toBeNull();
  expect(
    detectCostAnomaly({ ...burn, closedChildren: { ...burn.closedChildren, coverage: "partial" } }, now),
  ).toBeNull();
  expect(detectCostAnomaly({ ...burn, closedChildren: { ...burn.closedChildren, scopeId: "wrong" } }, now)).toBeNull();
  expect(detectCostAnomaly(burn, new Date(spend.windowStart))).toBeNull();
});

test("dominance uses like-for-like tokens and strict 50% threshold", () => {
  expect(detectCostAnomaly(dominant, now)?.body).toContain("51 tokens");
  for (const tokens of [49, 50, 101, NaN]) {
    expect(detectCostAnomaly({ ...dominant, busiestRoot: { ...dominant.busiestRoot, tokens } }, now)).toBeNull();
  }
  expect(
    detectCostAnomaly({ ...dominant, busiestRoot: { ...dominant.busiestRoot, coverage: "partial" } }, now),
  ).toBeNull();
  expect(detectCostAnomaly({ ...dominant, busiestRoot: { ...dominant.busiestRoot, pool: "wrong" } }, now)).toBeNull();
});

test("growth requires adjacent complete comparable windows and strictly over ten points", () => {
  expect(detectCostAnomaly(growth, now)?.body).toContain("21%");
  for (const value of [19, 20, 101]) {
    expect(detectCostAnomaly({ ...growth, current: { ...growth.current, value } }, now)).toBeNull();
  }
  expect(detectCostAnomaly({ ...growth, previous: { ...growth.previous, coverage: "partial" } }, now)).toBeNull();
  expect(detectCostAnomaly({ ...growth, previous: { ...growth.previous, attributionVersion: "v2" } }, now)).toBeNull();
  expect(detectCostAnomaly({ ...growth, currentTotal: null }, now)).toBeNull();
  for (const delta of [
    { pool: "wrong" },
    { unit: "usd" as const },
    { coverage: "partial" as const },
    { windowStart: growth.previous.windowStart },
  ]) {
    expect(detectCostAnomaly({ ...growth, currentTotal: { ...growth.currentTotal, ...delta } }, now)).toBeNull();
  }
});

test("charge requires matched dollars and complete, service-wide zero activity in a finished month", () => {
  expect(detectCostAnomaly(charge, now)?.body).toContain("USD 8");
  expect(detectCostAnomaly({ ...charge, activity: { ...charge.activity, quotaActivity: 1 } }, now)).toBeNull();
  expect(detectCostAnomaly({ ...charge, activity: { ...charge.activity, coverage: "partial" } }, now)).toBeNull();
  expect(detectCostAnomaly({ ...charge, activity: null }, now)).toBeNull();
  for (const delta of [{ month: "2026-09" }, { service: "Different Service" }, { sessions: 1 }]) {
    expect(detectCostAnomaly({ ...charge, activity: { ...charge.activity, ...delta } }, now)).toBeNull();
  }
  expect(detectCostAnomaly({ ...charge, charges: { ...charge.charges, month: "2026-13" } }, now)).toBeNull();
  expect(detectCostAnomaly({ ...charge, charges: { ...charge.charges, totalUsd: 0 } }, now)).toBeNull();
  expect(detectCostAnomaly(charge, new Date("2026-10-31T00:00:00Z"))).toBeNull();
});

test("intake is disabled by default and does not call its sources", async () => {
  const unexpected = async (): Promise<never> => {
    throw new Error("unexpected I/O");
  };
  expect(await postCostAnomalyTick({ now, loadEvidence: unexpected, store: { postIncident: unexpected } })).toBe(0);
});

test("actual alert ledger dedupes repeat ticks and queues no push for warnings", async () => {
  const db = new Database(":memory:");
  try {
    db.exec(`create table alerts_incidents(id text primary key default (lower(hex(randomblob(16)))), source text, incident_key text, severity text, title text, body text, status text default 'open', occurrences integer default 1, first_seen_at text default (strftime('%Y-%m-%dT%H:%M:%fZ','now')), last_seen_at text, delivered_at text, escalated_at text, acked_at text, closed_at text);
      create unique index alert_open_key on alerts_incidents(source,incident_key) where status <> 'closed';
      create table alerts_incident_events(id integer primary key, incident_id text, kind text, detail text);`);
    const store = new AlertsStore({
      ref: "fixture",
      async query<T>(input: string): Promise<T[]> {
        const { sql, args } = storeStatement(input);
        return db
          .query(sql)
          .all(...args.map((a) => a.value))
          .map((r) => decodeStoreRow(r as Record<string, unknown>)) as T[];
      },
    });
    const loadEvidence = async (): Promise<readonly CostEvidence[]> => [burn, burn];
    const opts = { enabled: true, now, loadEvidence, store };
    expect(await postCostAnomalyTick(opts)).toBe(1);
    expect(await postCostAnomalyTick(opts)).toBe(1);
    expect(db.query("select severity, occurrences from alerts_incidents").all()).toEqual([
      { severity: "warning", occurrences: 2 },
    ]);
    expect(await store.pendingDeliveries()).toEqual([]);
    expect(await store.pendingEscalations(0)).toEqual([]);
  } finally {
    db.close();
  }
});

test("disagreeing warning evidence for one key is not posted in either input order", async () => {
  const changed = { ...charge, charges: { ...charge.charges, totalUsd: 12 } };
  const store = {
    postIncident: async (): Promise<never> => {
      throw new Error("order selected evidence");
    },
  };
  for (const facts of [
    [charge, changed],
    [changed, charge, changed],
  ]) {
    expect(await postCostAnomalyTick({ enabled: true, now, store, loadEvidence: async () => facts })).toBe(0);
  }
});
