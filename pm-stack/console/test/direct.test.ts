// The edge's stand-ins for the laptop console's classes: `fleet gh api`
// argv become GitHub REST calls, the queue token comes from its Worker
// secret, and every other fleet verb is refused as the laptop's.
//
// Oracles: the exact request the stand-in fetch receives (method, URL,
// headers, body), and the exact RunResult the classes see back. For
// what is remembered of GitHub: a stand-in that tags its answers the
// way GitHub does and counts what it was asked; a memory written the
// plain way here (a list, oldest first) that the real one is compared
// with on generated runs; and for when to ask again, the bounds said
// here and held against generated headers.
import { describe, expect, test } from "bun:test";
import { readConfig } from "../src/config.ts";
import {
  askAgainAt,
  directClasses,
  directConfig,
  directRead,
  edgeRun,
  githubMemory,
  type KeptAnswer,
  type Outward,
  QUIET_MOST_MS,
  queueWrites,
  READ_FROM,
} from "../edge/src/direct.ts";
import type { Fetch } from "../src/ops.ts";
import type { EdgeConfig } from "../src/edge-proto.ts";
import { asThePlatformFetch, letters, pick, rng } from "./support.ts";

const cfg = readConfig({ HOME: "/h", PM_CONSOLE_OWNER_EMAILS: "owner@example.com" });
const edgeConfig: EdgeConfig = {
  owners: ["owner@example.com"],
  certNames: [],
  issuers: [],
  repo: "developing-today/code",
  queue: "repo-health-code-pilot",
  androidPackage: "today.developing.pmconsole",
  androidFingerprints: [],
  androidApp: false,
  queueDbUrl: cfg.queueDbUrl,
  queueTokenName: cfg.queueTokenName,
  promptLabel: cfg.promptLabel,
  needsRepos: cfg.needsRepos,
};

function harness() {
  const calls: { url: string; init: { method: string; headers: Record<string, string>; body?: string } }[] = [];
  const answer: { ok: boolean; status: number; body: unknown } = {
    ok: true,
    status: 200,
    body: [{ number: 1 }],
  };
  const fetchImpl = (async (url: string, init: { method: string; headers: Record<string, string>; body?: string }) => {
    calls.push({ url, init });
    return { ok: answer.ok, status: answer.status, json: async () => answer.body };
  }) as unknown as Fetch;
  // As the Worker runtime's fetch is: not to be called as a method.
  const strict = asThePlatformFetch(fetchImpl) as typeof fetchImpl;
  const run = edgeRun(directConfig(edgeConfig), { githubToken: "gh-test", queueToken: "queue-test" }, strict);
  return { calls, answer, fetchImpl: strict, run };
}

