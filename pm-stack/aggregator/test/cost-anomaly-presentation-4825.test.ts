// #4825 pre-activation verification: source-to-rollup reconciliation
// and warning presentation, without sending pushes and without
// activating the producer in production (COST-ANOMALY.md "Still
// needed"). Oracles: (1) the values printed on each posted warning
// reconcile exactly to the rollup-store records and the certified
// census totals that produced them — the producer is fed ONLY through
// the real certified source modules (cost-anomaly-sources.ts,
// cost-anomaly-rollup-sources.ts), never through stubs; (2) the
// warnings present on the alert throat's page surface (renderAlerts,
// the /alerts page — per render-attention.ts, incident alerts live on
// /alerts; the Needs-you queue carries label-census items) with their
// evidence named (scope, window, both compared values, drill-down
// link), at severity "warning", with no delivery or escalation —
// warnings never push in v1.
import { expect, test } from "bun:test";
import { Database } from "bun:sqlite";
import { mkdtemp, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { AlertsStore } from "../src/alerts.ts";
import { postCostAnomalyTick } from "../src/cost-anomaly.ts";
import { loadCostAnomalyEvidence, type CostAnomalyProducerDeps } from "../src/cost-anomaly-producer.ts";
import {
  certifiedMonthlyCharges,
  certifiedServiceActivity,
  type ServiceActivityCensusEntry,
} from "../src/cost-anomaly-sources.ts";
import {
  certifiedBusiestRoot,
  certifiedClosedChildren,
  certifiedSourceTotal,
  type ClosureCensusEntry,
  type RootSessionCensusEntry,
  type SourcePopulationCensusEntry,
} from "../src/cost-anomaly-rollup-sources.ts";
import { renderAlerts } from "../src/render.ts";
import type { Provenance } from "../src/model.ts";
import type { MatchedCharge } from "../src/spend-dollars.ts";
import { buildSpendRollups, SpendRollupStore, type SpendRollup, type SpendRollupEvent } from "../src/spend-rollups.ts";
import { decodeStoreRow, storeStatement } from "../src/turso-stores.ts";

const now = new Date("2026-11-19T12:00:00Z");
const COVERAGE_START = "2026-10-05T00:00:00.000Z";
const FULL: [{ from: string; to: string; status: "ok" }] = [
  { from: "2026-10-05T00:00:00.000Z", to: "2026-12-01T00:00:00.000Z", status: "ok" },
];
// The charges pipeline's completeness span must cover the whole
// calendar month the charge class compares (October), not just the
// rollup coverage window.
const MONTH_FULL: [{ from: string; to: string; status: "ok" }] = [
  { from: "2026-09-01T00:00:00.000Z", to: "2026-12-01T00:00:00.000Z", status: "ok" },
];
const W2 = "2026-11-09T00:00:00.000Z";

// Same constructed patterns as the producer suite: week 1 epic 95
// tokens with 5 unattributed (5% of 100); week 2 epic 79 with 21
// unattributed (21% of 100).
const EVENTS: SpendRollupEvent[] = [
  { ts: "2026-11-03T10:00:00Z", projectId: null, epicRef: "epic-1", tokensByPool: { eco: 95 } },
  { ts: "2026-11-04T10:00:00Z", projectId: null, epicRef: null, tokensByPool: { eco: 5 } },
  { ts: "2026-11-10T10:00:00Z", projectId: null, epicRef: "epic-1", tokensByPool: { eco: 79 } },
  { ts: "2026-11-11T10:00:00Z", projectId: null, epicRef: null, tokensByPool: { eco: 21 } },
];

// Two matched Modal charges in October 2026 totalling exactly USD 8.
const MATCHED: MatchedCharge[] = [
  { service: "Modal", at: "2026-10-03", amountUsd: 5 },
  { service: "Modal", at: "2026-10-20", amountUsd: 3 },
];

const provenance: Provenance = {
  board: "fixture",
  roadmap: "fixture",
  items: "fixture",
  comments: "fixture",
  quota: "fixture",
  usage: "fixture",
  attention: "fixture",
  orchestration: "fixture",
  dns: "fixture",
  registry: "fixture",
  aa: "fixture",
};

async function rollupStore(): Promise<SpendRollupStore> {
  const dir = await mkdtemp(join(tmpdir(), "cost-anomaly-presentation-"));
  const store = new SpendRollupStore(dir);
  await store.append(
    buildSpendRollups(EVENTS, { attributionVersion: "v1", coverageStart: COVERAGE_START, coverage: FULL }),
  );
  return store;
}

function alertsStore() {
  const db = new Database(":memory:");
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
  return { db, alerts };
}

/** Producer deps wired ONLY through the certified source modules,
 * with censuses derived from the store's own rollup records (so the
 * census keys can never drift from the rollups they certify). */
async function certifiedDeps(
  store: SpendRollupStore,
  opts: { week2Closures?: number; octoberSessions?: number } = {},
): Promise<CostAnomalyProducerDeps> {
  const rollups = await store.readAll();
  const epicWeeks = rollups.filter((r) => r.scopeKind === "epic" && r.windowKind === "week");
  const closureCensus: ClosureCensusEntry[] = epicWeeks.map((r) => ({
    scopeKind: r.scopeKind,
    scopeId: r.scopeId,
    windowKind: r.windowKind,
    windowStart: r.windowStart,
    windowEnd: r.windowEnd,
    closedChildren: r.windowStart === W2 ? (opts.week2Closures ?? 0) : 0,
    coverage: FULL,
  }));
  const rootCensus: RootSessionCensusEntry[] = epicWeeks.flatMap((r) =>
    r.windowStart === W2
      ? [
          {
            scopeId: r.scopeId,
            windowKind: r.windowKind,
            windowStart: r.windowStart,
            windowEnd: r.windowEnd,
            pool: "eco",
            rootSessionId: "root-a",
            tokens: 51,
            coverage: FULL,
          },
          {
            scopeId: r.scopeId,
            windowKind: r.windowKind,
            windowStart: r.windowStart,
            windowEnd: r.windowEnd,
            pool: "eco",
            rootSessionId: "root-b",
            tokens: 20,
            coverage: FULL,
          },
        ]
      : [
          {
            scopeId: r.scopeId,
            windowKind: r.windowKind,
            windowStart: r.windowStart,
            windowEnd: r.windowEnd,
            pool: "eco",
            rootSessionId: "root-a",
            tokens: 10,
            coverage: FULL,
          },
        ],
  );
  const populationCensus: SourcePopulationCensusEntry[] = rollups
    .filter((r) => r.scopeKind === "unattributed" && r.windowKind === "week")
    .map((r) => ({
      windowKind: r.windowKind,
      windowStart: r.windowStart,
      windowEnd: r.windowEnd,
      unit: r.unit,
      pool: r.pool,
      value: 100,
      coverage: FULL,
    }));
  const activityCensus: ServiceActivityCensusEntry[] = [
    { service: "Modal", month: "2026-10", sessions: opts.octoberSessions ?? 0, quotaActivity: 0, coverage: MONTH_FULL },
  ];
  return {
    readRollups: () => store.readAll(),
    closedChildren: certifiedClosedChildren(closureCensus),
    busiestRoot: certifiedBusiestRoot(rootCensus),
    sourceTotal: certifiedSourceTotal(populationCensus),
    monthlyCharges: certifiedMonthlyCharges(MATCHED, MONTH_FULL),
    serviceActivity: certifiedServiceActivity(activityCensus),
  };
}

test("reconciliation: warnings posted through the certified sources carry exactly the store and census values", async () => {
  const store = await rollupStore();
  const { db, alerts } = alertsStore();
  try {
    const deps = await certifiedDeps(store);
    const loadEvidence = () => loadCostAnomalyEvidence(deps);
    expect(await postCostAnomalyTick({ enabled: true, now, loadEvidence, store: alerts })).toBe(5);

    const incidents = await alerts.listIncidents();
    expect(incidents).toHaveLength(5);
    expect(incidents.map((i) => i.title.split(":")[0]).sort()).toEqual([
      "burn without progress",
      "burn without progress",
      "charge without activity",
      "single session dominance",
      "unattributed growth",
    ]);
    for (const incident of incidents) {
      expect(incident.source).toBe("cost-anomaly");
      expect(incident.severity).toBe("warning");
      expect(incident.status).toBe("open");
      // No push, no escalation: delivery fields stay null, and the
      // delivery batch (critical-only by construction) stays empty.
      expect(incident.delivered_at).toBeNull();
      expect(incident.escalated_at).toBeNull();
    }
    expect(await alerts.pendingDeliveries()).toEqual([]);

    // Each warning names its evidence, and the compared values are
    // the rollup-store values (95 / 79 epic tokens; 5% -> 21%
    // unattributed of the certified 100 total) and the census values
    // (0 closures; busiest root 51 of 79; USD 8 matched = 5 + 3).
    const bodies = incidents.map((i) => i.body ?? "");
    expect(bodies.some((b) => b.includes("95 tokens consumed; 0 child closures"))).toBe(true);
    expect(bodies.some((b) => b.includes("79 tokens consumed; 0 child closures"))).toBe(true);
    expect(bodies.some((b) => b.includes("51 tokens in busiest root / 79 tokens total"))).toBe(true);
    expect(bodies.some((b) => b.includes("21% unattributed vs 5% in preceding window"))).toBe(true);
    expect(bodies.some((b) => b.includes("matched PAYG USD 8; 0 sessions / quota activity"))).toBe(true);
    // Drill-down links ride along: rollup classes point at /projects,
    // the charge class at /quota.
    expect(bodies.filter((b) => b.includes("/projects"))).toHaveLength(4);
    expect(bodies.filter((b) => b.includes("/quota"))).toHaveLength(1);
  } finally {
    db.close();
    await rm(store.file, { force: true });
  }
});

test("presentation: the posted warnings render on the alerts page with their evidence, as warnings", async () => {
  const store = await rollupStore();
  const { db, alerts } = alertsStore();
  try {
    const deps = await certifiedDeps(store);
    await postCostAnomalyTick({ enabled: true, now, loadEvidence: () => loadCostAnomalyEvidence(deps), store: alerts });
    const incidents = await alerts.listIncidents();
    const html = renderAlerts(
      { available: true, incidents, events: await alerts.recentEvents(), subscriptions: [], ackWindowMin: 30 },
      provenance,
      "",
    );
    for (const incident of incidents) {
      expect(html).toContain(incident.title);
      expect(html).toContain(incident.body ?? "");
    }
    expect(html).toContain("warning");
    expect(html).toContain("cost-anomaly");
    expect(html).toContain("51 tokens in busiest root / 79 tokens total");
    expect(html).toContain("matched PAYG USD 8; 0 sessions / quota activity");
    // The page's own v1 statement: warnings are recorded, not pushed.
    expect(html).toContain("Warnings are recorded here and in the ledger");
  } finally {
    db.close();
    await rm(store.file, { force: true });
  }
});

test("reconciliation suppression: certified real activity and closures silence exactly their own classes", async () => {
  const store = await rollupStore();
  const { db, alerts } = alertsStore();
  try {
    // Week 2 gains 3 certified closures (burn silent for week 2 only)
    // and October gains 2 certified Modal sessions (charge silent);
    // dominance and growth are untouched and still fire.
    const deps = await certifiedDeps(store, { week2Closures: 3, octoberSessions: 2 });
    expect(
      await postCostAnomalyTick({
        enabled: true,
        now,
        loadEvidence: () => loadCostAnomalyEvidence(deps),
        store: alerts,
      }),
    ).toBe(3);
    const titles = (await alerts.listIncidents()).map((i) => i.title.split(":")[0]).sort();
    expect(titles).toEqual(["burn without progress", "single session dominance", "unattributed growth"]);
  } finally {
    db.close();
    await rm(store.file, { force: true });
  }
});
