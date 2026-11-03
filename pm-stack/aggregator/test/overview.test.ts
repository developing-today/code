import { describe, expect, test } from "bun:test";
import { PAGES, pageByRoute } from "../src/pages.ts";
import { deriveOverview, renderOverview, renderOverviewBody, type OverviewData } from "../src/render.ts";
import type { HealthView } from "../src/health.ts";

// #1134: "/" is the aggregator front door. The overview's numbers are
// derived from the same views the dedicated pages render, so these
// tests pin the derivation against hand-built source data — the
// overview count for a thing must equal the source page's count.

const health: HealthView = {
  status: "ok",
  mode: "fixtures",
  version: "test",
  generatedAt: "2026-10-29T00:00:00Z",
  bootAt: "2026-10-29T00:00:00Z",
  degraded: false,
  services: [],
  feed: { generatedAt: null, fileMtime: null, ageMs: 60_000, sections: [] },
  jobs: {
    orchestration: { state: "unknown" },
    collector: { generatedAt: null, fileMtime: null, ageMs: null },
  },
  pages: [],
  facets: null,
  itemsCoverage: null,
  comments: { pendingMessages: 0, pendingGithub: 0, total: 0 },
};

const data: OverviewData = {
  attention: {
    drew: [],
    orchestrator: [],
    drewExtra: [
      {
        id: "developing-today/code#42",
        kind: "decision",
        title: "Decide the thing",
        source: "opencode-sandbox",
        reason: "needs a ruling",
        since: "2026-10-28T00:00:00Z",
        lastSeenAt: "2026-10-29T00:00:00Z",
      },
      {
        id: "developing-today/code#43",
        kind: "task",
        title: "Approve the other thing",
        source: "opencode-sandbox",
        reason: "needs approval",
        since: "2026-10-28T00:00:00Z",
        lastSeenAt: "2026-10-29T00:00:00Z",
      },
    ],
    watchdog: [],
    orchestratorExtra: [],
    all: [],
    expiredCount: 0,
    tierSource: { present: true, state: "live" },
  },
  pool: [
    {
      id: "PVTI_1",
      url: "https://example.invalid/1",
      projectUrl: "https://example.invalid/project",
      ref: "developing-today/code#1",
      repo: "developing-today/code",
      number: 1,
      issueUrl: "https://example.invalid/1",
      title: "One",
      state: "open",
      contentKind: "issue",
      status: "In Progress",
      area: null,
      horizon: null,
      kind: "Task",
    },
    {
      id: "PVTI_2",
      url: "https://example.invalid/2",
      projectUrl: "https://example.invalid/project",
      ref: "developing-today/code#2",
      repo: "developing-today/code",
      number: 2,
      issueUrl: "https://example.invalid/2",
      title: "Two",
      state: "open",
      contentKind: "issue",
      status: "Done",
      area: null,
      horizon: null,
      kind: "Task",
    },
    {
      id: "PVTI_3",
      url: "https://example.invalid/3",
      projectUrl: "https://example.invalid/project",
      ref: "dezren39/nix#3",
      repo: "dezren39/nix",
      number: 3,
      issueUrl: "https://example.invalid/3",
      title: "Three",
      state: "open",
      contentKind: "issue",
      status: "In Review",
      area: null,
      horizon: null,
      kind: "Task",
    },
  ],
  quota: {
    pools: [
      {
        id: "meta",
        provider: "Meta",
        plan: "High Usage",
        state: "near",
        windows: [{ name: "weekly", unit: "tokens", limit: 100, used: 80, resetsAt: null, percent: 80 }],
      },
    ],
    burn: [],
    choices: { hasHeadroom: [], burnBeforeReset: [], doNotStart: [] },
  },
  health,
  board: [
    { id: "gh:a", title: "Shipped thing", repo: "developing-today/code", source: "github", status: "done", links: [], updatedAt: "2026-10-29T01:00:00Z" },
    { id: "gh:b", title: "Older done thing", repo: "developing-today/code", source: "github", status: "done", links: [], updatedAt: "2026-10-28T01:00:00Z" },
    { id: "gh:c", title: "Moving thing", repo: "dezren39/nix", source: "github", status: "in-progress", links: [] },
  ],
};