describe("the edge's run: GitHub through REST", () => {
  test("a GET with -f pairs becomes a query string, with the token on it", async () => {
    const h = harness();
    const out = await h.run([
      "fleet",
      "gh",
      "api",
      "search/issues",
      "-f",
      "q=repo:x is:open",
      "-f",
      "per_page=100",
      "-f",
      "page=2",
    ]);
    expect(h.calls).toHaveLength(1);
    expect(h.calls[0].url).toBe("https://api.github.com/search/issues?q=repo%3Ax+is%3Aopen&per_page=100&page=2");
    expect(h.calls[0].init.method).toBe("GET");
    expect(h.calls[0].init.headers.authorization).toBe("Bearer gh-test");
    expect(h.calls[0].init.headers["x-github-api-version"]).toBe("2022-11-28");
    // The Workers runtime supplies no User-Agent of its own, and GitHub
    // requires one: the call must carry this program's.
    expect(h.calls[0].init.headers["user-agent"]).toBe("pm-console-edge");
    expect(out).toEqual({ code: 0, stdout: '[{"number":1}]', stderr: "" });
  });

  test("a path that already carries a query keeps it, and -f pairs join it", async () => {
    const h = harness();
    await h.run(["fleet", "gh", "api", "repos/x/issues?labels=a&state=all", "-f", "per_page=20"]);
    expect(h.calls[0].url).toBe("https://api.github.com/repos/x/issues?labels=a&state=all&per_page=20");
  });

  test("a POST with --input sends the stdin JSON as the body", async () => {
    const h = harness();
    await h.run(["fleet", "gh", "api", "-X", "POST", "repos/x/issues/7/comments", "--input", "-"], {
      stdin: JSON.stringify({ body: "hello" }),
    });
    expect(h.calls[0].init.method).toBe("POST");
    expect(h.calls[0].init.body).toBe(JSON.stringify({ body: "hello" }));
  });

  test("a PUT with -f sends the fields as a JSON body", async () => {
    const h = harness();
    await h.run(["fleet", "gh", "api", "-X", "PUT", "repos/x/pulls/3/merge", "-f", "merge_method=merge"]);
    expect(h.calls[0].init.method).toBe("PUT");
    expect(h.calls[0].init.body).toBe(JSON.stringify({ merge_method: "merge" }));
  });

  test("a DELETE sends no body, and an empty answer stays empty", async () => {
    const h = harness();
    h.answer.body = null;
    const out = await h.run(["fleet", "gh", "api", "-X", "DELETE", "repos/x/issues/7/labels/now"]);
    expect(h.calls[0].init.method).toBe("DELETE");
    expect(h.calls[0].init.body).toBeUndefined();
    expect(out).toEqual({ code: 0, stdout: "null", stderr: "" });
  });

  test("a failed call answers non-zero with the body's own words, so already_exists is found", async () => {
    const h = harness();
    h.answer.ok = false;
    h.answer.status = 422;
    h.answer.body = { message: "Validation Failed", errors: [{ code: "already_exists", field: "name" }] };
    const out = await h.run(["fleet", "gh", "api", "-X", "POST", "repos/x/labels", "--input", "-"], {
      stdin: "{}",
    });
    expect(out.code).toBe(1);
    expect(out.stdout).toBe("");
    expect(out.stderr).toContain("already_exists");
  });

  test("without a GitHub token the call is refused before it is made", async () => {
    const h = harness();
    const noToken = edgeRun(directConfig(edgeConfig), { queueToken: "queue-test" }, h.fetchImpl);
    await expect(noToken(["fleet", "gh", "api", "repos/x/pulls"])).rejects.toThrow(/no GitHub token/);
    expect(h.calls).toHaveLength(0);
  });
});

describe("the edge's run: secrets and the laptop's verbs", () => {
  test("the queue token is answered by the name the config gives it", async () => {
    const h = harness();
    const out = await h.run(["fleet", "secrets", "get", "turso.fleet-queue-rw-token"]);
    expect(out).toEqual({ code: 0, stdout: "queue-test\n", stderr: "" });
  });

  test("any other secret name is refused, and the value is never the answer to a wrong name", async () => {
    const h = harness();
    await expect(h.run(["fleet", "secrets", "get", "something.else"])).rejects.toThrow(/no secret named/);
  });

  test("the queue verb, the bridge and systemctl are the laptop's", async () => {
    const h = harness();
    for (const argv of [
      ["fleet", "queue", "enqueue", "packet.json", "--queue", "repo-health-code-pilot"],
      ["fleet", "bridge", "agents"],
      ["systemctl", "--user", "restart", "pm-console.service"],
      ["python3", "scripts/expose.py", "--status"],
    ]) {
      await expect(h.run(argv)).rejects.toThrow(/needs a machine/);
    }
    expect(h.calls).toHaveLength(0);
  });
});

describe("the direct classes", () => {
  test("the config the edge builds carries what the classes need and nothing it cannot use", () => {
    const config = directConfig(edgeConfig);
    expect(config.repo).toBe("developing-today/code");
    expect(config.queue).toBe("repo-health-code-pilot");
    expect(config.queueDbUrl).toBe(edgeConfig.queueDbUrl);
    expect(config.queueTokenName).toBe(edgeConfig.queueTokenName);
    expect(config.promptLabel).toBe(edgeConfig.promptLabel);
    expect(config.needsRepos).toEqual(edgeConfig.needsRepos);
    expect(config.dashboardUrl).toBeNull();
  });

  test("each direct section is read by the class that reads it on the laptop, and nothing else is", async () => {
    const seen: string[] = [];
    const ops = {
      prs: async () => (seen.push("prs"), [] as unknown[]),
      priorities: async () => (seen.push("priorities"), {}),
    };
    const work = { board: async () => (seen.push("board"), {}), asks: async () => (seen.push("asks"), []) };
    for (const section of ["board", "prs", "priorities", "asks"]) {
      await directRead(section, ops as never, work as never);
    }
    expect(seen).toEqual(["board", "prs", "priorities", "asks"]);
    await expect(directRead("sessions", ops as never, work as never)).rejects.toThrow(/not a section the edge reads/);
  });

  test("the classes run unchanged on the stand-ins: the queue store is reached by fetch", async () => {
    const h = harness();
    const { ops } = directClasses(edgeConfig, {
      githubToken: "gh-test",
      queueToken: "queue-test",
      fetchImpl: h.fetchImpl,
    });
    h.answer.body = {
      results: [{ type: "ok", response: { result: { cols: [{ name: "n" }], rows: [[{ value: 3 }]] } } }],
    };
    const rows = await ops.sql("SELECT count(*) AS n FROM tasks WHERE queue = ?", ["repo-health-code-pilot"]);
    expect(rows).toEqual([{ n: 3 }]);
    expect(h.calls[0].url).toBe(`${cfg.queueDbUrl}/v2/pipeline`);
    expect(h.calls[0].init.headers.Authorization).toBe("Bearer queue-test");
  });
});

