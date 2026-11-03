import { afterAll, beforeAll, describe, expect, test } from "bun:test";
import { mapServerSessions } from "../src/sources/opencode.ts";

// A stub OpenCode v2 server with canned wire shapes (probed live
// 2026-10-25) exercising the mapper's classification signals.
const NOW = Date.parse("2026-10-25T15:00:00Z");
const recent = NOW - 5 * 60_000;
const old = NOW - 20 * 3_600_000;

const sessions = [
  {
    id: "ses_active",
    title: "working",
    model: { id: "deepseek-v4.1-flash", providerID: "cline-pass" },
    tokens: { input: 1000, output: 200, reasoning: 50 },
    outcome: null,
    time: { created: recent - 60_000, updated: recent },
    location: { directory: "/work/a" },
  },
  {
    id: "ses_errored",
    title: "walled",
    model: { id: "muse-wonder-1.0", providerID: "meta" },
    tokens: { input: 10, output: 0, reasoning: 0 },
    outcome: "failed",
    time: { created: recent - 120_000, updated: recent },
    location: { directory: "/work/b" },
  },
  {
    id: "ses_question",
    title: "asking",
    model: { id: "gpt-5.6-sol", providerID: "openai" },
    tokens: { input: 5, output: 5, reasoning: 0 },
    outcome: null,
    time: { created: recent - 90_000, updated: recent },
    location: { directory: "/work/c" },
  },
  {
    id: "ses_stale",
    title: "ancient",
    model: { id: "gpt-5.6-sol", providerID: "openai" },
    tokens: { input: 1, output: 1, reasoning: 0 },
    outcome: "succeeded",
    time: { created: old, updated: old },
    location: { directory: "/work/d" },
  },
  {
    // #2286: out-of-range updated stamp — must not earn freshness
    // inclusion and must not throw the whole server away.
    id: "ses_bad_updated",
    title: "bad updated",
    model: { id: "gpt-5.6-sol", providerID: "openai" },
    tokens: { input: 1, output: 1, reasoning: 0 },
    outcome: null,
    time: { created: recent - 10_000, updated: 1e16 },
    location: { directory: "/work/e" },
  },
  {
    // #2286: out-of-range created stamp, fresh updated — included
    // via freshness, with a null createdAt instead of a throw.
    id: "ses_bad_created",
    title: "bad created",
    model: { id: "gpt-5.6-sol", providerID: "openai" },
    tokens: { input: 1, output: 1, reasoning: 0 },
    outcome: null,
    time: { created: 1e16, updated: recent },
    location: { directory: "/work/f" },
  },
  {
    id: "ses_child",
    title: "explore child",
    parentID: "ses_active",
    agent: "explore",
    model: { id: "deepseek-v4-flash", providerID: "opencode-go" },
    tokens: { input: 3, output: 1, reasoning: 0 },
    outcome: "interrupted",
    time: { created: recent - 30_000, updated: recent },
    location: { directory: "/work/a" },
  },
];

const messages: Record<string, unknown[]> = {
  ses_active: [
    {
      id: "m2",
      type: "assistant",
      time: { created: recent },
      finish: "tool-calls",
      content: [{ type: "tool", state: { status: "running", input: { command: "bun test" } } }],
    },
    { id: "m1", type: "user", time: { created: recent - 1000 }, text: "go" },
  ],
  ses_errored: [
    {
      id: "m3",
      type: "idle",
      time: { created: recent },
      outcome: "failed",
      error: { type: "unknown", message: "429 subscription quota exhausted" },
    },
    { id: "m2", type: "assistant", time: { created: recent - 1000 }, finish: "error", content: [] },
    { id: "m1", type: "user", time: { created: recent - 2000 }, text: "go" },
  ],
  ses_question: [{ id: "m1", type: "user", time: { created: recent }, text: "go" }],
  ses_stale: [{ id: "m1", type: "user", time: { created: old }, text: "go" }],
  ses_child: [{ id: "m1", type: "user", time: { created: recent }, text: "explore" }],
  ses_bad_updated: [{ id: "m1", type: "user", time: { created: recent }, text: "go" }],
  ses_bad_created: [{ id: "m1", type: "user", time: { created: 1e16 }, text: "go" }],
};

