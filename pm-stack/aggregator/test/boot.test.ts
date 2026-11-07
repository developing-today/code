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

  test("item JSON exposes ready-for-agent dispatch state and provenance", async () => {
    const id = "kaneo:code/fx0dispatchtrial0000107";
    const res = await fetch(`http://127.0.0.1:${port}/api/item/${encodeURIComponent(id)}.json`);
    expect(res.status).toBe(200);
    const body = (await res.json()) as {
      linkedItems: { ref: string; labels: string[] }[];
      dispatchStates: { ref: string; state: string; provenance: { source: string; observedAt: string } }[];
    };
    expect(body.linkedItems.find((item) => item.ref === "developing-today/code#762")?.labels).toContain("ready-for-agent");
    expect(body.dispatchStates).toHaveLength(1);
    // Fixtures mode is deterministic and never reads the live harness:
    // the canned dispatch-trial item has no work item, so its state is
    // exactly "not picked up" — never an environment-dependent
    // "unavailable".
    expect(body.dispatchStates[0].state).toBe("not picked up");
    expect(body.dispatchStates[0].provenance.source).toBe("ready-for-agent");
    expect(body.dispatchStates[0].provenance.observedAt).toBeTruthy();
  });

  test("unknown item JSON id returns a structured JSON 404", async () => {
    const res = await fetch(`http://127.0.0.1:${port}/api/item/${encodeURIComponent("kaneo:code/no-such-item-4969")}.json`);
    expect(res.status).toBe(404);
    expect(res.headers.get("content-type")).toContain("application/json");
    const body = (await res.json()) as { error: string };
    expect(body.error).toBe("not found");
    const page = await fetch(`http://127.0.0.1:${port}/item/${encodeURIComponent("kaneo:code/no-such-item-4969")}`);
    expect(page.status).toBe(404);
    expect(page.headers.get("content-type")).toContain("text/html");
  });

  test("front door renders the overview, board lives at /board (#1134)", async () => {
    const root = await fetch(`http://127.0.0.1:${port}/`);
    expect(root.status).toBe(200);
    const rootHtml = await root.text();
    expect(rootHtml).toContain("Overview");
    const full = await (await fetch(`http://127.0.0.1:${port}/?full=1`)).text();
    expect(full).toContain("Needs you");
    const board = await fetch(`http://127.0.0.1:${port}/board`);
    expect(board.status).toBe(200);
    expect(await board.text()).toContain("Cross-repo WIP board");
    const api = await (await fetch(`http://127.0.0.1:${port}/api/overview.json`)).json();
    // Fixtures carry no orchestration section, so the tier source is
    // absent and the needs-you count is honestly null (unknown) --
    // never a confident number over a partial queue (PR 2424 review).
    expect(api.needsYou.count).toBeNull();
    expect(api.needsYou.tierState).toBe("absent");
    expect(Array.isArray(api.projects)).toBe(true);
  });

  test("sw.js serves the real worker (#3676; design §6 step 2)", async () => {
    // The kill switch was replaced by the real push worker under the
    // design's §6 fallback (no probe data existed): the eviction job
    // lives on in the worker's activate cache purge + the page-side
    // probe script, so no release serves a /sw.js that neither
    // evicts the stranded worker nor is the real worker.
    const res = await fetch(`http://127.0.0.1:${port}/sw.js`);
    expect(res.status).toBe(200);
    expect(res.headers.get("content-type")).toContain("javascript");
    expect(res.headers.get("cache-control")).toBe("no-cache");
    const body = await res.text();
    expect(body).toContain("skipWaiting");
    expect(body).toContain("addEventListener('push'");
    expect(body).toContain("addEventListener('notificationclick'");
    expect(body).toContain("caches.keys()");
    expect(body).toContain("clients.claim()");
  });

  test("manifest serves the PWA manifest (#3676/#3315)", async () => {
    const res = await fetch(`http://127.0.0.1:${port}/manifest.webmanifest`);
    expect(res.status).toBe(200);
    const body = (await res.json()) as { start_url: string; display: string };
    expect(body.start_url).toBe("/attention");
    expect(body.display).toBe("standalone");
  });

  test("roadmap page renders", async () => {
    const res = await fetch(`http://127.0.0.1:${port}/roadmap`);
    expect(res.status).toBe(200);
    expect(await res.text()).toContain("Roadmap");
  });

  test("quota and attention pages render", async () => {
    const quota = await (await fetch(`http://127.0.0.1:${port}/quota`)).text();
    const attention = await (await fetch(`http://127.0.0.1:${port}/attention`)).text();
    expect(quota.includes("Usage &amp; billing")).toBe(true); // the hub retitle (design 2026-10-25 §1.1)
    expect(attention.includes("Needs you")).toBe(true);
    expect(attention.includes("Needs User")).toBe(true);
    expect(attention.includes("amd-6.raptor-pumpkinseed.ts.net")).toBe(true);
    // #1444: /attention is a Datastar shell — the tier content streams
    // in from the fragment endpoints. The honesty assertions (#1139)
    // now run against the header fragment that carries them.
    expect(attention.includes("/assets/datastar.js")).toBe(true);
    expect(attention.includes("data-on-load=\"@get('/frag/attention/header')\"")).toBe(true);
    const header = await (await fetch(`http://127.0.0.1:${port}/frag/attention/header`)).text();
    // Fixtures carry no orchestration section, so the tier honesty
    // banner renders and the tier counts are "unknown", never 0 (#1139).
    expect(header.includes("Tier source missing")).toBe(true);
    expect(header.includes("Needs User (unknown)")).toBe(true);
    expect(header.includes("Session coverage")).toBe(true);
    // The no-JS full render still carries the same content inline.
    const full = await (await fetch(`http://127.0.0.1:${port}/attention?full=1`)).text();
    expect(full.includes("Tier source missing")).toBe(true);
    expect(full.includes("Needs User (unknown)")).toBe(true);
  });

  test("datastar asset + shell fragments serve (#1444)", async () => {
    const asset = await fetch(`http://127.0.0.1:${port}/assets/datastar.js`);
    expect(asset.status).toBe(200);
    expect(asset.headers.get("content-type")).toContain("javascript");
    expect(await asset.text()).toContain("Datastar");
    const board = await (await fetch(`http://127.0.0.1:${port}/frag/board`)).text();
    expect(board).toContain("datastar-patch-elements");
    expect(board).toContain('id="board-main"');
    const queues = await (await fetch(`http://127.0.0.1:${port}/frag/attention/queues`)).text();
    expect(queues).toContain('id="attn-queues"');
    expect(queues).toContain('id="drew"');
  });

  test("today page + slice-2 JSON endpoints (today, coverage, horizons)", async () => {
    // #1444: /today is a shell; its body streams from /frag/today.
    const today = await (await fetch(`http://127.0.0.1:${port}/frag/today`)).text();
    expect(today.includes("Top objectives")).toBe(true);
    expect(today.includes("Project momentum")).toBe(true);
    expect(today.includes("Next up")).toBe(true);
    expect(today.includes("stale inputs")).toBe(false); // fixture inputs are an intentional static source
    const todayJson = (await (await fetch(`http://127.0.0.1:${port}/api/today.json`)).json()) as {
      objectives: unknown[];
      momentum: unknown[];
      fromOrchestrator: boolean;
    };
    expect(todayJson.objectives.length).toBeGreaterThan(0);
    expect(todayJson.momentum.length).toBeGreaterThan(0);
    // Fixtures carry no orchestration section (#899: "not measured" is
    // the honest fixture state), so the today view derives — labelled.
    expect(todayJson.fromOrchestrator).toBe(false);
    const coverage = (await (await fetch(`http://127.0.0.1:${port}/api/coverage.json`)).json()) as {
      coverage: { source: string; wired: boolean }[];
      expired: number;
    };
    expect(coverage.coverage.find((c) => c.source === "jules")).toBeUndefined(); // source retired (code#5098)
    expect(coverage.coverage.find((c) => c.source === "claude-code")?.wired).toBe(false);
    expect(coverage.expired).toBeGreaterThan(0); // the finished fixture probe
    const week = await (await fetch(`http://127.0.0.1:${port}/roadmap?h=week`)).text();
    expect(week.includes("Next 3 days")).toBe(true);
  });

  test("queue page, fragment, and API expose the same four lists (#1160)", async () => {
    const shell = await (await fetch(`http://127.0.0.1:${port}/queue`)).text();
    expect(shell).toContain("Queue visibility");
    expect(shell).toContain("/frag/queue");
    const full = await (await fetch(`http://127.0.0.1:${port}/queue?full=1`)).text();
    for (const heading of ["In progress", "Blocked", "Done recently", "Next up"]) expect(full).toContain(heading);
    const fragment = await (await fetch(`http://127.0.0.1:${port}/frag/queue`)).text();
    expect(fragment).toContain("datastar-patch-elements");
    expect(fragment).toContain('id="queue-main"');
    const api = (await (await fetch(`http://127.0.0.1:${port}/api/queue.json`)).json()) as {
      inProgress: unknown[];
      blocked: unknown[];
      doneRecently: unknown[];
      nextUp: unknown[];
    };
    for (const key of ["inProgress", "blocked", "doneRecently", "nextUp"] as const)
      expect(Array.isArray(api[key])).toBe(true);
  });

  test("machine-first JSON endpoints", async () => {
    const board = (await (await fetch(`http://127.0.0.1:${port}/api/board.json`)).json()) as {
      board: Record<string, unknown>[];
    };
    expect(board.board.length).toBeGreaterThan(0);
    expect(board.board.every((item) => !("notes" in item))).toBe(true);
    const quota = (await (await fetch(`http://127.0.0.1:${port}/api/quota.json`)).json()) as { pools: unknown[] };
    expect(quota.pools.length).toBeGreaterThan(0);
    const attention = (await (await fetch(`http://127.0.0.1:${port}/api/attention.json`)).json()) as unknown[];
    expect(attention.length).toBeGreaterThan(0);
  });

  test("unknown project page is a styled 404; a known project stays 200 (#1186)", async () => {
    const missing = await fetch(`http://127.0.0.1:${port}/program/nope`);
    expect(missing.status).toBe(404);
    const missingHtml = await missing.text();
    expect(missingHtml).toContain("Not found");
    expect(missingHtml).toContain("no project with id");
    const known = await fetch(`http://127.0.0.1:${port}/program/id`);
    expect(known.status).toBe(200);
    expect(await known.text()).toContain("id platform");
  });

  test("item drill-down resolves a board card", async () => {
    const board = (await (await fetch(`http://127.0.0.1:${port}/api/board.json`)).json()) as {
      board: { id: string }[];
    };
    const id = encodeURIComponent(board.board[0].id);
    const res = await fetch(`http://127.0.0.1:${port}/item/${id}`);
    expect(res.status).toBe(200);
    const html = await res.text();
    expect(html).toContain("Linked issues / PRs");
    expect(html).toContain("Activity stream");
    expect(html).toContain("View these events in the global river");
    expect(html).toContain("Freshness: unknown");
    expect(html).toContain("Known activity gaps");
    expect(html).toContain("Activity source coverage");
    expect(html).not.toContain("Planning notes");
  });

  test("events filters persist in page and machine-twin URLs", async () => {
    const filters = {
      project: "project-a",
      epic: "developing-today/code#3323",
      kind: "issue-comment",
      actor: "drew",
      ref: "developing-today/code#3323",
      q: "coverage gaps",
      limit: "25",
    };
    const query = new URLSearchParams(filters);
    const page = await fetch(`http://127.0.0.1:${port}/events?${query}`);
    expect(page.status).toBe(200);
    const html = await page.text();
    expect(html).toContain('name="project"');
    expect(html).toContain("overflow-x:auto");
    expect(html).toContain("flex-wrap:wrap");
    expect(html).toContain('value="project-a" selected');
    expect(html).toContain('value="developing-today/code#3323" selected');
    expect(html).toContain('name="q" value="coverage gaps"');
    expect(html).toContain('name="actor" value="drew"');

    const apiHref = html.match(/href="([^"]+)">\/api\/events\.json/)?.[1];
    expect(apiHref).toBeDefined();
    const apiUrl = new URL(apiHref!.replaceAll("&amp;", "&"), `http://127.0.0.1:${port}`);
    for (const [key, value] of Object.entries(filters)) expect(apiUrl.searchParams.get(key)).toBe(value);
    const api = (await (await fetch(apiUrl)).json()) as {
      pagination: { snapshotAt: string };
      activity: { freshness: { status: string }; coverage: unknown[] };
    };
    expect(api.pagination.snapshotAt).toBeTruthy();
    expect(api.activity.freshness.status).toBe("unknown");
    expect(Array.isArray(api.activity.coverage)).toBe(true);
  });

  test("events page renders every ref filter as a visible editable input", async () => {
    // Codex P2 on PR #5266: refs beyond the first used to ride along as
    // hidden inputs, so they could not be edited or removed and
    // clearing the visible Ref field still restricted the river.
    const query = new URLSearchParams();
    query.append("ref", "developing-today/code#3323");
    query.append("ref", "developing-today/code#3321");
    const page = await fetch(`http://127.0.0.1:${port}/events?${query}`);
    expect(page.status).toBe(200);
    const html = await page.text();
    expect(html).not.toContain('type="hidden" name="ref"');
    expect(html).toContain('name="ref" value="developing-today/code#3323"');
    expect(html).toContain('name="ref" value="developing-today/code#3321"');
  });
});