describe("a local run's stand-ins", () => {
  test("GitHub's API and the queue store are where a local run says, and nowhere else otherwise", async () => {
    const calls: string[] = [];
    const fetchImpl = (async (url: string) => {
      calls.push(url);
      return { ok: true, status: 200, json: async () => [] };
    }) as unknown as Fetch;
    const local = edgeRun(
      directConfig(edgeConfig),
      { githubToken: "gh-test", queueToken: "queue-test", githubApi: "http://127.0.0.1:4898/github/" },
      fetchImpl,
    );
    await local(["fleet", "gh", "api", "repos/developing-today/code/pulls"]);
    expect(calls).toEqual(["http://127.0.0.1:4898/github/repos/developing-today/code/pulls"]);
    const deployed = edgeRun(directConfig(edgeConfig), { githubToken: "gh-test", queueToken: "queue-test" }, fetchImpl);
    await deployed(["fleet", "gh", "api", "repos/developing-today/code/pulls"]);
    expect(calls[1]).toBe("https://api.github.com/repos/developing-today/code/pulls");
    expect(
      directClasses(edgeConfig, {
        githubToken: "g",
        queueToken: "q",
        queueDb: "http://127.0.0.1:4898/queue/",
        fetchImpl,
      }).ops.cfg.queueDbUrl,
    ).toBe("http://127.0.0.1:4898/queue");
    expect(directClasses(edgeConfig, { githubToken: "g", queueToken: "q", fetchImpl }).ops.cfg.queueDbUrl).toBe(
      cfg.queueDbUrl,
    );
  });

  test("GitHub out of reach is a failed call with the reason, as when the binary fails", async () => {
    const down = (async () => {
      throw new Error("connection refused");
    }) as unknown as Fetch;
    const run = edgeRun(directConfig(edgeConfig), { githubToken: "gh-test", queueToken: "queue-test" }, down);
    expect(await run(["fleet", "gh", "api", "repos/developing-today/code/pulls"])).toEqual({
      code: 1,
      stdout: "",
      stderr: "GitHub could not be reached: connection refused",
    });
  });
});

