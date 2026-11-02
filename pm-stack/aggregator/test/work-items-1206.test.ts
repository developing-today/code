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
});