let server: ReturnType<typeof Bun.serve>;
let baseUrl = "";

beforeAll(() => {
  server = Bun.serve({
    port: 0,
    fetch(req) {
      const url = new URL(req.url);
      if (url.pathname === "/api/session") return Response.json({ data: sessions });
      if (url.pathname === "/api/session/active") return Response.json({ data: { ses_active: {} } });
      if (url.pathname === "/api/permission/request") {
        return Response.json({
          data: [{ sessionID: "ses_question", title: "bash: rm -rf build" }],
        });
      }
      const m = /^\/api\/session\/([^/]+)\/message$/.exec(url.pathname);
      if (m) return Response.json({ data: messages[m[1]] ?? [] });
      const f = /^\/api\/session\/([^/]+)\/form$/.exec(url.pathname);
      if (f) return Response.json({ data: [] });
      return new Response("not found", { status: 404 });
    },
  });
  baseUrl = `http://127.0.0.1:${server.port}`;
});

afterAll(() => server.stop(true));

describe("opencode v2 mapper", () => {
  test("maps signals and applies the inclusion window", async () => {
    const mapped = await mapServerSessions({ baseUrl, password: "x" }, NOW);
    expect(mapped).not.toBeNull();
    const byId = new Map(mapped!.entries.map((e) => [e.sessionId, e]));

    // Stale session (20 h old, not active, no pending permission) is out.
    expect(byId.has("ses_stale")).toBe(false);

    const active = byId.get("ses_active")!;
    expect(active.activeToolCalls).toBe(1);
    expect(active.longRunningCall?.command).toBe("bun test");
    expect(active.lastTurnError).toBeFalsy();
    expect(active.provider).toBe("cline-pass");
    expect(mapped!.tokensBySession.get("ses_active")).toBe(1250);
    expect(mapped!.activeIds.has("ses_active")).toBe(true);

    const errored = byId.get("ses_errored")!;
    expect(errored.lastTurnError).toContain("429 subscription quota");

    const question = byId.get("ses_question")!;
    expect(question.awaitingQuestion).toContain("Permission pending");

    // Sub-session linkage (slice 2): parent, agent, outcome, created.
    const child = byId.get("ses_child")!;
    expect(child.parentId).toBe("ses_active");
    expect(child.agent).toBe("explore");
    expect(child.outcome).toBe("interrupted");
    expect(child.createdAt).toBeTruthy();
  });

  test("out-of-range stamps do not drop the server's sessions (#2286)", async () => {
    const mapped = await mapServerSessions({ baseUrl, password: "x" }, NOW);
    expect(mapped).not.toBeNull();
    const byId = new Map(mapped!.entries.map((e) => [e.sessionId, e]));
    // The good sessions survive.
    expect(byId.has("ses_active")).toBe(true);
    // 1e16 updated earns no freshness inclusion.
    expect(byId.has("ses_bad_updated")).toBe(false);
    // 1e16 created maps with a null stamp, not a RangeError.
    const badCreated = byId.get("ses_bad_created")!;
    expect(badCreated).toBeTruthy();
    expect(badCreated.createdAt ?? null).toBeNull();
    expect(badCreated.lastActivityAt).toBeTruthy();
  });

  test("stamps the entry source when asked", async () => {
    const mapped = await mapServerSessions({ baseUrl, password: "x" }, NOW, { source: "opencode-laptop" });
    expect(mapped!.entries.every((e) => e.source === "opencode-laptop")).toBe(true);
  });

  test("unreachable server fails soft (null)", async () => {
    const mapped = await mapServerSessions({ baseUrl: "http://127.0.0.1:1", password: "x" }, NOW);
    expect(mapped).toBeNull();
  });
});
