import { describe, expect, test } from "bun:test";
import { todayFromSection } from "../src/derive.ts";
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

  // #949: the validator is the only gate before todayFromSection
  // dereferences per-goal fields; a goal missing its nested arrays used
  // to pass and crash /today with a TypeError instead of falling back.
  test("rejects a goal missing each nested array", () => {
    const goal = SECTION.goals[0]!;
    for (const field of ["linedUp", "stalled", "gaps"] as const) {
      const malformed = {
        ...SECTION,
        goals: [{ ...goal, [field]: undefined }],
      };
      expect(isOrchestrationSection(malformed)).toBe(false);
    }
  });

  test("rejects a section missing provenance", () => {
    const { provenance: _p, ...noProvenance } = SECTION;
    expect(isOrchestrationSection(noProvenance)).toBe(false);
  });

  test("accepts the single-string form of progressedSinceLastRun", () => {
    const stringForm = {
      ...SECTION,
      goals: [
        { ...SECTION.goals[0]!, progressedSinceLastRun: "PR #888 merged" },
      ],
    };
    expect(isOrchestrationSection(stringForm)).toBe(true);
  });

  test("todayFromSection does not throw on a validator-passing minimal section", () => {
    const minimal: OrchestrationSection = {
      ...SECTION,
      goals: [
        {
          goalId: null,
          goal: "Minimal goal",
          momentum: "moving",
          summary: "",
          linedUp: [],
          progressedSinceLastRun: ["one line of progress"],
          stalled: [],
          gaps: [],
        },
      ],
    };
    expect(isOrchestrationSection(minimal)).toBe(true);
    expect(() => todayFromSection(minimal)).not.toThrow();
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
