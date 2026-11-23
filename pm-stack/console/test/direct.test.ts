// The edge's stand-ins for the laptop console's classes: `fleet gh api`
// argv become GitHub REST calls, the queue token comes from its Worker
// secret, and every other fleet verb is refused as the laptop's.
//
// Oracles: the exact request the stand-in fetch receives (method, URL,
// headers, body), and the exact RunResult the classes see back.
import { describe, expect, test } from "bun:test";
import { readConfig } from "../src/config.ts";
import {
  directClasses,
  directConfig,
  directRead,
  edgeRun,
  type Outward,
  queueWrites,
  READ_FROM,
} from "../edge/src/direct.ts";
import type { Fetch } from "../src/ops.ts";
import type { EdgeConfig } from "../src/edge-proto.ts";
import { asThePlatformFetch } from "./support.ts";

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
    const work = {
      board: async () => (seen.push("board"), {}),
      asks: async () => (seen.push("asks"), []),
      censusFromGitHub: async () => (seen.push("census"), {}),
    };
    for (const section of ["board", "prs", "priorities", "asks", "census"]) {
      await directRead(section, ops as never, work as never);
    }
    expect(seen).toEqual(["board", "prs", "priorities", "asks", "census"]);
    await expect(directRead("sessions", ops as never, work as never)).rejects.toThrow(/not a section the edge reads/);
  });

  test("the census is read from GitHub, by label, in the repositories on the list (#13491)", async () => {
    const calls: string[] = [];
    const fetchImpl = (async (url: string) => {
      calls.push(url);
      const q = new URL(url).searchParams.get("q") ?? "";
      const m = /repo:([^ ]+) state:open label:"([^"]+)"/.exec(q);
      const repo = m?.[1] ?? "";
      const label = m?.[2] ?? "";
      const item = (n: number, labels: string[], title: string, body: string) => ({
        number: n,
        title,
        body,
        html_url: `https://github.com/${repo}/issues/${n}`,
        state: "open",
        updated_at: "2026-11-23T10:00:00Z",
        labels: labels.map((name) => ({ name })),
      });
      const items =
        repo === "developing-today/code" && label === "user/review"
          ? [item(10, ["user/review"], "Review the plan", "")]
          : repo === "developing-today/code" && label === "user/escalated"
            ? [
                item(
                  11,
                  ["user/escalated"],
                  "Fix the host",
                  '<!-- hands:v1 kind=physical action="Power on the host." -->',
                ),
              ]
            : [];
      return { ok: true, status: 200, json: async () => ({ items, total_count: items.length }) };
    }) as unknown as Fetch;
    const { ops, work } = directClasses(edgeConfig, {
      githubToken: "gh-test",
      queueToken: "queue-test",
      fetchImpl,
      now: () => Date.parse("2026-11-23T12:00:00Z"),
    });
    const c = await directRead("census", ops, work);
    expect(calls).toHaveLength(8);
    expect(calls[0]).toBe(
      "https://api.github.com/search/issues?q=repo%3Adeveloping-today%2Fcode+state%3Aopen+label%3A%22user%2Freview%22&per_page=100&page=1",
    );
    expect(c).toEqual({
      at: "2026-11-23T12:00:00.000Z",
      complete: true,
      hands: [
        {
          ref: "developing-today/code#11",
          title: "Fix the host",
          url: "https://github.com/developing-today/code/issues/11",
          pr: false,
          updated: "2026-11-23T10:00:0…",
          kind: "physical",
          action: "Power on the host.",
        },
      ],
      reviews: [
        {
          ref: "developing-today/code#10",
          title: "Review the plan",
          url: "https://github.com/developing-today/code/issues/10",
          pr: false,
          updated: "2026-11-23T10:00:0…",
        },
      ],
      other: [],
    });
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
      census: ["github"],
    });
  });
});
