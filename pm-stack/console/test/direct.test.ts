// The edge's stand-ins for the laptop console's classes: `fleet gh api`
// argv become GitHub REST calls, the queue token comes from its Worker
// secret, and every other fleet verb is refused as the laptop's.
//
// Oracles: the exact request the stand-in fetch receives (method, URL,
// headers, body), and the exact RunResult the classes see back.
import { describe, expect, test } from "bun:test";
import { readConfig } from "../src/config.ts";
import { directClasses, directConfig, directRead, edgeRun } from "../edge/src/direct.ts";
import type { Fetch } from "../src/ops.ts";
import type { EdgeConfig } from "../src/edge-proto.ts";

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
  const run = edgeRun(directConfig(edgeConfig), { githubToken: "gh-test", queueToken: "queue-test" }, fetchImpl);
  return { calls, answer, fetchImpl, run };
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
      await expect(h.run(argv)).rejects.toThrow(/needs the laptop/);
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
