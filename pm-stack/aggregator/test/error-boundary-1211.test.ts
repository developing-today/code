import { afterAll, beforeAll, describe, expect, test } from "bun:test";
import { spawn, type Subprocess } from "bun";
import { errorBoundaryResponse, isApiPath, notFoundResponse } from "../src/error-boundary.ts";

// #1211: consistent not-found semantics + the top-level error
// boundary. Unit tests pin the boundary responses (server.ts's
// contexts are fail-soft per facet by construction, mirroring the
// /program route's catches; a throwing handler lands in
// fetchHandler's catch, which answers with errorBoundaryResponse) —
// the boot tests below pin the HTTP behavior end to end.

describe("error-boundary responses (#1211)", () => {
  test("isApiPath separates API routes from page routes", () => {
    expect(isApiPath("/api/board.json")).toBe(true);
    expect(isApiPath("/board")).toBe(false);
    expect(isApiPath("/apiary")).toBe(false);
  });

  test("not found: page routes get styled HTML 404, APIs get JSON 404", async () => {
    const page = notFoundResponse("/no-such-page");
    expect(page.status).toBe(404);
    expect(page.headers.get("content-type")).toContain("text/html");
    expect(await page.text()).toContain("Not found");
    const api = notFoundResponse("/api/no-such");
    expect(api.status).toBe(404);
    expect(api.headers.get("content-type")).toContain("application/json");
    expect(((await api.json()) as { error: string }).error).toBe("not found");
  });

  test("error boundary: page routes get a styled 500, APIs get JSON 500 with the message", async () => {
    const page = errorBoundaryResponse("/board", new Error("boom"));
    expect(page.status).toBe(500);
    expect(page.headers.get("content-type")).toContain("text/html");
    expect(await page.text()).toContain("Something went wrong");
    const api = errorBoundaryResponse("/api/board.json", new Error("boom"));
    expect(api.status).toBe(500);
    expect(((await api.json()) as { error: string }).error).toContain("boom");
  });
});

let proc: Subprocess<"ignore", "pipe", "inherit"> | null = null;
let port = 0;

beforeAll(async () => {
  port = 4300 + Math.floor(Math.random() * 800);
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

describe("consistent not-found semantics over HTTP (#1211)", () => {
  test("/item and /quota/service misses are 404, not 200", async () => {
    const item = await fetch(`http://127.0.0.1:${port}/item/no-such-item`);
    expect(item.status).toBe(404);
    expect(await item.text()).toContain("Not found");
    const svc = await fetch(`http://127.0.0.1:${port}/quota/service/no-such-service`);
    expect(svc.status).toBe(404);
    expect(await svc.text()).toContain("Not found");
  });

  test("unknown page routes are styled HTML 404; unknown APIs are JSON 404", async () => {
    const page = await fetch(`http://127.0.0.1:${port}/no-such-page`);
    expect(page.status).toBe(404);
    expect(page.headers.get("content-type")).toContain("text/html");
    expect(await page.text()).toContain("Not found");
    const api = await fetch(`http://127.0.0.1:${port}/api/no-such-thing`);
    expect(api.status).toBe(404);
    expect(api.headers.get("content-type")).toContain("application/json");
    expect(((await api.json()) as { error: string }).error).toBe("not found");
  });

  test("known pages still render, and the degraded attention banner still renders", async () => {
    for (const path of ["/", "/board", "/roadmap", "/quota", "/program", "/health"]) {
      const res = await fetch(`http://127.0.0.1:${port}${path}`);
      expect(res.status).toBe(200);
    }
    // Fixtures carry no orchestration section: the attention page's
    // degraded form (tier-source-missing banner) is the fail-soft
    // render this issue's acceptance names.
    const full = await (await fetch(`http://127.0.0.1:${port}/attention?full=1`)).text();
    expect(full).toContain("Tier source missing");
  });
});
