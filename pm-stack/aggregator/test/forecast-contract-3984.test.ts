import { describe, expect, test } from "bun:test";
import {
  FORECAST_CONTRACT_VERSION,
  epicForecastContract,
  formatForecastSummary,
  quotaForecastContract,
  stallForecastContract,
  subjectForecastContracts,
  type ForecastContract,
} from "../src/forecast-contract.ts";
import { deriveEpicEta, type StallRisk } from "../src/forecast.ts";
import type { ItemHistoryCutoff, ItemHistoryItem } from "../src/item-history.ts";
import type { ProjectPoolItem, RoadmapRow } from "../src/model.ts";
import { buildRoadmapDrill, projectsBodyHtml, renderRoadmap } from "../src/render.ts";
import { reconcileRoadmap } from "../src/roadmap-reconcile.ts";

// #3984: one forecast contract, consumed by the API and rendered by
// Roadmap and Projects. ORACLE: the §4 field list and honesty states
// of doc/2026-11-01T21-07-00Z_design_pm-stack_forecasting.md — every
// contract carries scope/subject, asOf, model version, input
// window/coverage, remaining units, nullable P50/P80, confidence,
// baseline source, and structured reasons/limitations; the three
// forecast types are epic completion, quota exhaustion, stall risk.

const REPO = "developing-today/code";
const EPIC = `${REPO}#100`;
const W = [
  "2026-09-07T00:00:00Z",
  "2026-09-14T00:00:00Z",
  "2026-09-21T00:00:00Z",
  "2026-09-28T00:00:00Z",
  "2026-10-05T00:00:00Z",
  "2026-10-12T00:00:00Z",
  "2026-10-19T00:00:00Z",
  "2026-10-26T00:00:00Z",
  "2026-11-02T00:00:00Z",
];

function item(number: number, partial: Partial<ItemHistoryItem> = {}): ItemHistoryItem {
  return {
    ref: `${REPO}#${number}`,
    repo: REPO,
    number,
    kind: "issue",
    state: "open",
    status: null,
    effort: null,
    parentRef: EPIC,
    blockedBy: [],
    ...partial,
  };
}

function cutoff(at: string, items: ItemHistoryItem[], complete = true): ItemHistoryCutoff {
  return {
    schemaVersion: 1,
    cutoffAt: at,
    coverage: { source: "projects-pool", fetchedAt: at, complete, unavailable: !complete },
    items,
  };
}

function history(total: number, closesAtWeek: (number | null)[], weeks = W): ItemHistoryCutoff[] {
  return weeks.map((at, week) =>
    cutoff(
      at,
      Array.from({ length: total }, (_, i) =>
        item(i + 1, { state: closesAtWeek[i] !== null && closesAtWeek[i]! <= week ? "closed" : "open" }),
      ),
    ),
  );
}

const epicOf = (cutoffs: ItemHistoryCutoff[], opts?: Parameters<typeof subjectForecastContracts>[2]) =>
  subjectForecastContracts(cutoffs, EPIC, opts).find((c) => c.type === "epic_completion")!;

describe("epic completion contract (#3984 §4)", () => {
  test("high confidence: stable pace, >=12 completions over 8 weeks publishes a range with its evidence", () => {
    // 16 children close, 2 per week across all 8 transitions; 4 remain.
    const closes = Array.from({ length: 20 }, (_, i) => (i < 16 ? Math.floor(i / 2) + 1 : null));
    const c = epicOf(history(20, closes));
    expect(c.contractVersion).toBe(FORECAST_CONTRACT_VERSION);
    expect(c.subjectRef).toBe(EPIC);
    expect(c.scope).toContain(EPIC);
    expect(c.status).toBe("forecast");
    expect(c.confidence).toBe("high");
    expect(c.p50).not.toBeNull();
    expect(c.p80).not.toBeNull();
    expect(c.p50! <= c.p80!).toBe(true);
    expect(c.remainingUnits).toBe(4);
    expect(c.completedUnits).toBe(16);
    expect(c.baselineSource).toBe(EPIC);
    const line = formatForecastSummary(c);
    expect(line).toContain("high confidence");
    expect(line).toContain("16 of 20 completed children");
    // Never a bare date: the range always travels with its evidence.
    expect(line).not.toMatch(/^[A-Z][a-z]{2} \d{1,2}$/);
    expect(line.split("·").length).toBeGreaterThanOrEqual(3);
  });

  test("low confidence: borrowed baseline is named, never silent", () => {
    const closes = Array.from({ length: 6 }, (_, i) => (i === 0 ? 2 : null));
    const c = epicOf(history(6, closes, W.slice(0, 5)), {
      borrowedBaseline: { scopeRef: "portfolio baseline", weeklyCompletions: [1, 1, 1, 1] },
    });
    expect(c.status).toBe("borrowed_baseline");
    expect(c.confidence).toBe("low");
    expect(c.baselineSource).toBe("portfolio baseline");
    expect(c.p50).not.toBeNull();
    expect(c.reasons.map((r) => r.code)).toContain("borrowed_baseline");
    expect(formatForecastSummary(c)).toContain("portfolio baseline");
  });

  test("missing baseline: below the floor with no comparable scope publishes no date", () => {
    const c = epicOf(history(4, [null, null, null, null], W.slice(0, 2)));
    expect(c.status).toBe("not_enough_history");
    expect(c.p50).toBeNull();
    expect(c.p80).toBeNull();
    expect(c.limitations.map((l) => l.text).join(" ")).toContain("no comparable borrowed baseline");
    expect(formatForecastSummary(c)).toContain("Not enough history");
  });

  test("no measurable pace: a full zero-completion window is a state, not a date or a zero", () => {
    const c = epicOf(history(5, [null, null, null, null, null], W.slice(0, 5)));
    expect(c.status).toBe("no_measurable_pace");
    expect(c.p50).toBeNull();
    expect(formatForecastSummary(c)).toContain("No measurable pace");
  });

  test("status disagreement stays visible on the contract", () => {
    const eta = deriveEpicEta({ subjectRef: EPIC, cutoffs: history(4, [null, null, null, null], W.slice(0, 2)) });
    const c = epicForecastContract(eta, { statusDisagreement: "GitHub state open disagrees with Project Status Done" });
    expect(c.reasons.map((r) => r.code)).toContain("status_disagreement");
  });
});

