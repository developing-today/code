import { describe, expect, test } from "bun:test";
import type { BoardItem, GhItem, OrchestrationSection, ProjectPoolItem } from "../src/model.ts";
import { buildQueueView } from "../src/queue-view.ts";
import { renderQueue, renderQueueBody } from "../src/render.ts";
import { PAGES } from "../src/pages.ts";

const NOW = new Date("2026-11-01T12:00:00Z");
const recent = "2026-10-30T12:00:00Z";

function poolItem(overrides: Partial<ProjectPoolItem> & Pick<ProjectPoolItem, "ref" | "title">): ProjectPoolItem {
  const { ref, title, ...rest } = overrides;
  return {
    id: `pool-${ref}`,
    url: `https://github.com/orgs/developing-today/projects/4?pane=issue&itemId=${ref}`,
    projectUrl: "https://github.com/orgs/developing-today/projects/4",
    ref,
    repo: ref.split("#")[0],
    number: Number(ref.split("#")[1]),
    issueUrl: `https://github.com/${ref.replace("#", "/issues/")}`,
    title,
    state: "open",
    contentKind: "issue",
    status: null,
    area: "PM stack",
    horizon: null,
    kind: "Task",
    ...rest,
  };
}

function boardItem(overrides: Partial<BoardItem> & Pick<BoardItem, "title" | "status">): BoardItem {
  return {
    id: `board:${overrides.title}`,
    repo: "developing-today/code",
    source: "kaneo",
    links: [],
    ...overrides,
  };
}

function section(linedUp: OrchestrationSection["goals"][number]["linedUp"]): OrchestrationSection {
  return {
    version: 1,
    generatedAt: "2026-11-01T11:00:00Z",
    runId: "orch-test",
    provenance: { sources: ["test"] },
    tiers: { needsDrew: [], needsOrchestrator: [] },
    dependencies: [],
    goals: [{ goalId: "g1", goal: "Keep code healthy", momentum: "moving", summary: "", linedUp, progressedSinceLastRun: [], stalled: [], gaps: [] }],
    actions: [],
  };
}

const task = (title: string, status: "queued" | "blocked" | "in-progress", horizon: "today" | "tomorrow" | "next-3-days" = "today", extra: Partial<OrchestrationSection["goals"][number]["linedUp"][number]> = {}) => ({
  title,
  status,
  horizon,
  horizonWhy: `${horizon} pick`,
  ...extra,
});

describe("buildQueueView (#1160)", () => {
  test("reconciles board work with Project status labels and dedupes by issue ref", () => {
    const ref = "developing-today/code#10";
    const view = buildQueueView({
      board: [boardItem({ title: "Working task", status: "in-progress", links: [{ ref, relation: "tracks" }] })],
      pool: [poolItem({ ref, title: "Working task", labels: ["status/in-progress"] })],
      items: new Map(),
      orchestration: null,
      now: NOW,
    });
    expect(view.inProgress).toHaveLength(1);
    expect(view.inProgress[0].ref).toBe(ref);
    expect(view.inProgress[0].area).toBe("PM stack");
  });

  test("includes blockers from Project edges and orchestration with holder and age", () => {
    const ref = "developing-today/code#11";
    const view = buildQueueView({
      board: [],
      pool: [poolItem({ ref, title: "Blocked task", status: "Blocked", blockedBy: ["developing-today/code#12"], updatedAt: recent })],
      items: new Map(),
      orchestration: section([task("Held task", "blocked", "today", {
        ref: "developing-today/openchamber#23",
        blockedBy: [{ task: "Prerequisite", status: "in-progress", workerLabel: "opencode-laptop" }],
      })]),
      now: NOW,
    });
    expect(view.blocked.map((item) => item.title)).toEqual(["Blocked task", "Held task"]);
    expect(view.blocked[0].blockedBy).toEqual(["developing-today/code#12"]);
    expect(view.blocked[0].ageDays).toBe(2);
    expect(view.blocked[1].worker).toBe("opencode-laptop");
  });

  test("shows only seven-day done cards and merged PRs, excluding bookkeeping", () => {
    const view = buildQueueView({
      board: [
        boardItem({ title: "Recent done", status: "done", updatedAt: recent, links: [{ ref: "developing-today/code#20", relation: "tracks" }] }),
        boardItem({ title: "ROADMAP Done table cleanup", status: "done", updatedAt: recent }),
        boardItem({ title: "Old done", status: "done", updatedAt: "2026-10-20T12:00:00Z" }),
      ],
      pool: [poolItem({ ref: "developing-today/code#21", title: "Merged PR", contentKind: "pr", state: "merged", updatedAt: recent })],
      items: new Map<string, GhItem>(),
      orchestration: null,
      now: NOW,
    });
    expect(view.doneRecently.map((item) => item.title)).toEqual(["Recent done", "Merged PR"]);
  });

  test("orders queued tasks by horizon and carries why and blockers to the row", () => {
    const view = buildQueueView({
      board: [],
      pool: [],
      items: new Map(),
      orchestration: section([
        task("Tomorrow", "queued", "tomorrow"),
        task("Today", "queued", "today", { blockedBy: [{ task: "Prerequisite", status: "queued" }] }),
      ]),
      now: NOW,
    });
    expect(view.nextUp.map((item) => item.title)).toEqual(["Today", "Tomorrow"]);
    expect(view.nextUp[0].why).toBe("today pick");
    expect(view.nextUp[0].blockedBy).toEqual(["Prerequisite (queued)"]);
  });

  test("renders all sections and the manifest/API/route pair", () => {
    const view = buildQueueView({ board: [], pool: [], items: new Map(), orchestration: null, now: NOW });
    const page = PAGES.find((item) => item.id === "queue");
    expect(page?.route).toBe("/queue");
    expect(page?.api).toBe("/api/queue.json");
    const body = renderQueueBody(view);
    for (const heading of ["In progress", "Blocked", "Done recently", "Next up"]) expect(body).toContain(heading);
    expect(renderQueue(view, {
      board: "fixtures", roadmap: "fixtures", items: "fixtures", comments: "fixtures", quota: "fixtures",
      usage: "fixtures", attention: "fixtures", orchestration: "fixtures", dns: "fixtures", registry: "fixtures", aa: "fixtures",
    })).toContain('href="/api/queue.json"');
  });
});
