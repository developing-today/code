import { describe, expect, test } from "bun:test";
import type { OrchestrationSection, ProjectPoolItem } from "../src/model.ts";
import { todayFromSection } from "../src/derive.ts";
import { derivedProjectState } from "../src/projects.ts";
import { buildProgramLive } from "../src/program.ts";
import { inFlightBoardItems } from "../src/sources/live.ts";
import { assembleWorkItems, workItemForRef, workItemRefSet } from "../src/work-items.ts";

// #1206: one canonical work-item projection backs Today, Board,
// Needs-you, Projects, and Program. One shared fixture section feeds
// every projection; the same item must carry the same id, status,
// rank, and blockers on all of them.

const GENERATED = "2026-11-02T12:00:00Z";
const REF = "developing-today/code#101";
const BLOCKER_REF = "developing-today/code#100";

function fixtureSection(): OrchestrationSection {
  return {
    version: 1,
    generatedAt: GENERATED,
    runId: `orch-${GENERATED}`,
    provenance: { sources: ["test section"] },
    tiers: {
      needsDrew: [
        {
          id: REF,
          kind: "task",
          title: "Ship the thing",
          source: "github",
          reason: "a decision only Drew can make",
          since: GENERATED,
          lastSeenAt: GENERATED,
          ref: REF,
        },
      ],
      needsOrchestrator: [
        {
          id: "gap-1",
          kind: "gap",
          title: "Lonely gap",
          source: "backlog",
          reason: "no owner yet",
          since: GENERATED,
          lastSeenAt: GENERATED,
        },
      ],
    },
    dependencies: [{ task: "Prereq", ref: BLOCKER_REF, status: "in-progress", taskRef: REF }],
    goals: [
      {
        goalId: "goal-a",
        goal: "Goal A",
        momentum: "blocked",
        summary: "one goal",
        linedUp: [
          {
            title: "Ship the thing",
            ref: REF,
            status: "blocked",
            horizon: "today",
            horizonWhy: "due today",
            blockedBy: [{ task: "Prereq", ref: BLOCKER_REF, status: "in-progress" }],
          },
          { title: "Queued thing", status: "queued", horizon: "today", horizonWhy: "next up" },
        ],
        progressedSinceLastRun: [],
        stalled: [],
        gaps: [],
      },
      {
        // The same task lined up under a second goal is ONE item.
        goalId: "goal-b",
        goal: "Goal B",
        momentum: "moving",
        summary: "another goal",
        linedUp: [
          {
            title: "Ship the thing",
            ref: REF,
            status: "in-progress",
            horizon: "tomorrow",
            horizonWhy: "also here",
          },
        ],
        progressedSinceLastRun: [],
        stalled: [],
        gaps: [],
      },
    ],
    actions: [],
  };
}

const poolItem = (ref: string): ProjectPoolItem => ({
  id: `PVTI_${ref}`,
  url: "https://github.com/orgs/developing-today/projects/4",
  projectUrl: "https://github.com/orgs/developing-today/projects/4",
  ref,
  repo: "developing-today/code",
  number: Number(ref.split("#")[1]),
  issueUrl: `https://github.com/${ref.replace("#", "/issues/")}`,
  title: "pool item",
  state: "open",
  contentKind: "issue",
  status: null,
  area: null,
  horizon: null,
  kind: null,
});