describe("what this side notes of each call it makes", () => {
  const answering = (status: number, body: unknown) =>
    (async () => ({ ok: status >= 200 && status < 300, status, json: async () => body })) as unknown as Fetch;
  const github = async (status: number, body: unknown, argv = ["repos/developing-today/code/pulls"]) => {
    const seen: Outward[] = [];
    const run = edgeRun(
      directConfig(edgeConfig),
      { githubToken: "gh-test", queueToken: "queue-test", seen: (call) => seen.push(call) },
      answering(status, body),
    );
    await run(["fleet", "gh", "api", ...argv]);
    return seen;
  };

  test("a token that may not do the thing and one calling too often are told apart, though GitHub answers both 403", async () => {
    expect(await github(200, [])).toEqual([{ to: "github", write: false, status: 200 }]);
    expect(await github(403, { message: "Resource not accessible by personal access token" })).toEqual([
      { to: "github", write: false, status: 403 },
    ]);
    expect(await github(403, { message: "API rate limit exceeded for user ID 1." })).toEqual([
      { to: "github", write: false, status: 403, limited: true },
    ]);
    expect(
      await github(403, { message: "You have exceeded a secondary rate limit. Please wait a few minutes." }),
    ).toEqual([{ to: "github", write: false, status: 403, limited: true }]);
    expect(await github(429, { message: "Too many requests" })).toEqual([
      { to: "github", write: false, status: 429, limited: true },
    ]);
    // A 404 that happens to speak of limits is not one.
    expect(await github(404, { message: "rate limit" })).toEqual([{ to: "github", write: false, status: 404 }]);
    // Anything but a GET may have changed something.
    expect(await github(403, { message: "no" }, ["-X", "PUT", "repos/developing-today/code/pulls/55/merge"])).toEqual([
      { to: "github", write: true, status: 403 },
    ]);
  });

  test("a request of the queue store is a write only if a statement in it is not a SELECT", async () => {
    const request = (...sql: string[]) =>
      JSON.stringify({
        requests: [...sql.map((q) => ({ type: "execute", stmt: { sql: q, args: [] } })), { type: "close" }],
      });
    expect(queueWrites(request("SELECT 1"))).toBe(false);
    expect(queueWrites(request("  select status FROM tasks", "SELECT 2"))).toBe(false);
    expect(queueWrites(request("SELECT 1", "UPDATE tasks SET priority = 10"))).toBe(true);
    expect(queueWrites(request("INSERT INTO tasks DEFAULT VALUES"))).toBe(true);
    // Not to be fooled by a name that begins the same way.
    expect(queueWrites(request("SELECTED"))).toBe(true);
    // What cannot be read is taken to be one.
    expect(queueWrites(undefined)).toBe(true);
    expect(queueWrites("not json")).toBe(true);
    expect(queueWrites(JSON.stringify({ requests: "no" }))).toBe(true);
    // And that is what is noted of a call made through the classes.
    const seen: Outward[] = [];
    const rows = { ok: true, status: 200, json: async () => ({ results: [] }) };
    const { ops } = directClasses(edgeConfig, {
      githubToken: "g",
      queueToken: "q",
      fetchImpl: (async () => rows) as unknown as Fetch,
      seen: (call) => seen.push(call),
    });
    await ops.reads([{ sql: "SELECT 1", args: [] }]).catch(() => {});
    expect(seen).toEqual([{ to: "queue", write: false, status: 200 }]);
    // The queue store saying "too many" is noted as that, and one that
    // gave no answer as no answer.
    const noted = async (fetchImpl: Fetch) => {
      const calls: Outward[] = [];
      const made = directClasses(edgeConfig, {
        githubToken: "g",
        queueToken: "q",
        fetchImpl,
        seen: (call) => calls.push(call),
      });
      await made.ops.reads([{ sql: "SELECT 1", args: [] }]).catch(() => {});
      return calls;
    };
    expect(await noted((async () => ({ ok: false, status: 429, json: async () => ({}) })) as unknown as Fetch)).toEqual(
      [{ to: "queue", write: false, status: 429, limited: true }],
    );
    expect(
      await noted((async () => {
        throw new Error("no route");
      }) as unknown as Fetch),
    ).toEqual([{ to: "queue", write: false, status: 0 }]);
  });

  test("each section this side reads names where it is read from", () => {
    expect(READ_FROM).toEqual({
      board: ["queue"],
      prs: ["github"],
      priorities: ["github"],
      asks: ["github", "queue"],
    });
  });
});

/** GitHub as it answers a list asked for by its tag. The tag is the
 * number of times the list has changed; a read that carries the list's
 * own tag is "not modified", with no body. A test can have it turn
 * every call away, saying what it likes of when to ask again. */