describe("quota exhaustion contract (#3984 §2/§4)", () => {
  const base = {
    poolId: "pool-a",
    provider: "example",
    asOf: "2026-11-09T12:00:00Z",
    unit: "tokens",
    remainingAmount: 500,
    limitAmount: 1000,
    nextReset: "2026-11-10T12:00:00Z",
  };

  test("fresh measured rate publishes an exhaustion interval and its relation to reset", () => {
    const c = quotaForecastContract({ ...base, rate: { amountPerHour: 100, observedAt: "2026-11-09T11:55:00Z" } });
    expect(c.type).toBe("quota_exhaustion");
    expect(c.status).toBe("forecast");
    expect(c.exhaustion?.estimatedAt).toBe("2026-11-09T17:00:00.000Z");
    expect(c.exhaustion?.beforeReset).toBe(true);
    expect(c.limitations.map((l) => l.code)).toContain("single_linear_rate");
    expect(formatForecastSummary(c)).toContain("Quota wall");
  });

  test("an observed rate band widens the interval honestly", () => {
    const c = quotaForecastContract({
      ...base,
      rate: { amountPerHour: 100, observedAt: "2026-11-09T11:55:00Z", intervalPerHour: { low: 50, high: 200 } },
    });
    expect(c.exhaustion?.earliest).toBe("2026-11-09T14:30:00.000Z");
    expect(c.exhaustion?.latest).toBe("2026-11-09T22:00:00.000Z");
    expect(c.limitations.map((l) => l.code)).not.toContain("single_linear_rate");
  });

  test("quota rate unavailable is first-class: no rate in pool units, no date, never safe", () => {
    const c = quotaForecastContract({ ...base, rate: null });
    expect(c.status).toBe("quota_rate_unavailable");
    expect(c.confidence).toBe("unavailable");
    expect(c.exhaustion).toBeNull();
    expect(c.inputCoverage.missing).toContain("measured burn rate");
    const line = formatForecastSummary(c);
    expect(line).toContain("Quota rate unavailable");
    expect(line).not.toContain("safe");
  });

  test("stale source: an old rate observation publishes no date", () => {
    const c = quotaForecastContract({ ...base, rate: { amountPerHour: 100, observedAt: "2026-11-09T09:00:00Z" } });
    expect(c.status).toBe("quota_rate_unavailable");
    expect(c.exhaustion).toBeNull();
    expect(c.reasons.map((r) => r.code)).toContain("stale_source");
  });

  test("zero burn is no_recent_burn, explicitly not evidence of safety", () => {
    const c = quotaForecastContract({ ...base, rate: { amountPerHour: 0, observedAt: "2026-11-09T11:55:00Z" } });
    expect(c.status).toBe("no_recent_burn");
    expect(c.exhaustion).toBeNull();
    expect(formatForecastSummary(c)).toContain("not evidence the pool is safe");
  });

  test("missing limit/reset: the missing fields are named as structured reasons", () => {
    const c = quotaForecastContract({ ...base, limitAmount: null, nextReset: null, rate: null });
    expect(c.remainingAmount).toBe(500);
    expect(c.limitAmount).toBeNull();
    expect(c.inputCoverage.missing).toContain("limit");
    expect(c.inputCoverage.missing).toContain("next reset");
  });
});

