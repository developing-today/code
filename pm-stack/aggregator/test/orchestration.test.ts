import { describe, expect, test } from "bun:test";
import { mkdtemp, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { loadConfig } from "../src/config.ts";
import { todayFromSection } from "../src/derive.ts";
import {
  isFleetFeed,
  isOrchestrationSection,
  normalizeOrchestrationSection,
  orchestrationSectionProblem,
} from "../src/feed.ts";
import type { OrchestrationSection } from "../src/model.ts";
import { liveSource } from "../src/sources/live.ts";

// #899: the orchestration section rides the fleet feed. The collector
// embeds the orchestrator's output file only when it passes the
// structural check; the aggregator serves it at /api/orchestration.json.

const SECTION: OrchestrationSection = {
  version: 1,
  generatedAt: "2026-10-25T17:20:00.000Z",
  runId: "orch-2026-10-25T17:20:00.000Z",
  provenance: {
    sources: ["fleet-feed.json, keeper state, repo backlogs (orchestration-pass run)"],
    runId: "orch-2026-10-25T17:20:00.000Z",
  },
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
      goals: [{ ...SECTION.goals[0]!, progressedSinceLastRun: "PR #888 merged" }],
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

// #1110 regression: the publisher's actual shape. This fixture mirrors
// the orchestration.json the orchestration-pass cron really published
// (run orch-2026-10-27T03:14:56Z, trimmed to one entry per list) —
// most importantly `provenance` as an OBJECT { sources, runId }, the
// form the string-only validator rejected fleet-wide for a day. If the
// publisher's shape drifts again, this test must fail loudly.
const LIVE_SHAPE = {
  version: 1,
  generatedAt: "2026-10-27T03:14:56Z",
  runId: "orch-2026-10-27T03:14:56Z",
  provenance: {
    sources: [
      "fleet-feed.json 2026-10-27T03:08:15Z (attention: 1 Jules IN_PROGRESS session for code #724, no awaitingQuestion; quota pools healthy)",
      "laptop attention via laptop-ssh SUCCEEDED this run: 6 entries, all needs-poke empty sessions",
      "jules-review-state.json keeper last_run 2026-10-26T23:03:16Z",
    ],
    runId: "orch-2026-10-27T03:14:56Z",
  },
  tiers: {
    needsDrew: [
      {
        id: "developing-today/code#812",
        kind: "decision",
        title: "Decide the public directory feed design",
        source: "backlog",
        reason: "Scope requires an owner design decision only Drew can make.",
        since: "2026-10-27T03:14:56Z",
        lastSeenAt: "2026-10-27T03:14:56Z",
        ref: "developing-today/code#812",
      },
    ],
    needsOrchestrator: [],
  },
  dependencies: [
    {
      task: "code #807 wire exposure (F12.1 apply wiring)",
      ref: "developing-today/code#807",
      status: "queued",
      taskRef: "developing-today/code#818",
    },
  ],
  goals: [
    {
      goalId: "goal_a243772844c3",
      goal: "Keep the four repos healthy",
      momentum: "moving",
      summary: "Strong movement: code #996, #997 and #688 closed via merged PRs.",
      progressedSinceLastRun: ["code #996 closed via merged PR"],
      linedUp: [],
      stalled: [],
      gaps: [],
    },
  ],
  actions: [
    {
      at: "2026-10-27T03:14:56Z",
      kind: "verify",
      target: "fleet queues",
      summary: "Reviewed fleet feed 03:08Z and laptop attention.",
    },
  ],
};

describe("orchestration provenance contract (#1110)", () => {
  test("accepts the section exactly as the orchestrator publishes it", () => {
    expect(isOrchestrationSection(LIVE_SHAPE)).toBe(true);
    expect(orchestrationSectionProblem(LIVE_SHAPE)).toBeNull();
    const normalized = normalizeOrchestrationSection(LIVE_SHAPE);
    expect(normalized?.runId).toBe("orch-2026-10-27T03:14:56Z");
    expect(normalized?.provenance).toEqual(LIVE_SHAPE.provenance);
    expect(normalized?.tiers.needsDrew.length).toBe(1);
    expect(() => todayFromSection(normalized!)).not.toThrow();
  });

  test("normalizes the legacy plain-string provenance to the object form", () => {
    const legacy = { ...LIVE_SHAPE, provenance: "orchestration-pass run orch-2026-10-27T03:14:56Z" };
    expect(isOrchestrationSection(legacy)).toBe(true);
    const normalized = normalizeOrchestrationSection(legacy);
    expect(normalized?.provenance).toEqual({
      sources: ["orchestration-pass run orch-2026-10-27T03:14:56Z"],
    });
  });

  test("rejects malformed provenance forms, naming the field", () => {
    for (const provenance of [42, {}, { sources: "fleet feed" }, { sources: ["ok", 7] }, { sources: [], runId: 5 }]) {
      const bad = { ...LIVE_SHAPE, provenance };
      expect(isOrchestrationSection(bad)).toBe(false);
      expect(orchestrationSectionProblem(bad)).toBe("provenance");
      expect(normalizeOrchestrationSection(bad)).toBeNull();
    }
  });

  test("the problem reporter names the first failing check", () => {
    expect(orchestrationSectionProblem({ ...SECTION, version: 2 })).toBe("version");
    expect(orchestrationSectionProblem({ ...SECTION, tiers: { needsDrew: [] } })).toBe("tiers.needsOrchestrator");
    expect(orchestrationSectionProblem(null)).toBe("not an object");
  });
});

describe("live source orchestration states (#1140)", () => {
  async function sourceWithFeed(extra: Record<string, unknown>) {
    const dir = await mkdtemp(join(tmpdir(), "pm-orch-"));
    const feedFile = join(dir, "fleet-feed.json");
    await writeFile(
      feedFile,
      JSON.stringify({
        version: 1,
        generatedAt: new Date().toISOString(),
        quota: { pools: [], burn: [], choices: { hasHeadroom: [], burnBeforeReset: [], doNotStart: [] } },
        attention: [],
        ...extra,
      }),
    );
    const cfg = loadConfig({ PM_AGG_SOURCE: "live", PM_AGG_FEED_FILE: feedFile, PM_PROJECTOR_URL: "off" });
    return liveSource(cfg);
  }

  test("absent: no section in the feed reads as not published", async () => {
    const src = await sourceWithFeed({});
    expect(await src.orchestration()).toBeNull();
    expect(src.provenance().orchestration).toContain("has not published");
  });

  test("invalid: a present-but-malformed section is named, not 'absent'", async () => {
    const src = await sourceWithFeed({ orchestration: { ...LIVE_SHAPE, provenance: 42 } });
    expect(await src.orchestration()).toBeNull();
    expect(src.provenance().orchestration).toContain("invalid");
    expect(src.provenance().orchestration).toContain("provenance");
    expect(src.provenance().orchestration).not.toContain("has not published");
  });

  test("live: the published shape is served with its run id", async () => {
    const src = await sourceWithFeed({ orchestration: LIVE_SHAPE });
    const section = await src.orchestration();
    expect(section?.runId).toBe("orch-2026-10-27T03:14:56Z");
    expect(section?.tiers.needsDrew.length).toBe(1);
    expect(section?.provenance.sources.length).toBe(3);
    expect(src.provenance().orchestration).toContain("live (orchestration section orch-2026-10-27T03:14:56Z");
  });
});