function tagging(options: { memory?: boolean; start?: number } = {}) {
  const lists: Record<string, { rev: number; body: unknown }> = {};
  const calls: { method: string; url: string; tag: string | null }[] = [];
  const seen: Outward[] = [];
  const state: { limited: Record<string, string> | null; tags: boolean; status: number | null } = {
    limited: null,
    tags: true,
    status: null,
  };
  const clock = { t: options.start ?? Date.UTC(2026, 10, 23, 17, 0, 0) };
  const fetchImpl = (async (url: string, init: { method: string; headers: Record<string, string> }) => {
    calls.push({ method: init.method, url, tag: init.headers["if-none-match"] ?? null });
    if (state.limited) {
      return {
        ok: false,
        status: 403,
        headers: new Headers(state.limited),
        json: async () => ({ message: "API rate limit exceeded for user ID 1." }),
      };
    }
    if (state.status !== null)
      return { ok: false, status: state.status, headers: new Headers(), json: async () => JSON.parse("") };
    if (init.method !== "GET")
      return { ok: true, status: 200, headers: new Headers(), json: async () => ({ done: true }) };
    lists[url] ??= { rev: 1, body: [{ number: 1 }] };
    const list = lists[url];
    const etag = `W/"${list.rev}"`;
    if (state.tags && init.headers["if-none-match"] === etag) {
      return { ok: false, status: 304, headers: new Headers({ etag }), json: async () => JSON.parse("") };
    }
    return { ok: true, status: 200, headers: new Headers(state.tags ? { etag } : {}), json: async () => list.body };
  }) as unknown as Fetch;
  const memory = options.memory === false ? undefined : githubMemory();
  const run = edgeRun(
    directConfig(edgeConfig),
    { githubToken: "gh-test", seen: (call) => seen.push(call), github: memory, now: () => clock.t },
    asThePlatformFetch(fetchImpl) as typeof fetchImpl,
  );
  const api = (...argv: string[]) => run(["fleet", "gh", "api", ...argv]);
  const change = (url: string, body: unknown) => {
    lists[url] = { rev: (lists[url]?.rev ?? 0) + 1, body };
  };
  return { api, calls, seen, state, clock, memory, change };
}

