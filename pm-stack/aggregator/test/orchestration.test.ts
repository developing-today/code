import { describe, expect, test } from "bun:test";
import { isFleetFeed, isOrchestrationSection } from "../src/feed.ts";
import type { OrchestrationSection } from "../src/model.ts";

// #899: the orchestration section rides the fleet feed. The collector
// embeds the orchestrator's output file only when it passes the
// structural check; the aggregator serves it at /api/orchestration.json.

const SECTION: OrchestrationSection = {
  version: 1,
  generatedAt: "2026-10-25T17:20:00.000Z",
  runId: "orch-2026-10-25T17:20:00.000Z",
  provenance: "orchestration-pass run orch-2026-10-25T17:20:00.000Z",
  tiers: {
    needsDrew: [
      {
        id: "developing-today/code#812",
        kind: "decision",
        title: "Decide the public directory feed design",
        source: "backlog",
        reason: "Scope requires an owner design decision only Drew can make.",
        since: "2026-10-25T17:20:00.000Z",
        lastSeenAt: "2026-10-25T17:20:00.000Z",
        ref: "developing-today/code#812",
      },
    ],
    needsOrchestrator: [],
  },
  dependencies: [],
  goals: [
    {
      goalId: null,
      goal: "Keep the four repos healthy",
      momentum: "moving",
      summary: "Sweeps merged 3 PRs since the last run.",
      linedUp: [],
      progressedSinceLastRun: ["PR #888 merged"],
      stalled: [],
      gaps: [],
    },
  ],
  actions: [],
};

describe("orchestration section validation", () => {
  test("accepts a well-formed section", () => {
    expect(isOrchestrationSection(SECTION)).toBe(true);
  });

  test("rejects non-objects and wrong versions", () => {
    expect(isOrchestrationSection(null)).toBe(false);
    expect(isOrchestrationSection("orchestration")).toBe(false);
    expect(isOrchestrationSection({ ...SECTION, version: 2 })).toBe(false);
  });

  test("rejects missing tier arrays or goal list", () => {
    const { tiers: _t, ...noTiers } = SECTION;
    expect(isOrchestrationSection(noTiers)).toBe(false);
    expect(isOrchestrationSection({ ...SECTION, tiers: { needsDrew: [] } })).toBe(false);
    expect(isOrchestrationSection({ ...SECTION, goals: undefined })).toBe(false);
  });

  test("a fleet feed carrying the section still validates as a feed", () => {
    const feed = {
      version: 1,
      generatedAt: "2026-10-25T17:25:00.000Z",
      quota: { pools: [], burn: [], choices: { hasHeadroom: [], burnBeforeReset: [], doNotStart: [] } },
      attention: [],
      orchestration: SECTION,
    };
    expect(isFleetFeed(feed)).toBe(true);
  });
});