describe("stall risk contract (#3984 §3/§4)", () => {
  test("score, threshold, contributing signals, and unknown signals are all exposed", () => {
    const risk: StallRisk = {
      ref: EPIC,
      asOf: "2026-11-02T00:00:00Z",
      modelVersion: "stall-risk/v1",
      score: 50,
      threshold: 60,
      atRisk: false,
      escalatesToNeedsYou: false,
      evidenceCoverage: 0.5,
      signals: [
        { code: "blocked_edge_age", contribution: 30, known: true, observedAt: "2026-10-26T00:00:00Z", detail: "blocked edge unresolved for at least 7 days (lower bound)" },
        { code: "no_meaningful_activity", contribution: 20, known: true, observedAt: "2026-11-02T00:00:00Z", detail: "no meaningful activity for at least 7 days (updatedAt proxy)" },
        { code: "queue_heartbeat", contribution: 0, known: false, observedAt: null, detail: "no queue source" },
        { code: "queue_requeues", contribution: 0, known: false, observedAt: null, detail: "no queue source" },
      ],
    };
    const c = stallForecastContract(risk);
    expect(c.type).toBe("stall_risk");
    expect(c.score).toBe(50);
    expect(c.escalatesToNeedsYou).toBe(false);
    expect(c.reasons.map((r) => r.code)).toEqual(["blocked_edge_age", "no_meaningful_activity"]);
    expect(c.limitations.map((l) => l.code)).toEqual(["signal_unknown", "signal_unknown"]);
    const line = formatForecastSummary(c);
    expect(line).toContain("Stall risk 50 (threshold 60)");
    expect(line).not.toContain("%");
  });

  test("subject contracts bundle the epic and stall planes for the same subject", () => {
    const contracts = subjectForecastContracts(history(5, [null, null, null, null, null], W.slice(0, 5)), EPIC);
    expect(contracts.map((c) => c.type).sort()).toEqual(["epic_completion", "stall_risk"]);
    for (const c of contracts as ForecastContract[]) {
      expect(c.contractVersion).toBe(FORECAST_CONTRACT_VERSION);
      expect(c.scope.length).toBeGreaterThan(0);
      expect(Array.isArray(c.reasons)).toBe(true);
      expect(Array.isArray(c.limitations)).toBe(true);
    }
  });
});

// --- Rendering: Roadmap and Projects consume the same contract -----------

const poolItem = (
  ref: string,
  title: string,
  opts: { kind?: string | null; state?: ProjectPoolItem["state"]; epicRef?: string } = {},
): ProjectPoolItem => ({
  id: `PVTI_${ref}`,
  url: "https://github.com/orgs/developing-today/projects/4",
  projectUrl: "https://github.com/orgs/developing-today/projects/4",
  ref: ref as ProjectPoolItem["ref"],
  repo: ref.split("#")[0],
  number: Number(ref.split("#")[1]),
  issueUrl: `https://github.com/${ref.replace("#", "/issues/")}`,
  title,
  state: opts.state ?? "open",
  contentKind: "issue",
  status: "Todo",
  area: "id",
  horizon: "Quarter",
  kind: opts.kind === undefined ? "Task" : opts.kind,
  ...(opts.epicRef ? { epicRef: opts.epicRef as ProjectPoolItem["epicRef"] } : {}),
});

describe("rendering consumes the shared contract (#3984)", () => {
  const closes = Array.from({ length: 20 }, (_, i) => (i < 16 ? Math.floor(i / 2) + 1 : null));
  const contracts = subjectForecastContracts(history(20, closes), EPIC);
  const byRef = new Map([[EPIC, contracts] as const]);

  test("Projects pool card renders the forecast line for its epic", () => {
    const pool = [poolItem(EPIC, "Identity epic", { kind: "Epic" })];
    const html = projectsBodyHtml(pool, { forecasts: byRef });
    expect(html).toContain("high confidence");
    expect(html).toContain("16 of 20 completed children");
    expect(html).toContain("Stall risk");
  });

  test("Roadmap drill-down renders the same forecast line for the matched epic", () => {
    const row: RoadmapRow = {
      id: 1,
      subject: "grist:doc/roadmap/1",
      title: "Identity",
      area: "id",
      status: "In Progress",
      horizon: "Quarter",
      source: "grist",
      notes: "",
      links: [{ ref: EPIC as RoadmapRow["links"][number]["ref"], relation: "tracks" as const }],
    };
    const pool = [
      poolItem(EPIC, "Identity epic", { kind: "Epic" }),
      poolItem(`${REPO}#101`, "Child one", { state: "closed", epicRef: EPIC }),
    ];
    const rec = reconcileRoadmap([row], pool);
    const drill = buildRoadmapDrill([row], rec, pool);
    const prov = {
      board: "x", roadmap: "x", items: "x", comments: "x", quota: "x", usage: "x",
      attention: "x", orchestration: "x", dns: "x", registry: "x", aa: "x",
    } as Parameters<typeof renderRoadmap>[2];
    const html = renderRoadmap([row], new Map(), prov, "quarter", undefined, undefined, rec, drill, undefined, "", undefined, byRef);
    expect(html).toContain("high confidence");
    expect(html).toContain("16 of 20 completed children");
  });

  test("no forecasts supplied: pages render exactly as before (no forecast block)", () => {
    const pool = [poolItem(EPIC, "Identity epic", { kind: "Epic" })];
    const html = projectsBodyHtml(pool, {});
    expect(html).not.toContain("forecast-line");
  });
});