describe("what the edge remembers of GitHub", () => {
  const PULLS = "repos/developing-today/code/pulls?state=open&per_page=60";
  const ISSUES = "repos/developing-today/code/issues?state=open";
  const at = (path: string) => `https://api.github.com/${path}`;

  test("a list asked for again is asked for by its tag, and one that has not changed is answered from what was kept", async () => {
    const g = tagging();
    const first = await g.api(PULLS);
    expect(first).toEqual({ code: 0, stdout: '[{"number":1}]', stderr: "" });
    expect(g.calls).toEqual([{ method: "GET", url: at(PULLS), tag: null }]);
    // Again: by its tag, and GitHub's "not modified" is the same answer to whoever asked.
    expect(await g.api(PULLS)).toEqual(first);
    expect(g.calls[1]).toEqual({ method: "GET", url: at(PULLS), tag: 'W/"1"' });
    expect(g.seen).toEqual([
      { to: "github", write: false, status: 200 },
      { to: "github", write: false, status: 304 },
    ]);
    // The list changes: the old tag gets a full answer, which replaces what was kept.
    g.change(at(PULLS), [{ number: 1 }, { number: 2 }]);
    expect((await g.api(PULLS)).stdout).toBe('[{"number":1},{"number":2}]');
    expect(g.calls[2].tag).toBe('W/"1"');
    expect((await g.api(PULLS)).stdout).toBe('[{"number":1},{"number":2}]');
    expect(g.calls[3].tag).toBe('W/"2"');
    expect(g.seen.map((call) => call.status)).toEqual([200, 304, 200, 304]);
  });

  test("each address has its own answer, and a write is never asked for by a tag", async () => {
    const g = tagging();
    await g.api(PULLS);
    g.change(at(ISSUES), [{ number: 9 }]);
    expect((await g.api(ISSUES)).stdout).toBe('[{"number":9}]');
    expect((await g.api(PULLS)).stdout).toBe('[{"number":1}]');
    expect((await g.api(ISSUES)).stdout).toBe('[{"number":9}]');
    expect(g.calls.map((call) => call.tag)).toEqual([null, null, 'W/"1"', 'W/"2"']);
    // The same address with other things asked of it is another address.
    await g.api(PULLS, "-f", "page=2");
    expect(g.calls[4]).toEqual({ method: "GET", url: `${at(PULLS)}&page=2`, tag: null });
    // Writes: no tag sent, nothing kept from them, and what is kept is not disturbed.
    await g.api("-X", "PUT", "repos/developing-today/code/pulls/55/merge", "-f", "merge_method=merge");
    await g.api("-X", "POST", PULLS, "-f", "title=x");
    expect(g.calls.slice(5).map((call) => [call.method, call.tag])).toEqual([
      ["PUT", null],
      ["POST", null],
    ]);
    await g.api(PULLS);
    expect(g.calls[7].tag).toBe('W/"1"');
    expect(g.memory?.size()).toBe(3);
  });

  test("an answer with no tag is not kept, one that is gone is let go, and with no memory every call is a full one", async () => {
    const g = tagging();
    await g.api(PULLS);
    // GitHub stops tagging: the full answer is used, and the old one is not kept under it.
    g.state.tags = false;
    await g.api(PULLS);
    await g.api(PULLS);
    expect(g.calls.map((call) => call.tag)).toEqual([null, 'W/"1"', null]);
    expect(g.memory?.size()).toBe(0);
    // Gone from GitHub: what was kept under the address goes too.
    g.state.tags = true;
    await g.api(PULLS);
    expect(g.memory?.size()).toBe(1);
    g.state.status = 404;
    expect((await g.api(PULLS)).code).toBe(1);
    expect(g.memory?.size()).toBe(0);
    // A failure that says nothing of the list leaves what was kept alone.
    g.state.status = null;
    await g.api(PULLS);
    g.state.status = 502;
    expect(await g.api(PULLS)).toEqual({ code: 1, stdout: "", stderr: "GitHub answered http 502" });
    expect(g.memory?.size()).toBe(1);
    // "Not modified" for something never kept is not an answer.
    g.state.status = 304;
    expect(await g.api(ISSUES)).toEqual({ code: 1, stdout: "", stderr: "GitHub answered http 304" });
    // No memory given: nothing is asked by a tag, ever.
    const plain = tagging({ memory: false });
    await plain.api(PULLS);
    await plain.api(PULLS);
    expect(plain.calls.map((call) => call.tag)).toEqual([null, null]);
    expect(plain.seen.map((call) => call.status)).toEqual([200, 200]);
  });

  test("turned away for calling too often with a time to ask again: the time is told, said at the front of why, and nothing is asked until it has come", async () => {
    const g = tagging();
    await g.api(PULLS);
    const back = g.clock.t + 40 * 60_000;
    g.state.limited = { "x-ratelimit-remaining": "0", "x-ratelimit-reset": String(back / 1000) };
    const turned = await g.api(PULLS);
    expect(turned).toEqual({
      code: 1,
      stdout: "",
      stderr: `GitHub says to ask again at ${new Date(back).toISOString()}: {"message":"API rate limit exceeded for user ID 1."}`,
    });
    expect(g.seen[1]).toEqual({ to: "github", write: false, status: 403, limited: true, until: back });
    // Until then nothing goes out: not a read, not a write, though GitHub would answer now.
    g.state.limited = null;
    g.clock.t += 39 * 60_000;
    const asked = g.calls.length;
    const read = await g.api(ISSUES);
    const write = await g.api("-X", "PUT", "repos/developing-today/code/pulls/55/merge");
    expect(g.calls.length).toBe(asked);
    for (const held of [read, write]) {
      expect(held.code).toBe(1);
      expect(held.stderr).toStartWith(`GitHub says to ask again at ${new Date(back).toISOString()}: not asked`);
      expect(held.stderr).toMatch(/rate limit/i);
    }
    // Each is recorded as the answer it would have had: not sent, so nothing written.
    expect(g.seen.slice(2)).toEqual([
      { to: "github", write: false, status: 429, limited: true, until: back },
      { to: "github", write: true, status: 429, limited: true, until: back },
    ]);
    // The time comes: it is asked again, and what was kept is still asked for by its tag.
    g.clock.t = back;
    expect((await g.api(PULLS)).code).toBe(0);
    expect(g.calls[asked]).toEqual({ method: "GET", url: at(PULLS), tag: 'W/"1"' });
  });

  test("when to ask again: the seconds GitHub gives, or the turn of the hour when nothing is left of it; never the past, never much over an hour", () => {
    const now = Date.UTC(2026, 10, 23, 17, 0, 0);
    const said = (headers: Record<string, string>) => askAgainAt(new Headers(headers), now);
    expect(said({ "retry-after": "60" })).toBe(now + 60_000);
    expect(said({ "retry-after": "0" })).toBe(now);
    expect(said({ "x-ratelimit-remaining": "0", "x-ratelimit-reset": String(now / 1000 + 1500) })).toBe(
      now + 1_500_000,
    );
    // Both: the seconds are for this caller, now.
    expect(
      said({ "retry-after": "30", "x-ratelimit-remaining": "0", "x-ratelimit-reset": String(now / 1000 + 1500) }),
    ).toBe(now + 30_000);
    // Something left of the hour's allowance: the turn of the hour is not when to ask again.
    expect(said({ "x-ratelimit-remaining": "12", "x-ratelimit-reset": String(now / 1000 + 1500) })).toBeUndefined();
    expect(said({ "x-ratelimit-reset": String(now / 1000 + 1500) })).toBeUndefined();
    expect(said({})).toBeUndefined();
    expect(askAgainAt(undefined, now)).toBeUndefined();
    // A time that cannot be right is cut to a little over the hour; one already past is now.
    expect(said({ "retry-after": "86400" })).toBe(now + QUIET_MOST_MS);
    expect(said({ "x-ratelimit-remaining": "0", "x-ratelimit-reset": String(now / 1000 + 7 * 3600) })).toBe(
      now + QUIET_MOST_MS,
    );
    expect(said({ "x-ratelimit-remaining": "0", "x-ratelimit-reset": String(now / 1000 - 30) })).toBe(now);
    // A date, as the header may also be written, is not read: no time is better than a wrong one.
    expect(said({ "retry-after": "Mon, 23 Nov 2026 18:00:00 GMT" })).toBeUndefined();
  });

  test("property: whatever GitHub's headers say, the time to ask again is none, or between now and a little over an hour on", () => {
    const next = rng(13489);
    let none = 0;
    let some = 0;
    for (let i = 0; i < 6000; i++) {
      const now = Math.floor(next() * 2e12);
      const value = () =>
        pick(next, [
          "0",
          "1",
          "60",
          "3600",
          String(Math.floor(now / 1000) + Math.floor((next() - 0.3) * 10_000)),
          String(Math.floor(next() * 1e13)),
          "-5",
          "1.5",
          "1e9",
          "",
          " 30 ",
          "soon",
          "Infinity",
          "NaN",
          letters(next, "0123456789 -.eE+x", 0, 14),
        ]);
      const headers: Record<string, string> = {};
      for (const name of ["retry-after", "x-ratelimit-remaining", "x-ratelimit-reset"])
        if (next() < 0.7) headers[name] = value();
      const at = askAgainAt(new Headers(headers), now);
      if (at === undefined) none++;
      else {
        some++;
        expect([headers, now, Number.isInteger(at) && at >= now && at <= now + QUIET_MOST_MS]).toEqual([
          headers,
          now,
          true,
        ]);
      }
    }
    expect([none > 500, some > 500]).toEqual([true, true]);
  });

  test("property: what is kept is what a plain list kept the same way would hold, within its room and its count", () => {
    const next = rng(304);
    for (let round = 0; round < 60; round++) {
      const room = 40 + Math.floor(next() * 400);
      const most = 1 + Math.floor(next() * 6);
      const real = githubMemory(room, most);
      // The model: [address, answer] pairs, the one asked for longest ago first.
      let model: [string, KeptAnswer][] = [];
      const cost = (url: string, answer: KeptAnswer) => url.length + answer.etag.length + answer.text.length;
      const urls = Array.from({ length: 2 + Math.floor(next() * 8) }, (_, n) => `https://api.github.com/list/${n}`);
      for (let step = 0; step < 300; step++) {
        const url = pick(next, urls);
        const roll = next();
        if (roll < 0.45) {
          const answer = { etag: `W/"${step}"`, text: letters(next, "abc", 0, pick(next, [5, 30, 120, 500])) };
          real.set(url, answer);
          model = model.filter(([kept]) => kept !== url);
          if (cost(url, answer) <= room) {
            model.push([url, answer]);
            while (model.length > most || model.reduce((sum, [u, a]) => sum + cost(u, a), 0) > room) model.shift();
          }
        } else if (roll < 0.9) {
          const want = model.find(([kept]) => kept === url);
          expect([round, step, real.get(url)]).toEqual([round, step, want?.[1]]);
          if (want) model = [...model.filter(([kept]) => kept !== url), want];
        } else {
          real.delete(url);
          model = model.filter(([kept]) => kept !== url);
        }
        const held = model.reduce((sum, [u, a]) => sum + cost(u, a), 0);
        expect([round, step, real.size(), real.held()]).toEqual([round, step, model.length, held]);
        expect([real.size() <= most, real.held() <= room]).toEqual([true, true]);
      }
      // What is left is exactly the model's, each under its own address.
      for (const url of urls) expect(real.get(url)).toEqual(model.find(([kept]) => kept === url)?.[1]);
    }
  });
});
