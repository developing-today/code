import { expect, test } from "bun:test";
import { loadConfig } from "../src/config.ts";
import { buildHealth, createProbeCache, type HealthInputs, type ProbeFetch } from "../src/health.ts";
import type { FleetFeed } from "../src/feed.ts";
import { renderHealth } from "../src/render.ts";

const NOW = Date.parse("2026-11-18T00:00:00Z");
const config = loadConfig({ PM_AGG_DEV: "1" });
const feed: FleetFeed = {
  version: 1,
  generatedAt: new Date(NOW).toISOString(),
  quota: { pools: [], burn: [], choices: { hasHeadroom: [], burnBeforeReset: [], doNotStart: [] } },
  attention: [],
  orchestration: {
    version: 1,
    generatedAt: new Date(NOW).toISOString(),
    runId: "slo-render-test",
    provenance: { sources: ["test"] },
    tiers: { needsDrew: [], needsOrchestrator: [] },
    dependencies: [],
    goals: [],
    actions: [],
  },
} as unknown as FleetFeed;

test("health page renders SLO targets, current values, budgets, burn, and unknown state", async () => {
  const fetcher: ProbeFetch = async () => ({ status: 200 });
  const inputs: HealthInputs = {
    mode: "live",
    cfg: config,
    feed,
    feedFileMtimeMs: NOW,
  };
  const view = await buildHealth(inputs, {
    fetcher,
    cache: createProbeCache(),
    now: NOW,
  });
  view.slo = [
    {
      id: "feed-age",
      label: "Feed age",
      target: "p95 ≤ 10 min",
      sourced: true,
      currentValue: 120_000,
      latestBad: false,
      budgetRemainingPercent: 98.5,
      burnRate1h: 0.25,
      state: "ok",
      badMinutes: 1,
      budgetMinutes: 216,
      unknownSamples: 0,
    },
    {
      id: "tunnel-up",
      label: "Tunnel",
      target: "99.9% of minutes",
      sourced: false,
      currentValue: null,
      latestBad: null,
      budgetRemainingPercent: null,
      burnRate1h: null,
      state: "unknown",
      badMinutes: 0,
      budgetMinutes: 43,
      unknownSamples: 0,
    },
  ];

  const html = renderHealth(view, {} as Parameters<typeof renderHealth>[1]);
  expect(html).toContain("SLO error budgets (#9988)");
  expect(html).toContain("Feed age</td><td>p95 ≤ 10 min</td><td>2m old</td><td>98.5%</td><td>0.25</td><td>ok");
  expect(html).toContain("Tunnel</td><td>99.9% of minutes</td><td><span class=\"muted\">unknown / no signal source</span></td>");
  expect(html).toContain("—</td><td>—</td><td>unknown");
});
