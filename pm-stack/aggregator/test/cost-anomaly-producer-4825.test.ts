import { expect, test } from "bun:test";
import { Database } from "bun:sqlite";
import { mkdtemp, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { AlertsStore } from "../src/alerts.ts";
import { postCostAnomalyTick, type CostEvidence } from "../src/cost-anomaly.ts";
import {
  loadCostAnomalyEvidence,
  type BusiestRootRecord,
  type ClosedChildrenRecord,
  type SourceTotalRecord,
} from "../src/cost-anomaly-producer.ts";
import {
  buildSpendRollups,
  SpendRollupStore,
  type SpendRollup,
  type SpendRollupEvent,
} from "../src/spend-rollups.ts";
import { decodeStoreRow, storeStatement } from "../src/turso-stores.ts";

// Oracle: the #4825 producer contract — evidence is assembled only from
// version-consistent rollup records (selectRollups), auxiliary facts
// stay null until a certified source supplies them, and a complete
// store-to-ledger pass posts exactly the anomalies whose patterns the
// fixtures construct (firing / silent cases per class live in
// cost-anomaly-4825.test.ts; this file covers assembly and the
// end-to-end tick).
const now = new Date("2026-11-19T12:00:00Z");
const COVERAGE_START = "2026-10-05T00:00:00.000Z";
const FULL: [{ from: string; to: string; status: "ok" }] = [
  { from: "2026-10-05T00:00:00.000Z", to: "2026-12-01T00:00:00.000Z", status: "ok" },
];
const W1 = "2026-11-02T00:00:00.000Z";
const W2 = "2026-11-09T00:00:00.000Z";

// Week 1: epic 95 tokens, unattributed 5 (5% of the 100 total).
// Week 2: epic 79 tokens, unattributed 21 (21% of the 100 total).
const EVENTS: SpendRollupEvent[] = [
  { ts: "2026-11-03T10:00:00Z", projectId: null, epicRef: "epic-1", tokensByPool: { eco: 95 } },
  { ts: "2026-11-04T10:00:00Z", projectId: null, epicRef: null, tokensByPool: { eco: 5 } },
  { ts: "2026-11-10T10:00:00Z", projectId: null, epicRef: "epic-1", tokensByPool: { eco: 79 } },
  { ts: "2026-11-11T10:00:00Z", projectId: null, epicRef: null, tokensByPool: { eco: 21 } },
];

async function storeWith(events: SpendRollupEvent[], version = "v1"): Promise<SpendRollupStore> {
  const dir = await mkdtemp(join(tmpdir(), "cost-anomaly-producer-"));
  const store = new SpendRollupStore(dir);
  await store.append(buildSpendRollups(events, { attributionVersion: version, coverageStart: COVERAGE_START, coverage: FULL }));
  return store;
}

const windowOf = (spend: SpendRollup) => ({
  windowKind: spend.windowKind,
  windowStart: spend.windowStart,
  windowEnd: spend.windowEnd,
  coverage: spend.coverage,
  attributionVersion: spend.attributionVersion,
  scopeKind: spend.scopeKind,
  scopeId: spend.scopeId,
});

test("producer assembles burn, dominance and adjacent-window growth evidence from the store", async () => {
  const store = await storeWith(EVENTS);
  try {
    const evidence = await loadCostAnomalyEvidence({ readRollups: () => store.readAll() });
    const ofKind = <K extends CostEvidence["kind"]>(kind: K): Array<Extract<CostEvidence, { kind: K }>> =>
      evidence.filter((e): e is Extract<CostEvidence, { kind: K }> => e.kind === kind);
    // Epic token rollups exist for weeks 1-2 and November: 3 burn + 3
    // dominance. Unattributed rollups are never burn/dominance scopes.
    expect(ofKind("burn-without-progress")).toHaveLength(3);
    expect(ofKind("single-session-dominance")).toHaveLength(3);
    // Only week 2 has an in-store predecessor; November does not.
    const growth = ofKind("unattributed-growth");
    expect(growth).toHaveLength(1);
    const pair = growth[0]!;
    expect(pair.current.windowStart).toBe(W2);
    expect(pair.previous.windowStart).toBe(W1);
    expect(pair.previous.windowEnd).toBe(pair.current.windowStart);
    // No certified sources were injected: every auxiliary fact is null,
    // so the detector must stay silent on the whole batch.
    for (const e of evidence) {
      if (e.kind === "burn-without-progress") expect(e.closedChildren).toBeNull();
      if (e.kind === "single-session-dominance") expect(e.busiestRoot).toBeNull();
      if (e.kind === "unattributed-growth") {
        expect(e.currentTotal).toBeNull();
        expect(e.previousTotal).toBeNull();
      }
    }
  } finally {
    await rm(store.file, { force: true });
  }
});

test("producer pairs only the newest attribution version per window", async () => {
  const store = await storeWith(EVENTS);
  try {
    const restated: SpendRollupEvent[] = [
      { ts: "2026-11-10T10:00:00Z", projectId: null, epicRef: "epic-1", tokensByPool: { eco: 60 } },
      { ts: "2026-11-11T10:00:00Z", projectId: null, epicRef: null, tokensByPool: { eco: 40 } },
    ];
    await store.append(
      buildSpendRollups(restated, { attributionVersion: "v2", coverageStart: COVERAGE_START, coverage: FULL }),
    );
    const evidence = await loadCostAnomalyEvidence({ readRollups: () => store.readAll() });
    const week2Burn = evidence.filter(
      (e): e is Extract<CostEvidence, { kind: "burn-without-progress" }> =>
        e.kind === "burn-without-progress" && e.spend.windowKind === "week" && e.spend.windowStart === W2,
    );
    // The v1 and v2 records for week 2 must not both appear: one
    // evidence, carrying the restated value.
    expect(week2Burn).toHaveLength(1);
    expect(week2Burn[0]!.spend.value).toBe(60);
    expect(week2Burn[0]!.spend.attributionVersion).toBe("v2");
  } finally {
    await rm(store.file, { force: true });
  }
});

test("empty store produces no evidence", async () => {
  const dir = await mkdtemp(join(tmpdir(), "cost-anomaly-producer-"));
  try {
    const store = new SpendRollupStore(dir);
    expect(await loadCostAnomalyEvidence({ readRollups: () => store.readAll() })).toEqual([]);
  } finally {
    await rm(dir, { recursive: true, force: true });
  }
});

test("end to end: store + certified aux sources post exactly the constructed anomalies, deduped on retick", async () => {
  const store = await storeWith(EVENTS);
  const db = new Database(":memory:");
  try {
    db.exec(`create table alerts_incidents(id text primary key default (lower(hex(randomblob(16)))), source text, incident_key text, severity text, title text, body text, status text default 'open', occurrences integer default 1, first_seen_at text default (strftime('%Y-%m-%dT%H:%M:%fZ','now')), last_seen_at text, delivered_at text, escalated_at text, acked_at text, closed_at text);
      create unique index alert_open_key on alerts_incidents(source,incident_key) where status <> 'closed';
      create table alerts_incident_events(id integer primary key, incident_id text, kind text, detail text);`);
    const alerts = new AlertsStore({
      ref: "fixture",
      async query<T>(input: string): Promise<T[]> {
        const { sql, args } = storeStatement(input);
        return db
          .query(sql)
          .all(...args.map((a) => a.value))
          .map((r) => decodeStoreRow(r as Record<string, unknown>)) as T[];
      },
    });
    const deps = {
      readRollups: () => store.readAll(),
      closedChildren: (spend: SpendRollup): ClosedChildrenRecord => ({ ...windowOf(spend), count: 0 }),
      busiestRoot: (spend: SpendRollup): BusiestRootRecord => ({
        ...windowOf(spend),
        pool: spend.pool ?? "eco",
        tokens: spend.windowKind === "week" && spend.windowStart === W2 ? 51 : 10,
      }),
      sourceTotal: (rollup: SpendRollup): SourceTotalRecord | null =>
        rollup.windowKind === "week"
          ? {
              windowKind: rollup.windowKind,
              windowStart: rollup.windowStart,
              windowEnd: rollup.windowEnd,
              coverage: rollup.coverage,
              attributionVersion: rollup.attributionVersion,
              unit: rollup.unit,
              pool: rollup.pool,
              value: 100,
            }
          : null,
      monthlyCharges: () => [{ service: "Modal", month: "2026-10", totalUsd: 8, coverage: "complete" as const }],
      serviceActivity: () => ({ service: "Modal", month: "2026-10", sessions: 0, quotaActivity: 0, coverage: "complete" as const }),
    };
    const loadEvidence = () => loadCostAnomalyEvidence(deps);
    // Fires: burn for both finished epic weeks, dominance for week 2
    // (51/79 > 50%), unattributed growth week 2 (21% vs 5%), and the
    // October charge with zero activity. November month windows are
    // unfinished at `now`, so their evidence stays silent.
    expect(await postCostAnomalyTick({ enabled: true, now, loadEvidence, store: alerts })).toBe(5);
    expect(await postCostAnomalyTick({ enabled: true, now, loadEvidence, store: alerts })).toBe(5);
    const rows = db.query("select title, occurrences from alerts_incidents order by title").all() as Array<{
      title: string;
      occurrences: number;
    }>;
    expect(rows).toHaveLength(5);
    expect(rows.every((r) => r.occurrences === 2)).toBe(true);
    expect(rows.map((r) => r.title.split(":")[0])).toEqual([
      "burn without progress",
      "burn without progress",
      "charge without activity",
      "single session dominance",
      "unattributed growth",
    ]);
    expect(await alerts.pendingDeliveries()).toEqual([]);
  } finally {
    db.close();
    await rm(store.file, { force: true });
  }
});
