import { afterAll, afterEach, beforeAll, describe, expect, test } from "bun:test";
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
let overrides: Record<string, unknown> = {};

beforeAll(() => {
  server = Bun.serve({
    port: 0,
    fetch(req) {
      const url = new URL(req.url);
      if (url.pathname === "/api/session") {
        return Response.json(Object.hasOwn(overrides, "sessions") ? overrides.sessions : { data: sessions });
      }
      if (url.pathname === "/api/session/active") {
        return Response.json(Object.hasOwn(overrides, "active") ? overrides.active : { data: { ses_active: {} } });
      }
      if (url.pathname === "/api/permission/request") {
        return Response.json(
          Object.hasOwn(overrides, "permissions")
            ? overrides.permissions
            : { data: [{ sessionID: "ses_question", title: "bash: rm -rf build" }] },
        );
      }
      const m = /^\/api\/session\/([^/]+)\/message$/.exec(url.pathname);
      if (m) {
        const bySession = overrides.messages as Record<string, unknown> | undefined;
        const response = bySession && Object.hasOwn(bySession, m[1]) ? bySession[m[1]] : { data: messages[m[1]] ?? [] };
        return Response.json(response);
      }
      const f = /^\/api\/session\/([^/]+)\/form$/.exec(url.pathname);
      if (f) {
        const bySession = overrides.forms as Record<string, unknown> | undefined;
        return Response.json(bySession && Object.hasOwn(bySession, f[1]) ? bySession[f[1]] : { data: [] });
      }
      return new Response("not found", { status: 404 });
    },
  });
  baseUrl = `http://127.0.0.1:${server.port}`;
});

afterAll(() => server.stop(true));
afterEach(() => {
  overrides = {};
});

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

  test("in-range future stamps earn no freshness inclusion (#4180)", async () => {
    const future = NOW + 365 * 24 * 3_600_000;
    const withinSkew = NOW + 30_000;
    overrides = {
      sessions: {
        data: [
          { id: "ses_future", title: "from tomorrow", time: { created: recent, updated: future } },
          { id: "ses_skewed", title: "barely ahead", time: { created: recent, updated: withinSkew } },
        ],
      },
      active: { data: {} },
      permissions: { data: {} },
    };

    const mapped = await mapServerSessions({ baseUrl, password: "x" }, NOW);
    expect(mapped).not.toBeNull();
    const byId = new Map(mapped!.entries.map((e) => [e.sessionId, e]));
    // A stamp a year ahead is in Date range but earns no freshness.
    expect(byId.has("ses_future")).toBe(false);
    // A stamp within the 60s skew tolerance is freshness-included
    // and keeps its raw value.
    const skewed = byId.get("ses_skewed")!;
    expect(skewed).toBeTruthy();
    expect(skewed.lastActivityAt).toBe(new Date(withinSkew).toISOString());
  });

  test("future-stamped sessions cannot crowd out recent ones (#4180)", async () => {
    const futureSessions = Array.from({ length: 61 }, (_, i) => ({
      id: `ses_future_${i}`,
      title: `future ${i}`,
      time: { created: recent, updated: NOW + (i + 1) * 3_600_000 },
    }));
    overrides = {
      sessions: {
        data: [
          ...futureSessions,
          { id: "ses_recent", title: "genuinely recent", time: { created: recent, updated: recent } },
        ],
      },
      active: { data: {} },
      permissions: { data: {} },
    };

    const mapped = await mapServerSessions({ baseUrl, password: "x" }, NOW);
    expect(mapped).not.toBeNull();
    const ids = mapped!.entries.map((e) => e.sessionId);
    expect(ids).toContain("ses_recent");
    expect(ids.some((id) => id.startsWith("ses_future_"))).toBe(false);
  });

  test("active/pending-included future sessions map a null lastActivityAt (#4180)", async () => {
    const future = NOW + 365 * 24 * 3_600_000;
    overrides = {
      sessions: {
        data: [{ id: "ses_future_active", title: "future but live", time: { created: recent, updated: future } }],
      },
      active: { data: { ses_future_active: {} } },
      permissions: { data: [] },
    };

    const mapped = await mapServerSessions({ baseUrl, password: "x" }, NOW);
    expect(mapped).not.toBeNull();
    const entry = mapped!.entries.find((e) => e.sessionId === "ses_future_active")!;
    expect(entry).toBeTruthy();
    // Included via the active set, but the raw future value is withheld.
    expect(entry.lastActivityAt ?? null).toBeNull();
  });

  test("stamps the entry source when asked", async () => {
    const mapped = await mapServerSessions({ baseUrl, password: "x" }, NOW, { source: "opencode-laptop" });
    expect(mapped!.entries.every((e) => e.source === "opencode-laptop")).toBe(true);
  });

  test("unreachable server fails soft (null)", async () => {
    const mapped = await mapServerSessions({ baseUrl: "http://127.0.0.1:1", password: "x" }, NOW);
    expect(mapped).toBeNull();
  });

  test("malformed required session list container fails soft", async () => {
    overrides.sessions = { data: {} };

    const mapped = await mapServerSessions({ baseUrl, password: "x" }, NOW);

    expect(mapped).toBeNull();
  });

  test("malformed endpoint contributions and rows do not drop valid sibling sessions", async () => {
    const brokenSession = { id: "ses_broken", title: "bad optional data", time: { created: recent, updated: recent } };
    const validSibling = { ...sessions[0], id: "ses_valid_sibling", title: "valid sibling" };
    overrides = {
      sessions: { data: [brokenSession, null, { title: "missing id", time: { updated: recent } }, validSibling] },
      active: { data: { ses_broken: {}, ses_valid_sibling: {} } },
      permissions: { data: {} },
      forms: {
        ses_broken: { data: {} },
      },
      messages: {
        ses_broken: { data: {} },
        ses_valid_sibling: { data: messages.ses_active },
      },
    };

    const mapped = await mapServerSessions({ baseUrl, password: "x" }, NOW);

    expect(mapped).not.toBeNull();
    expect(mapped!.entries.map((entry) => entry.sessionId).sort()).toEqual(["ses_broken", "ses_valid_sibling"]);
    expect(mapped!.entries.every((entry) => typeof entry.sessionId === "string" && entry.sessionId.length > 0)).toBe(
      true,
    );
    expect(mapped!.tokensBySession.has("undefined")).toBe(false);
    // The null/id-less sessions plus malformed permission/form/message
    // containers are dropped and counted, never silently swallowed (#2543).
    expect(mapped!.droppedElements).toBe(5);
    expect(mapped!.entries.find((entry) => entry.sessionId === "ses_broken")?.messageCount).toBe(0);
    expect(mapped!.entries.find((entry) => entry.sessionId === "ses_valid_sibling")?.messageCount).toBe(2);
  });
});
