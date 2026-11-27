// #4825 wiring slice: runAlertTick runs the cost-anomaly detector
// when (and only when) a caller injects an evidence loader, posting
// warning incidents to the same alert store as every other throat
// incident. Oracles: the tick's own store boundary — what postIncident
// receives, and that the tick survives a failing loader.
import { describe, expect, test } from "bun:test";
import { runAlertTick, type AlertsStore, type NewIncident } from "../src/alerts.ts";
import type { Config } from "../src/config.ts";
import type { CostEvidence } from "../src/cost-anomaly.ts";
import type { SpendRollup } from "../src/spend-rollups.ts";

const tickConfig = {
  publicOrigin: "https://alerts.example.ts.net",
  alerts: { ackWindowMin: 30, tickMs: 60_000 },
} as Config;

const now = Date.parse("2026-11-19T12:00:00Z");
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
const windowFields = {
  windowKind: spend.windowKind,
  windowStart: spend.windowStart,
  windowEnd: spend.windowEnd,
  attributionVersion: spend.attributionVersion,
  coverage: spend.coverage,
};
const burn: CostEvidence = {
  kind: "burn-without-progress",
  spend,
  closedChildren: { ...windowFields, scopeKind: "epic", scopeId: spend.scopeId, count: 0 },
};

function wiringStore() {
  const posted: NewIncident[] = [];
  const store = {
    listSubscriptions: async () => [],
    latestIncidentEventId: async () => 0,
    incidentEventsAfter: async () => [],
    pendingDeliveries: async () => [],
    pendingEscalations: async () => [],
    postIncident: async (incident: NewIncident) => {
      posted.push(incident);
      return { id: `incident-${posted.length}`, created: true };
    },
  };
  return { store: store as unknown as AlertsStore, posted };
}

describe("alert tick cost-anomaly wiring (#4825)", () => {
  test("a firing evidence batch posts one warning incident through the throat", async () => {
    const { store, posted } = wiringStore();
    await runAlertTick(tickConfig, store, undefined, {
      now,
      costAnomalyEvidence: async () => [burn],
    });
    expect(posted).toHaveLength(1);
    expect(posted[0].source).toBe("cost-anomaly");
    expect(posted[0].severity).toBe("warning");
    expect(posted[0].title).toContain("burn without progress");
    expect(posted[0].body).toContain("100 tokens");
  });

  test("without an injected loader the tick posts no cost-anomaly incident", async () => {
    const { store, posted } = wiringStore();
    await runAlertTick(tickConfig, store, undefined, { now });
    expect(posted).toHaveLength(0);
  });

  test("non-firing evidence posts nothing", async () => {
    const { store, posted } = wiringStore();
    const quiet: CostEvidence = {
      kind: "burn-without-progress",
      spend,
      closedChildren: { ...windowFields, scopeKind: "epic", scopeId: spend.scopeId, count: 1 },
    };
    await runAlertTick(tickConfig, store, undefined, {
      now,
      costAnomalyEvidence: async () => [quiet],
    });
    expect(posted).toHaveLength(0);
  });

  test("a failing loader is fail-soft: the tick completes and posts nothing", async () => {
    const { store, posted } = wiringStore();
    await runAlertTick(tickConfig, store, undefined, {
      now,
      costAnomalyEvidence: async () => {
        throw new Error("evidence source unavailable");
      },
    });
    expect(posted).toHaveLength(0);
  });
});