describe("canonical work-item projection (#1206)", () => {
  test("assembles one item per work item, tier-merged, canonically ranked", () => {
    const items = assembleWorkItems(fixtureSection());
    const canonical = workItemForRef(items, REF);
    expect(canonical).toBeDefined();
    // Lined up under two goals, claimed by the needs-drew tier: one
    // item, the tier's canonical status, the lined-up status preserved.
    expect(items.filter((i) => i.ref === REF)).toHaveLength(1);
    expect(canonical!.id).toBe("orch:goal-a/ship-the-thing");
    expect(canonical!.status).toBe("needs-drew");
    expect(canonical!.tier).toBe("needs-drew");
    expect(canonical!.linedUpStatus).toBe("blocked");
    expect(canonical!.goal).toBe("Goal A");
    expect(canonical!.goalId).toBe("goal-a");
    expect(canonical!.blockers).toEqual([{ task: "Prereq", ref: BLOCKER_REF, status: "in-progress" }]);
    // Rank: needs-drew item first, then the standalone tier gap, then
    // the merely-queued lined-up task.
    expect(items[0]!.ref).toBe(REF);
    expect(items[1]!.id).toBe("gap-1");
    expect(items[1]!.status).toBe("needs-orchestrator");
    expect(items[2]!.title).toBe("Queued thing");
    expect(items[0]!.rank).toBeLessThan(items[1]!.rank);
    expect(items[1]!.rank).toBeLessThan(items[2]!.rank);
  });

  test("the same item has the same id, status, and blockers on Board, Today, Projects, and Program", () => {
    const section = fixtureSection();
    const canonical = workItemForRef(assembleWorkItems(section), REF)!;

    // Board: the in-flight entry IS the canonical item, re-shaped.
    const board = inFlightBoardItems(section, []);
    const boardItem = board.find((b) => b.links.some((l) => l.ref === REF));
    expect(boardItem).toBeDefined();
    expect(boardItem!.id).toBe(canonical.id);
    expect(boardItem!.orchStatus).toBe(canonical.linedUpStatus);
    expect(boardItem!.goal).toBe(canonical.goal);
    expect(boardItem!.blockedBy).toEqual(["Prereq (in-progress)"]);

    // Today: the objective carries the canonical id and blockers.
    const today = todayFromSection(section);
    const objective = today.objectives.find((o) => o.ref === REF);
    expect(objective).toBeDefined();
    expect(objective!.id).toBe(canonical.id);
    expect(objective!.detail).toContain("blocked by Prereq (in-progress)");

    // Projects: derived state consults the canonical ref set — the
    // item and its blocker both count as in the section.
    expect(derivedProjectState(poolItem(REF), section)).toBe("In Progress");
    expect(derivedProjectState(poolItem(BLOCKER_REF), section)).toBe("In Progress");
    expect(derivedProjectState(poolItem("developing-today/code#999"), section)).toBe("Todo");
    expect(workItemRefSet(section).has(REF)).toBe(true);
    expect(workItemRefSet(section).has(BLOCKER_REF)).toBe(true);

    // Program: the API twin exposes the canonical list verbatim.
    const live = buildProgramLive(new Map(), [], section);
    const programItem = workItemForRef(live.workItems, REF);
    expect(programItem).toEqual(canonical);
  });

  test("keeps Drew-tier precedence when both attention tiers claim one ref", () => {
    const section = fixtureSection();
    section.tiers!.needsOrchestrator!.push({
      ...section.tiers!.needsDrew![0]!,
      reason: "orchestrator follow-up",
    });

    const item = workItemForRef(assembleWorkItems(section), REF)!;
    expect(item.status).toBe("needs-drew");
    expect(item.tier).toBe("needs-drew");
    expect(item.reason).toBe("a decision only Drew can make");
    expect(item.rank).toBe(0);
  });

  test("does not merge distinct unreferenced tier items just because titles match", () => {
    const section = fixtureSection();
    section.tiers!.needsOrchestrator = [
      { ...section.tiers!.needsOrchestrator![0]!, id: "session-a", title: "Review changes" },
      { ...section.tiers!.needsOrchestrator![0]!, id: "session-b", title: "Review changes" },
    ];

    const sameTitle = assembleWorkItems(section).filter((item) => item.title === "Review changes");
    expect(sameTitle.map((item) => item.id)).toEqual(["session-a", "session-b"]);
  });

  test("a later active observation supersedes an earlier done-recently one for the same ref", () => {
    const section = fixtureSection();
    // REF first appears as done-recently under Goal A, then as queued
    // under Goal B (fixture order is the reverse; rebuild explicitly).
    section.goals = [
      {
        goalId: "goal-a",
        goal: "Goal A",
        momentum: "moving",
        summary: "one goal",
        linedUp: [
          { title: "Ship the thing", ref: REF, status: "done-recently", horizon: "today", horizonWhy: "shipped?" },
        ],
        progressedSinceLastRun: [],
        stalled: [],
        gaps: [],
      },
      {
        goalId: "goal-b",
        goal: "Goal B",
        momentum: "moving",
        summary: "another goal",
        linedUp: [{ title: "Ship the thing", ref: REF, status: "queued", horizon: "today", horizonWhy: "reopened" }],
        progressedSinceLastRun: [],
        stalled: [],
        gaps: [],
      },
    ];
    section.tiers = { needsDrew: [], needsOrchestrator: [] };

    const items = assembleWorkItems(section).filter((i) => i.ref === REF);
    expect(items).toHaveLength(1);
    expect(items[0]!.linedUpStatus).toBe("queued");
    expect(items[0]!.status).toBe("queued");
    // The Board must show the revived work, not drop it as completed.
    const board = inFlightBoardItems(section, []);
    expect(board.some((b) => b.links.some((l) => l.ref === REF))).toBe(true);
  });

  test("registers a merged tier item's publisher id so a later tier item with that id merges too", () => {
    const section = fixtureSection();
    const base = section.tiers!.needsDrew![0]!;
    section.tiers = {
      needsDrew: [{ ...base, id: "session-S", ref: REF }],
      needsOrchestrator: [
        {
          ...base,
          id: "session-S",
          ref: undefined,
          title: "Ship the thing (orchestrator view)",
          reason: "orchestrator also sees it",
        },
      ],
    };

    const items = assembleWorkItems(section);
    // Goal A's two lined-up items only: the orchestrator tier item must
    // merge into the canonical REF item via the registered publisher
    // id, not mint a second item carrying the same published identity.
    expect(items).toHaveLength(2);
    const merged = workItemForRef(items, REF)!;
    expect(merged.tier).toBe("needs-drew");
    expect(items.some((item) => item.id === "session-S")).toBe(false);
  });

  test("keeps Today eligibility when a later goal lines the same task up for today", () => {
    const section = fixtureSection();
    section.goals = [
      {
        goalId: "goal-a",
        goal: "Goal A",
        momentum: "moving",
        summary: "one goal",
        linedUp: [
          { title: "Ship the thing", ref: REF, status: "queued", horizon: "tomorrow", horizonWhy: "planned tomorrow" },
        ],
        progressedSinceLastRun: [],
        stalled: [],
        gaps: [],
      },
      {
        goalId: "goal-b",
        goal: "Goal B",
        momentum: "moving",
        summary: "another goal",
        linedUp: [
          {
            title: "Ship the thing",
            ref: REF,
            status: "queued",
            horizon: "today",
            horizonWhy: "pulled forward to today",
          },
        ],
        progressedSinceLastRun: [],
        stalled: [],
        gaps: [],
      },
    ];
    section.tiers = { needsDrew: [], needsOrchestrator: [] };

    const item = workItemForRef(assembleWorkItems(section), REF)!;
    expect(item.horizon).toBe("today");
    // Today must list the item: one goal still schedules it for today.
    const today = todayFromSection(section);
    expect(today.objectives.some((o) => o.ref === REF)).toBe(true);
  });

  test("does not throw on a tier item with a non-string title", () => {
    const section = fixtureSection();
    const bad = section.tiers!.needsOrchestrator![0]! as unknown as { title: unknown; ref?: unknown };
    bad.title = 42;
    delete bad.ref;

    expect(() => assembleWorkItems(section)).not.toThrow();
  });

  test("ignores malformed optional blockers rather than throwing", () => {
    const section = fixtureSection();
    const drew = section.tiers!.needsDrew![0]! as unknown as { blockedBy: unknown };
    drew.blockedBy = {};
    const linedUp = section.goals![0]!.linedUp![0]! as unknown as { blockedBy: unknown };
    linedUp.blockedBy = 42;

    expect(() => assembleWorkItems(section)).not.toThrow();
    expect(workItemForRef(assembleWorkItems(section), REF)!.blockers).toEqual([]);
  });
});