describe("overview front door (#1134)", () => {
  test("PAGES registers overview at / and board at /board, routes unique", () => {
    expect(pageByRoute("/")?.id).toBe("overview");
    expect(pageByRoute("/board")?.id).toBe("board");
    const routes = PAGES.map((p) => p.route);
    expect(new Set(routes).size).toBe(routes.length);
  });

  test("needs-you count and top items agree with the attention view", () => {
    const summary = deriveOverview(data);
    expect(summary.needsYou.count).toBe(2);
    expect(summary.needsYou.top.map((t) => t.title)).toEqual(["Decide the thing", "Approve the other thing"]);
  });

  test("project cards aggregate the pool by repo and status", () => {
    const summary = deriveOverview(data);
    expect(summary.projects).toEqual([
      { repo: "developing-today/code", total: 2, inProgress: 1, inReview: 0, done: 1 },
      { repo: "dezren39/nix", total: 1, inProgress: 0, inReview: 1, done: 0 },
    ]);
  });

  test("quota strip reports the tightest window per pool", () => {
    const summary = deriveOverview(data);
    expect(summary.quota).toEqual([{ provider: "Meta", plan: "High Usage", state: "near", worstPercent: 80 }]);
  });

  test("recently-done is recency-ordered; in-progress comes from the board", () => {
    const summary = deriveOverview(data);
    expect(summary.recentDone.map((i) => i.title)).toEqual(["Shipped thing", "Older done thing"]);
    expect(summary.inProgress.map((i) => i.title)).toEqual(["Moving thing"]);
  });

  test("rendered overview carries every section and links the board at /board", () => {
    const html = renderOverview(data, {
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
    });
    expect(html).toContain("<h1>Overview</h1>");
    expect(html).toContain("Needs you (2)");
    expect(html).toContain("Decide the thing");
    expect(html).toContain("developing-today/code");
    expect(html).toContain("80% of tightest window");
    expect(html).toContain("Shipped thing");
    expect(html).toContain('href="/board"');
    expect(html).not.toContain("Cross-repo WIP board</h1>");
  });
});

describe("overview Codex fixes (PR 2424 review)", () => {
  const FUTURE = new Date(Date.now() + 86_400_000).toISOString();

  test("absent tier source: needs-you count is unknown, never a confident zero or list", () => {
    const absent: OverviewData = {
      ...data,
      attention: { ...data.attention, tierSource: { present: false, state: "absent" } },
    };
    const summary = deriveOverview(absent);
    expect(summary.needsYou.count).toBeNull();
    expect(summary.needsYou.tierState).toBe("absent");
    expect(summary.needsYou.top).toEqual([]);
    const html = renderOverview(absent, {
      board: "x", roadmap: "x", items: "x", comments: "x", quota: "x", usage: "x",
      attention: "x", orchestration: "x", dns: "x", registry: "x", aa: "x",
    });
    expect(html).toContain("Needs you (unknown)");
    expect(html).not.toContain("Nothing needs you right now.");
  });

  test("quota strip derives the tightest window from used/limit when percent is absent", () => {
    const noPercent: OverviewData = {
      ...data,
      quota: {
        ...data.quota,
        pools: [
          {
            ...data.quota.pools[0],
            windows: [
              { name: "weekly", unit: "tokens", limit: 100, used: 92, resetsAt: null },
              { name: "daily", unit: "tokens", limit: 100, used: 40, resetsAt: null, percent: 41 },
            ],
          },
        ],
      },
    };
    const summary = deriveOverview(noPercent);
    // 92% comes from used/limit alone: a window without an explicit
    // percent must still count towards the tightest window.
    expect(summary.quota[0]?.worstPercent).toBeCloseTo(92, 5);
  });

  test("stale tier source with an empty last published queue still warns", () => {
    const summary = deriveOverview(data);
    const html = renderOverviewBody({ ...summary, needsYou: { count: 0, tierState: "stale", top: [] } });
    expect(html).toContain("Nothing needs you right now.");
    expect(html).toContain("Tier source stale");
  });

  test("suppressed needs-you items leave the overview count and top list", () => {
    const snooze = {
      id: "da-ov-1",
      clientToken: "ct-ov-1",
      kind: "triage",
      verb: "snooze",
      objectRef: "orch:developing-today/code#42",
      payload: { until: FUTURE },
      author: { kind: "drew", id: "drew", label: "Drew" },
      createdAt: new Date(Date.now() - 60_000).toISOString(),
      state: "new",
    } as never;
    const withActions: OverviewData = {
      ...data,
      actionsByObject: new Map([["orch:developing-today/code#42", [snooze]]]),
    };
    const summary = deriveOverview(withActions);
    expect(summary.needsYou.count).toBe(1);
    expect(summary.needsYou.top.map((t) => t.title)).toEqual(["Approve the other thing"]);
  });

  test("in-flight list holds running work only — blocked entries cannot crowd it out", () => {
    const blocked = Array.from({ length: 5 }, (_, i) => ({
      id: `orch:blocked-${i}`,
      title: `Blocked thing ${i}`,
      repo: "developing-today/code",
      source: "orchestrator" as const,
      status: "in-flight",
      orchStatus: "blocked",
      links: [],
    }));
    const running = {
      id: "orch:running",
      title: "Running thing",
      repo: "developing-today/code",
      source: "orchestrator" as const,
      status: "in-flight",
      orchStatus: "in-progress",
      links: [],
    };
    const summary = deriveOverview({ ...data, board: [...blocked, running] });
    expect(summary.inProgress.map((i) => i.title)).toEqual(["Running thing"]);
  });
});
