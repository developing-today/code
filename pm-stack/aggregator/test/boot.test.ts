import { afterAll, beforeAll, describe, expect, test } from "bun:test";
import { spawn, type Subprocess } from "bun";

// Boots the real server on an ephemeral port and verifies the fixture-backed
// app serves all four views and its health/JSON endpoints.
let proc: Subprocess<"ignore", "pipe", "inherit"> | null = null;
let port = 0;

beforeAll(async () => {
  port = 4100 + Math.floor(Math.random() * 1000);
  proc = spawn({
    cmd: ["bun", "run", "src/server.ts"],
    cwd: import.meta.dir + "/..",
    env: { ...Bun.env, PM_AGG_PORT: String(port), PM_AGG_SOURCE: "fixtures" },
    stdout: "pipe",
    stderr: "inherit",
  });
  const deadline = Date.now() + 8000;
  let ready = false;
  while (Date.now() < deadline) {
    try {
      const res = await fetch(`http://127.0.0.1:${port}/api/health`);
      if (res.ok) {
        ready = true;
        break;
      }
    } catch {
      /* not up yet */
    }
    await Bun.sleep(120);
  }
  expect(ready).toBe(true);
});

afterAll(() => {
  proc?.kill();
});

describe("server boots from fixtures", () => {
  test("health reports ok", async () => {
    const res = await fetch(`http://127.0.0.1:${port}/api/health`);
    expect(res.status).toBe(200);
    const body = (await res.json()) as { status: string; source: { board: string } };
    expect(body.status).toBe("ok");
    expect(body.source.board).toBe("fixtures");
  });

  test("board page renders", async () => {
    const res = await fetch(`http://127.0.0.1:${port}/`);
    expect(res.status).toBe(200);
    const html = await res.text();
    expect(html).toContain("Cross-repo WIP board");
  });

  test("roadmap page renders", async () => {
    const res = await fetch(`http://127.0.0.1:${port}/roadmap`);
    expect(res.status).toBe(200);
    expect(await res.text()).toContain("Roadmap");
  });

  test("quota and attention pages render", async () => {
    const quota = await (await fetch(`http://127.0.0.1:${port}/quota`)).text();
    const attention = await (await fetch(`http://127.0.0.1:${port}/attention`)).text();
    expect(quota.includes("Quota + throughput")).toBe(true);
    expect(attention.includes("Needs you")).toBe(true);
    expect(attention.includes("Needs Drew")).toBe(true);
    expect(attention.includes("Session coverage")).toBe(true);
    expect(attention.includes("amd-6.raptor-pumpkinseed.ts.net")).toBe(true);
  });

  test("today page + slice-2 JSON endpoints (today, coverage, horizons)", async () => {
    const today = await (await fetch(`http://127.0.0.1:${port}/today`)).text();
    expect(today.includes("Top objectives")).toBe(true);
    expect(today.includes("Project momentum")).toBe(true);
    const todayJson = (await (await fetch(`http://127.0.0.1:${port}/api/today.json`)).json()) as {
      objectives: unknown[];
      momentum: unknown[];
      fromOrchestrator: boolean;
    };
    expect(todayJson.objectives.length).toBeGreaterThan(0);
    expect(todayJson.momentum.length).toBeGreaterThan(0);
    expect(todayJson.fromOrchestrator).toBe(true); // fixture summary exists
    const coverage = (await (await fetch(`http://127.0.0.1:${port}/api/coverage.json`)).json()) as {
      coverage: { source: string; wired: boolean }[];
      expired: number;
    };
    expect(coverage.coverage.find((c) => c.source === "jules")?.wired).toBe(true);
    expect(coverage.coverage.find((c) => c.source === "claude-code")?.wired).toBe(false);
    expect(coverage.expired).toBeGreaterThan(0); // the finished fixture probe
    const week = await (await fetch(`http://127.0.0.1:${port}/roadmap?h=week`)).text();
    expect(week.includes("Next 3 days")).toBe(true);
  });

  test("machine-first JSON endpoints", async () => {
    const board = (await (await fetch(`http://127.0.0.1:${port}/api/board.json`)).json()) as { board: unknown[] };
    expect(board.board.length).toBeGreaterThan(0);
    const quota = (await (await fetch(`http://127.0.0.1:${port}/api/quota.json`)).json()) as { pools: unknown[] };
    expect(quota.pools.length).toBeGreaterThan(0);
    const attention = (await (await fetch(`http://127.0.0.1:${port}/api/attention.json`)).json()) as unknown[];
    expect(attention.length).toBeGreaterThan(0);
  });

  test("item drill-down resolves a board card", async () => {
    const board = (await (await fetch(`http://127.0.0.1:${port}/api/board.json`)).json()) as {
      board: { id: string }[];
    };
    const id = encodeURIComponent(board.board[0].id);
    const res = await fetch(`http://127.0.0.1:${port}/item/${id}`);
    expect(res.status).toBe(200);
    expect(await res.text()).toContain("Linked issues / PRs");
  });
});
