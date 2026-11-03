import { describe, expect, test } from "bun:test";
import { mkdtemp, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { loadConfig } from "../src/config.ts";
import type { BoardItem } from "../src/model.ts";
import { liveSource } from "../src/sources/live.ts";
import { renderBoardBody } from "../src/render.ts";
import { applyBoardDonePolicy, isRetiredDoneTableTitle } from "../src/board-policy.ts";

// #1146: the board is a WIP view, not a completion log. Done cards
// older than 7 days drop off entirely, the surviving done renders
// collapsed behind a "Done recently (N)" expander, and Kaneo cards of
// the retired ROADMAP Done-table bookkeeping class are filtered at
// the aggregator.

const DAY = 24 * 60 * 60 * 1000;
const now = Date.now();
const iso = (msAgo: number) => new Date(now - msAgo).toISOString();

const card = (title: string, status: BoardItem["status"], updatedAt?: string): BoardItem => ({
  id: `kaneo:code/${title}`,
  title,
  repo: "developing-today/code",
  source: "kaneo",
  status,
  updatedAt,
  links: [],
});

describe("board done policy (#1146)", () => {
  test("isRetiredDoneTableTitle matches the retired bookkeeping class", () => {
    expect(isRetiredDoneTableTitle("ROADMAP Done table omits merged PRs")).toBe(true);
    expect(isRetiredDoneTableTitle("roadmap  done  table refresh")).toBe(true);
    expect(isRetiredDoneTableTitle("Roadmap planning for Q4")).toBe(false);
    expect(isRetiredDoneTableTitle("Done table manners")).toBe(false);
  });

  test("applyBoardDonePolicy drops old done and the Done-table class, keeps the rest", () => {
    const board = [
      card("Recent finish", "done", iso(1 * DAY)),
      card("Ancient finish", "done", iso(30 * DAY)),
      card("ROADMAP Done table omits merged PRs", "done", iso(1 * DAY)),
      card("Undated finish", "done"),
      card("Live work", "in-progress", iso(30 * DAY)),
      card("Under review", "in-review", iso(30 * DAY)),
    ];
    const out = applyBoardDonePolicy(board, now).map((i) => i.title);
    expect(out).toContain("Recent finish");
    expect(out).toContain("Undated finish"); // age unproven — kept
    expect(out).not.toContain("Ancient finish");
    expect(out).not.toContain("ROADMAP Done table omits merged PRs");
    expect(out).toContain("Live work"); // age-out applies to done only
    expect(out).toContain("Under review");
  });

  test("renderBoardBody renders only recent done, inside a collapsed expander", () => {
    const board = [
      card("Live work", "in-progress", iso(0)),
      card("Recent finish", "done", iso(2 * DAY)),
      card("Ancient finish", "done", iso(30 * DAY)),
      card("ROADMAP Done table omits merged PRs", "done", iso(1 * DAY)),
    ];
    const html = renderBoardBody(board, new Map());
    expect(html).toContain("Recent finish");
    expect(html).not.toContain("Ancient finish");
    expect(html).not.toContain("ROADMAP Done table");
    // Collapsed expander with the surviving count, after the live lanes.
    expect(html).toContain("<details class=\"donestrip\">");
    expect(html).toContain("Done recently (1)");
    expect(html).not.toContain("done (1)</h2>");
    expect(html.indexOf("Live work")).toBeLessThan(html.indexOf("Done recently"));
    // The recent card lives inside the expander, not a full column.
    expect(html.indexOf("Done recently")).toBeLessThan(html.indexOf("Recent finish"));
  });
});

describe("live board: fully filtered is still live (Codex review, PR #2430)", () => {
  test("board() returns the in-flight lane, not fixtures, when every Kaneo card is filtered", async () => {
    const dir = await mkdtemp(join(tmpdir(), "pm-done-filter-"));
    const feedFile = join(dir, "fleet-feed.json");
    const nowIso = new Date().toISOString();
    await writeFile(
      feedFile,
      JSON.stringify({
        version: 1,
        generatedAt: nowIso,
        quota: { pools: [], burn: [], choices: { hasHeadroom: [], burnBeforeReset: [], doNotStart: [] } },
        attention: [],
        orchestration: {
          version: 1,
          generatedAt: nowIso,
          runId: "orch-test",
          provenance: { sources: ["test"], runId: "orch-test" },
          tiers: { needsDrew: [], needsOrchestrator: [] },
          dependencies: [],
          goals: [
            {
              goalId: null,
              goal: "Keep the four repos healthy",
              momentum: "moving",
              summary: "",
              linedUp: [{ title: "Live orchestrator task", status: "in-progress", horizon: "today", horizonWhy: "test" }],
              progressedSinceLastRun: [],
              stalled: [],
              gaps: [],
            },
          ],
          actions: [],
        },
      }),
    );
    const oldDone = new Date(Date.now() - 30 * DAY).toISOString();
    const recent = new Date(Date.now() - 1 * DAY).toISOString();
    const realFetch = globalThis.fetch;
    globalThis.fetch = (async (input: unknown) => {
      const url = String(input);
      const json = (body: unknown, headers: Record<string, string> = {}) =>
        new Response(JSON.stringify(body), { status: 200, headers: { "Content-Type": "application/json", ...headers } });
      if (url.includes("/api/auth/sign-in/email")) return json({}, { "set-cookie": "session=test; Path=/" });
      if (url.includes("/api/workspace")) return json([{ id: "ws1" }]);
      if (url.includes("/api/project?")) return json([{ id: "p1", slug: "code", name: "Code" }]);
      if (url.includes("/api/task/tasks/"))
        return json({
          data: {
            columns: [
              {
                id: "done",
                name: "Done",
                tasks: [
                  { id: "t-old", number: 1, title: "Ancient finish", status: "done", createdAt: oldDone, updatedAt: oldDone },
                  { id: "t-ret", number: 2, title: "ROADMAP Done table omits merged PRs", status: "done", createdAt: recent, updatedAt: recent },
                ],
              },
            ],
          },
          pagination: { totalPages: 1 },
        });
      return new Response("not found", { status: 404 });
    }) as typeof fetch;
    try {
      const cfg = loadConfig({
        ...Bun.env,
        PM_AGG_SOURCE: "live",
        PM_AGG_FEED_FILE: feedFile,
        PM_PROJECTOR_URL: "off",
        KANEO_URL: "http://kaneo.test",
        KANEO_ADMIN_EMAIL: "test@example.com",
        KANEO_ADMIN_PASSWORD: "test-password",
      });
      const src = liveSource(cfg);
      const board = await src.board();
      // Every Kaneo card was filtered (aged-out + retired class), so the
      // board is exactly the orchestrator in-flight lane — never the
      // fixture board a failed fetch would serve.
      expect(board.map((b) => b.title)).toEqual(["Live orchestrator task"]);
      expect(board[0]?.source).toBe("orchestrator");
      expect(src.provenance().board).toContain("live (Kaneo");
    } finally {
      globalThis.fetch = realFetch;
    }
  });

  test("board() serves fixtures when every Kaneo task read fails (503), not a live empty board", async () => {
    const dir = await mkdtemp(join(tmpdir(), "pm-done-filter-"));
    const feedFile = join(dir, "fleet-feed.json");
    await writeFile(
      feedFile,
      JSON.stringify({
        version: 1,
        generatedAt: new Date().toISOString(),
        quota: { pools: [], burn: [], choices: { hasHeadroom: [], burnBeforeReset: [], doNotStart: [] } },
        attention: [],
      }),
    );
    const realFetch = globalThis.fetch;
    globalThis.fetch = (async (input: unknown) => {
      const url = String(input);
      const json = (body: unknown, headers: Record<string, string> = {}) =>
        new Response(JSON.stringify(body), { status: 200, headers: { "Content-Type": "application/json", ...headers } });
      if (url.includes("/api/auth/sign-in/email")) return json({}, { "set-cookie": "session=test; Path=/" });
      if (url.includes("/api/workspace")) return json([{ id: "ws1" }]);
      if (url.includes("/api/project?")) return json([{ id: "p1", slug: "code", name: "Code" }]);
      if (url.includes("/api/task/tasks/")) return new Response("unavailable", { status: 503 });
      return new Response("not found", { status: 404 });
    }) as typeof fetch;
    try {
      const cfg = loadConfig({
        ...Bun.env,
        PM_AGG_SOURCE: "live",
        PM_AGG_FEED_FILE: feedFile,
        PM_PROJECTOR_URL: "off",
        KANEO_URL: "http://kaneo.test",
        KANEO_ADMIN_EMAIL: "test@example.com",
        KANEO_ADMIN_PASSWORD: "test-password",
      });
      const src = liveSource(cfg);
      const board = await src.board();
      expect(src.provenance().board).toContain("fixtures (kaneo unreachable");
      expect(src.provenance().board).toContain("kaneo task reads failed for every project");
      expect(board.length).toBeGreaterThan(0); // the fixture board, not an empty live board
    } finally {
      globalThis.fetch = realFetch;
    }
  });
});
