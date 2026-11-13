import { describe, expect, test } from "bun:test";
import type { BoardItem, GhItem, OrchestrationSection, ProjectPoolItem } from "../src/model.ts";
import { buildQueueView, queueInputsStale } from "../src/queue-view.ts";
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
    goals: [
      {
        goalId: "g1",
        goal: "Keep code healthy",
        momentum: "moving",
        summary: "",
        linedUp,
        progressedSinceLastRun: [],
        stalled: [],
        gaps: [],
      },
    ],
    actions: [],
  };
}

const task = (
  title: string,
  status: "queued" | "blocked" | "in-progress",
  horizon: "today" | "tomorrow" | "next-3-days" = "today",
  extra: Partial<OrchestrationSection["goals"][number]["linedUp"][number]> = {},
) => ({
  title,
  status,
  horizon,
  horizonWhy: `${horizon} pick`,
  ...extra,
});

describe("buildQueueView (#1160)", () => {
  test("queue freshness follows source diagnostics, not projection time", () => {
    const fresh = {
      mode: "live",
      facets: { board: { text: "live", state: "live" }, items: { text: "live", state: "live" } },
      feed: { sections: [{ key: "projects", present: true, status: "success", ageMs: 60_000 }] },
      jobs: { orchestration: { state: "live" } },
    } as unknown as Exclude<Parameters<typeof queueInputsStale>[0], null>;
    expect(queueInputsStale(fresh)).toBe(false);
    expect(queueInputsStale(null)).toBe(true);
    expect(queueInputsStale({ ...fresh, mode: "fixtures" })).toBe(false);
    expect(
      queueInputsStale({
        ...fresh,
        facets: { ...fresh.facets!, board: { ...fresh.facets!.board, text: "stale", state: "stale" } },
      }),
    ).toBe(true);
    expect(
      queueInputsStale({
        ...fresh,
        feed: {
          ...fresh.feed,
          sections: [{ key: "projects", present: false, status: "unknown", ageMs: 60_000, behindBy: null, lastSyncAt: null, lagNote: null }],
        },
      }),
    ).toBe(true);
    expect(
      queueInputsStale({
        ...fresh,
        jobs: { ...fresh.jobs, orchestration: { ...fresh.jobs.orchestration, state: "stale" } },
      }),
    ).toBe(true);
  });

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
      pool: [
        poolItem({
          ref,
          title: "Blocked task",
          status: "Blocked",
          blockedBy: ["developing-today/code#12"],
          updatedAt: recent,
        }),
      ],
      items: new Map(),
      orchestration: section([
        task("Held task", "blocked", "today", {
          ref: "developing-today/openchamber#23",
          blockedBy: [{ task: "Prerequisite", status: "in-progress", workerLabel: "opencode-laptop" }],
        }),
      ]),
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
        boardItem({
          title: "Recent done",
          status: "done",
          updatedAt: recent,
          links: [{ ref: "developing-today/code#20", relation: "tracks" }],
        }),
        boardItem({ title: "ROADMAP Done table cleanup", status: "done", updatedAt: recent }),
        boardItem({ title: "Old done", status: "done", updatedAt: "2026-10-20T12:00:00Z" }),
      ],
      pool: [
        poolItem({
          ref: "developing-today/code#21",
          title: "Merged PR",
          contentKind: "pr",
          state: "merged",
          updatedAt: recent,
        }),
      ],
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

  test("same-horizon next-up candidates keep insertion order across sources (#8352)", () => {
    // Oracle: with the dead priority tier removed, the next-up sort is
    // horizonRank then insertion sequence — candidates are collected
    // orchestration first, then the Project pool, and neither source
    // order nor cross-source order is disturbed by title or ref.
    const view = buildQueueView({
      board: [],
      pool: [
        poolItem({ ref: "developing-today/code#61", title: "Alpha pool", status: "queued", horizon: "today" }),
        poolItem({ ref: "developing-today/code#62", title: "Mike pool", status: "queued", horizon: "today" }),
      ],
      items: new Map(),
      orchestration: section([task("Zulu orch", "queued", "today", { ref: "developing-today/code#60" })]),
      now: NOW,
    });
    expect(view.nextUp.map((item) => item.title)).toEqual(["Zulu orch", "Alpha pool", "Mike pool"]);
    expect(view.nextUp.map((item) => item.source)).toEqual(["orchestration", "project-pool", "project-pool"]);
  });

  test("keeps lined-up tasks with invalid refs but omits their URLs", () => {
    const view = buildQueueView({
      board: [],
      pool: [],
      items: new Map(),
      orchestration: section([
        task("Bad queued ref", "queued", "today", { ref: "goal-id/812" }),
        task("Bad blocked ref", "blocked", "today", {
          ref: "developing-today/code#99999999999999999999",
        }),
      ]),
      now: NOW,
    });

    expect(view.nextUp).toHaveLength(1);
    expect(view.nextUp[0]).toMatchObject({ title: "Bad queued ref", ref: "goal-id/812", source: "orchestration" });
    expect(view.nextUp[0].url).toBeUndefined();
    expect(view.blocked).toHaveLength(1);
    expect(view.blocked[0]).toMatchObject({
      title: "Bad blocked ref",
      ref: "developing-today/code#99999999999999999999",
      source: "orchestration",
    });
    expect(view.blocked[0].url).toBeUndefined();
  });

  test("keeps an out-of-pool invalid epic ref unlinked", () => {
    const epicRef = "developing-today/code#99999999999999999999";
    const view = buildQueueView({
      board: [],
      pool: [poolItem({ ref: "developing-today/code#51", title: "Pool task", status: "queued", epicRef })],
      items: new Map(),
      orchestration: null,
      now: NOW,
    });

    expect(view.nextUp).toHaveLength(1);
    expect(view.nextUp[0].url).toBe("https://github.com/developing-today/code/issues/51");
    expect(view.nextUp[0].epic).toEqual({ ref: epicRef, title: epicRef });
    expect(renderQueueBody(view)).toContain(`Epic: ${epicRef}`);
    expect(renderQueueBody(view)).not.toContain(
      `<a href="https://github.com/developing-today/code/issues/99999999999999999999">`,
    );
  });

  test("keeps generated URLs for valid lined-up refs and out-of-pool epic refs", () => {
    const queuedRef = "developing-today/code#90";
    const blockedRef = "developing-today/code#91";
    const epicRef = "developing-today/code#77";
    const view = buildQueueView({
      board: [],
      pool: [poolItem({ ref: "developing-today/code#51", title: "Pool task", status: "queued", epicRef })],
      items: new Map(),
      orchestration: section([
        task("Good queued ref", "queued", "today", { ref: queuedRef }),
        task("Good blocked ref", "blocked", "today", { ref: blockedRef }),
      ]),
      now: NOW,
    });

    expect(view.nextUp.find((item) => item.ref === queuedRef)?.url).toBe(
      "https://github.com/developing-today/code/issues/90",
    );
    expect(view.blocked.find((item) => item.ref === blockedRef)?.url).toBe(
      "https://github.com/developing-today/code/issues/91",
    );
    expect(view.nextUp.find((item) => item.ref === "developing-today/code#51")?.epic).toEqual({
      ref: epicRef,
      title: epicRef,
      url: "https://github.com/developing-today/code/issues/77",
    });
  });

  test("renders all sections and the manifest/API/route pair", () => {
    const view = buildQueueView({ board: [], pool: [], items: new Map(), orchestration: null, now: NOW });
    const page = PAGES.find((item) => item.id === "queue");
    expect(page?.route).toBe("/queue");
    expect(page?.api).toBe("/api/queue.json");
    const body = renderQueueBody(view);
    for (const heading of ["In progress", "Blocked", "Done recently", "Next up"]) expect(body).toContain(heading);
    expect(
      renderQueue(view, {
        board: "fixtures",
        roadmap: "fixtures",
        items: "fixtures",
        comments: "fixtures",
        quota: "fixtures",
        usage: "fixtures",
        attention: "fixtures",
        orchestration: "fixtures",
        dns: "fixtures",
        registry: "fixtures",
        aa: "fixtures",
      }),
    ).toContain('href="/api/queue.json"');
  });
});
