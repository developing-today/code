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
  drawsOn,
  edgeRun,
  ANSWER_PIECE,
  githubMemory,
  type KeptAnswer,
  mapStore,
  type MemoryStore,
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

describe("the edge's run: the bridge through its binding", () => {
  // The bridge Worker, standing in: every call recorded, its answer
  // set by the test. The credential is a fixture, never a real one.
  function bridgeHarness() {
    const calls: { url: string; init: { method: string; headers: Record<string, string>; body?: string } }[] = [];
    const answer: { ok: boolean; status: number; body: unknown } = {
      ok: true,
      status: 201,
      body: { messages: [], fleet: [{ id: 7, agent: "felix-1" }] },
    };
    const bridgeFetch = (async (
      url: string,
      init: { method: string; headers: Record<string, string>; body?: string },
    ) => {
      calls.push({ url: String(url), init });
      return { ok: answer.ok, status: answer.status, json: async () => answer.body };
    }) as unknown as Fetch;
    const seen: Outward[] = [];
    const secrets = {
      githubToken: "gh-test",
      queueToken: "queue-test",
      bridgeToken: "pm-cred-test",
      bridgeFetch,
      seen: (call: Outward) => seen.push(call),
    };
    const run = edgeRun(directConfig(edgeConfig), secrets, bridgeFetch);
    return { calls, answer, run, seen, secrets, bridgeFetch };
  }

  test("a send becomes a POST to the bridge as pm-stack, the text on the body, never in the address", async () => {
    const h = bridgeHarness();
    const out = await h.run(["fleet", "bridge", "send", "--to", "felix-1,group:muse", "--command", "-"], {
      stdin: "do the thing",
    });
    expect(h.calls).toHaveLength(1);
    expect(h.calls[0].url).toBe("https://bridge.internal/fleet/pm-cred-test/send");
    expect(h.calls[0].init.method).toBe("POST");
    expect(JSON.parse(h.calls[0].init.body ?? "{}")).toEqual({
      agent: "pm-stack",
      to: ["felix-1", "group:muse"],
      body: "do the thing",
      kind: "command",
    });
    expect(out.code).toBe(0);
    expect(JSON.parse(out.stdout)).toEqual({ messages: [], fleet: [{ id: 7, agent: "felix-1" }] });
    expect(h.seen).toEqual([{ to: "bridge", write: true, status: 201 }]);
  });

  test("a send without --command is a message; agents and the log are reads of the same binding", async () => {
    const h = bridgeHarness();
    await h.run(["fleet", "bridge", "send", "--to", "all", "-"], { stdin: "hello" });
    expect(JSON.parse(h.calls[0].init.body ?? "{}").kind).toBe("message");
    h.answer.status = 200;
    h.answer.body = { agents: [{ name: "felix-1" }] };
    const agents = await h.run(["fleet", "bridge", "agents"]);
    expect(h.calls[1].url).toBe("https://bridge.internal/fleet/pm-cred-test/agents");
    expect(h.calls[1].init.method).toBe("GET");
    expect(JSON.parse(agents.stdout)).toEqual({ agents: [{ name: "felix-1" }] });
    h.answer.body = { messages: [{ id: 3 }] };
    const log = await h.run(["fleet", "bridge", "log", "--limit", "30", "--order", "desc"]);
    expect(h.calls[2].url).toBe("https://bridge.internal/fleet/pm-cred-test/messages?limit=30&order=desc");
    expect(JSON.parse(log.stdout)).toEqual({ messages: [{ id: 3 }] });
    expect(h.seen.map((c) => [c.to, c.write, c.status])).toEqual([
      ["bridge", true, 201],
      ["bridge", false, 200],
      ["bridge", false, 200],
    ]);
  });

  test("a credential the bridge no longer knows is said plainly, as a failed call", async () => {
    const h = bridgeHarness();
    h.answer.ok = false;
    h.answer.status = 404;
    h.answer.body = null;
    const out = await h.run(["fleet", "bridge", "send", "--to", "felix-1", "-"], { stdin: "hi" });
    expect(out.code).toBe(1);
    expect(out.stderr).toContain("turned the PM credential away (unknown or revoked)");
    expect(h.seen).toEqual([{ to: "bridge", write: true, status: 404 }]);
  });

  test("without the credential, or without the binding, the bridge is still a machine's", async () => {
    const h = bridgeHarness();
    const noToken = edgeRun(directConfig(edgeConfig), { bridgeFetch: h.bridgeFetch }, h.bridgeFetch);
    await expect(noToken(["fleet", "bridge", "agents"])).rejects.toThrow(/needs a machine/);
    const noBinding = edgeRun(directConfig(edgeConfig), { bridgeToken: "pm-cred-test" }, h.bridgeFetch);
    await expect(noBinding(["fleet", "bridge", "agents"])).rejects.toThrow(/needs a machine/);
    // And a bridge verb this side does not carry is a machine's too.
    await expect(h.run(["fleet", "bridge", "inbox", "--agent", "felix-1"])).rejects.toThrow(/needs a machine/);
    expect(h.calls).toHaveLength(0);
  });

  test("the classes send and read through the bridge unchanged", async () => {
    const h = bridgeHarness();
    const { ops } = directClasses(edgeConfig, h.secrets);
    const sent = await ops.send(["agent:felix-1"], "hello there", true);
    expect(sent).toEqual({ to: ["agent:felix-1"], command: true });
    expect(JSON.parse(h.calls[0].init.body ?? "{}")).toEqual({
      agent: "pm-stack",
      to: ["agent:felix-1"],
      body: "hello there",
      kind: "command",
    });
    h.answer.status = 200;
    h.answer.body = { agents: [{ name: "felix-1", groups: [] }] };
    expect(await ops.agents()).toEqual([{ name: "felix-1", groups: [] }]);
    h.answer.body = { messages: [{ id: 1, body: "one" }] };
    expect(await ops.messages(5)).toEqual([{ id: 1, body: "one" }]);
    expect(h.calls[2].url).toBe("https://bridge.internal/fleet/pm-cred-test/messages?limit=5&order=desc");
  });

  test("the unified views are reads of the same binding; a revoke is a machine's", async () => {
    const h = bridgeHarness();
    h.answer.status = 200;
    h.answer.body = { messages: [{ id: 1, surface: "fleet", sender_addr: "agent:felix-1" }] };
    const feed = await h.run([
      "fleet",
      "bridge",
      "pipeline",
      "--kind",
      "command",
      "--sender",
      "felix-1",
      "--unread",
      "--limit",
      "25",
      "--order",
      "desc",
    ]);
    expect(h.calls[0].url).toBe(
      "https://bridge.internal/fleet/pm-cred-test/pipeline?kind=command&sender=felix-1&unread_only=true&limit=25&order=desc",
    );
    expect(JSON.parse(feed.stdout)).toEqual({ messages: [{ id: 1, surface: "fleet", sender_addr: "agent:felix-1" }] });
    await h.run(["fleet", "bridge", "directory"]);
    expect(h.calls[1].url).toBe("https://bridge.internal/fleet/pm-cred-test/directory");
    await h.run(["fleet", "bridge", "credentials"]);
    expect(h.calls[2].url).toBe("https://bridge.internal/fleet/pm-cred-test/credentials");
    expect(h.seen).toEqual([
      { to: "bridge", write: false, status: 200 },
      { to: "bridge", write: false, status: 200 },
      { to: "bridge", write: false, status: 200 },
    ]);
    // Revoking is the owner's act through the owner token: the edge's
    // read-scope credential never carries it, so the verb is refused
    // here and the action is held for a machine instead.
    await expect(h.run(["fleet", "bridge", "revoke", "--agent", "felix-1"])).rejects.toThrow(/needs a machine/);
    await expect(h.run(["fleet", "bridge", "revoke", "--identity", "muse/felix-1"])).rejects.toThrow(/needs a machine/);
    expect(h.calls).toHaveLength(3);
    // The provisioning state list is a read of the same binding
    // (#14376); the mints and the enroll revoke are machines' verbs,
    // refused here like the revoke above.
    h.answer.body = { enroll_tokens: [{ agent: "new-1", state: "pending" }] };
    const listed = await h.run(["fleet", "bridge", "enroll-tokens"]);
    expect(h.calls[3].url).toBe("https://bridge.internal/fleet/pm-cred-test/enroll-tokens");
    expect(JSON.parse(listed.stdout)).toEqual({ enroll_tokens: [{ agent: "new-1", state: "pending" }] });
    await expect(h.run(["fleet", "bridge", "mint-enroll", "--agent", "new-1"])).rejects.toThrow(/needs a machine/);
    await expect(h.run(["fleet", "bridge", "mint-credential", "--agent", "felix-1"])).rejects.toThrow(
      /needs a machine/,
    );
    await expect(h.run(["fleet", "bridge", "revoke-enroll", "--agent", "new-1"])).rejects.toThrow(/needs a machine/);
    expect(h.calls).toHaveLength(4);
  });

  test("the classes read the pipeline and the merged directory through the bridge, and cannot revoke there", async () => {
    // A bridge that answers each of the unified views with its own rows.
    const byPath = (url: string) => {
      if (url.includes("/pipeline")) {
        return {
          messages: [
            {
              surface: "connector",
              id: 1,
              kind: "instruction",
              sender_addr: "user:drew",
              receiver: "claude",
              body: "owner instruction",
            },
            {
              surface: "fleet",
              id: 2,
              kind: "command",
              sender_addr: "agent:felix-1",
              receiver: "agent:archer-1",
              body: "agent command",
            },
            {
              surface: "notes",
              id: 3,
              kind: "note",
              sender_addr: "user:drew",
              receiver: "muse/felix-1",
              body: "a note",
            },
          ],
        };
      }
      if (url.endsWith("/directory")) {
        return {
          agents: [
            {
              surface: "fleet",
              name: "felix-1",
              groups: ["muse"],
              presence: "last_seen",
              last_seen: "2026-11-24T01:00:00Z",
              unread: 2,
            },
            {
              surface: "notes",
              name: "muse/scout-1",
              identity: "muse/scout-1",
              scopes: { can_send: false, can_inbox: false },
              presence: "none",
              last_seen: null,
            },
          ],
        };
      }
      if (url.endsWith("/credentials")) {
        return {
          credentials: [
            {
              agent: "felix-1",
              state: "active",
              scopes: ["inbox", "send"],
              addr: null,
              credentials: 1,
              live: 1,
              created: "2026-11-24T00:00:00Z",
              revoked_at: null,
            },
          ],
        };
      }
      if (url.endsWith("/enroll-tokens")) {
        return {
          enroll_tokens: [
            {
              agent: "new-1",
              kind: "muse",
              groups: ["muse"],
              notes_identity: null,
              created: "2026-11-24T00:00:00Z",
              expires_at: "2026-11-25T00:00:00Z",
              used_at: null,
              revoked_at: null,
              state: "pending",
            },
          ],
        };
      }
      throw new Error(`unexpected bridge fetch ${url}`);
    };
    const bridgeFetch = (async (url: string) => ({
      ok: true,
      status: 200,
      json: async () => byPath(String(url)),
    })) as unknown as Fetch;
    const { ops } = directClasses(edgeConfig, {
      githubToken: "gh-test",
      queueToken: "queue-test",
      bridgeToken: "pm-cred-test",
      bridgeFetch,
    } as never);
    // The pipeline: rows from all three surfaces, real senders kept.
    const feed = (await ops.pipelineFeed(new URLSearchParams({ kind: "command" }))) as { sender_addr: string }[];
    expect(feed.map((m) => m.sender_addr)).toEqual(["user:drew", "agent:felix-1", "user:drew"]);
    // The directory: the fleet row carries its credential state; the
    // notes-only identity is present, presence none, and its revoked
    // tokens read as a revoked credential.
    const directory = (await ops.directory()) as {
      name: string;
      presence: string;
      last_seen: string | null;
      credential: { state: string } | null;
    }[];
    const felix = directory.find((a) => a.name === "felix-1");
    expect(felix?.credential?.state).toBe("active");
    const scout = directory.find((a) => a.name === "muse/scout-1");
    expect(scout?.presence).toBe("none");
    expect(scout?.last_seen).toBeNull();
    expect(scout?.credential?.state).toBe("revoked");
    // And a revoke asked of the edge's classes is a machine's.
    await expect(ops.revokeCredential("fleet", "felix-1")).rejects.toThrow(/needs a machine/);
    // The enroll tokens' state is read through the bridge the same
    // way (#14376), and taking tokens back is a machine's too.
    const enrollTokens = (await ops.enrollTokens()) as { agent: string; state: string }[];
    expect(enrollTokens).toEqual([
      {
        agent: "new-1",
        kind: "muse",
        groups: ["muse"],
        notes_identity: null,
        created: "2026-11-24T00:00:00Z",
        expires_at: "2026-11-25T00:00:00Z",
        used_at: null,
        revoked_at: null,
        state: "pending",
      },
    ]);
    await expect(ops.revokeEnrollTokens("new-1")).rejects.toThrow(/needs a machine/);
    await expect(ops.mintEnrollToken({ agent: "new-2" })).rejects.toThrow(/needs a machine/);
  });

  test("the agents section is read by the class that reads it on the laptop", async () => {
    const ops = { agents: async () => [{ name: "felix-1" }] };
    expect(await directRead("agents", ops as never, {} as never)).toEqual([{ name: "felix-1" }]);
  });

  test("the pipeline section is the unread-instruction count, read the same way (#14377)", async () => {
    const ops = { unreadInstructions: async () => ({ unreadInstructions: 2 }) };
    expect(await directRead("pipeline", ops as never, {} as never)).toEqual({ unreadInstructions: 2 });
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

  test("the queue store has twenty seconds to answer, body and all: stopped then, the read fails saying so, and however it ends no timer is left running", async () => {
    // The timers a call starts, held here instead of run: the test fires the deadline itself.
    const real = { set: globalThis.setTimeout, clear: globalThis.clearTimeout };
    const set: { fire: () => void; ms: number; cleared: boolean }[] = [];
    globalThis.setTimeout = ((fire: () => void, ms: number) => set.push({ fire, ms, cleared: false })) as never;
    globalThis.clearTimeout = ((id: number) => {
      if (set[id - 1]) set[id - 1].cleared = true;
    }) as never;
    const started = async (count: number) => {
      for (let turn = 0; turn < 100 && set.length < count; turn++) await Promise.resolve();
      expect(set.length).toBe(count);
    };
    type Init = { signal: AbortSignal; headers: Record<string, string> };
    const stopped = (signal: AbortSignal) =>
      new Promise<never>((_resolve, reject) => signal.addEventListener("abort", () => reject(signal.reason)));
    const good = { results: [{ type: "ok", response: { result: { cols: [{ name: "n" }], rows: [[{ value: 3 }]] } } }] };
    const classes = (fetchImpl: unknown) =>
      directClasses(edgeConfig, { githubToken: "gh-test", queueToken: "queue-test", fetchImpl: fetchImpl as Fetch })
        .ops;
    const SQL = "SELECT count(*) AS n FROM tasks";
    try {
      // Answered: under a deadline that had not run out, and its timer cleared.
      const fine = classes(
        asThePlatformFetch(async (_url: string, init: Init) => ({
          ok: !init.signal.aborted,
          status: 200,
          json: async () => good,
        })),
      );
      expect(await fine.sql(SQL, [])).toEqual([{ n: 3 }]);
      expect(set.map((timer) => [timer.ms, timer.cleared])).toEqual([[20_000, true]]);
      // Never answered: stopped at the deadline, and the read says what happened.
      const hung = classes(asThePlatformFetch((_url: string, init: Init) => stopped(init.signal)));
      const waiting = hung.sql(SQL, []).catch((error: Error) => error.message);
      await started(2);
      set[1].fire();
      expect(await waiting).toBe("the queue store did not answer within 20 seconds");
      // Answered, with a body that never ends: the same.
      const endless = classes(
        asThePlatformFetch(async (_url: string, init: Init) => ({
          ok: true,
          status: 200,
          json: () => stopped(init.signal),
        })),
      );
      const reading = endless.sql(SQL, []).catch((error: Error) => error.message);
      await started(3);
      set[2].fire();
      expect(await reading).toBe("the queue store did not answer within 20 seconds");
      // Refused: its answer's body is let go of, unread; and a failure that is not the
      // deadline's is said as itself.
      let letGo = 0;
      const refused = classes(
        asThePlatformFetch(async () => ({
          ok: false,
          status: 500,
          json: async () => ({}),
          body: {
            cancel: async () => {
              letGo++;
            },
          },
        })),
      );
      expect(await refused.sql(SQL, []).catch((error: Error) => error.message)).toBe("queue store answered http 500");
      expect(letGo).toBe(1);
      const down = classes(
        asThePlatformFetch(async () => {
          throw new Error("connection refused");
        }),
      );
      expect(await down.sql(SQL, []).catch((error: Error) => error.message)).toBe("connection refused");
      expect(set.map((timer) => [timer.ms, timer.cleared])).toEqual([
        [20_000, true],
        [20_000, true],
        [20_000, true],
        [20_000, true],
        [20_000, true],
      ]);
    } finally {
      globalThis.setTimeout = real.set;
      globalThis.clearTimeout = real.clear;
    }
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

  test("a call has twenty seconds for its answer and the body: stopped then, it fails with the reason, and however it ends it leaves no timer running", async () => {
    // The timers a call starts, held here instead of run: the test fires the deadline itself.
    const real = { set: globalThis.setTimeout, clear: globalThis.clearTimeout };
    const set: { fire: () => void; ms: number; cleared: boolean }[] = [];
    globalThis.setTimeout = ((fire: () => void, ms: number) => set.push({ fire, ms, cleared: false })) as never;
    globalThis.clearTimeout = ((id: number) => {
      if (set[id - 1]) set[id - 1].cleared = true;
    }) as never;
    const started = async (count: number) => {
      for (let turn = 0; turn < 100 && set.length < count; turn++) await Promise.resolve();
      expect(set.length).toBe(count);
    };
    type Init = { signal: AbortSignal };
    const stopped = (signal: AbortSignal) =>
      new Promise<never>((_resolve, reject) => signal.addEventListener("abort", () => reject(signal.reason)));
    const secrets = { githubToken: "gh-test" };
    const call = ["fleet", "gh", "api", "repos/developing-today/code/pulls"];
    try {
      // Answered: the timer set for it is cleared.
      const fine = edgeRun(
        directConfig(edgeConfig),
        secrets,
        asThePlatformFetch(async (_url: string, init: Init) => ({
          ok: !init.signal.aborted,
          status: 200,
          headers: new Headers(),
          json: async () => [1],
        })) as unknown as Fetch,
      );
      expect(await fine(call)).toEqual({ code: 0, stdout: "[1]", stderr: "" });
      expect(set.map((timer) => [timer.ms, timer.cleared])).toEqual([[20_000, true]]);
      // Never answered: at the deadline the call is stopped, and says why.
      const hung = edgeRun(
        directConfig(edgeConfig),
        secrets,
        asThePlatformFetch((_url: string, init: Init) => stopped(init.signal)) as unknown as Fetch,
      );
      const waiting = hung(call);
      await started(2);
      set[1].fire();
      expect(await waiting).toEqual({
        code: 1,
        stdout: "",
        stderr: "GitHub could not be reached: The operation timed out.",
      });
      // Answered, with a body that never ends: stopped the same way, and (as with any body cut
      // off part-way) the answer is an empty one this once.
      const endless = edgeRun(
        directConfig(edgeConfig),
        secrets,
        asThePlatformFetch(async (_url: string, init: Init) => ({
          ok: true,
          status: 200,
          headers: new Headers(),
          json: () => stopped(init.signal),
        })) as unknown as Fetch,
      );
      const reading = endless(call);
      await started(3);
      expect(set[2].cleared).toBe(false);
      set[2].fire();
      expect(await reading).toEqual({ code: 0, stdout: "", stderr: "" });
      // A failure of its own (out of reach) clears the timer too.
      const down = edgeRun(
        directConfig(edgeConfig),
        secrets,
        asThePlatformFetch(async () => {
          throw new Error("connection refused");
        }) as unknown as Fetch,
      );
      expect((await down(call)).code).toBe(1);
      expect(set.map((timer) => [timer.ms, timer.cleared])).toEqual([
        [20_000, true],
        [20_000, true],
        [20_000, true],
        [20_000, true],
      ]);
    } finally {
      globalThis.setTimeout = real.set;
      globalThis.clearTimeout = real.clear;
    }
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
      done: ["queue", "github"],
      agents: ["bridge"],
      pipeline: ["bridge"],
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
  const state: {
    limited: Record<string, string> | null;
    tags: boolean;
    status: number | null;
    cut: boolean;
    bodiless: number;
  } = {
    limited: null,
    tags: true,
    status: null,
    // The connection drops while a good answer's body is being read.
    cut: false,
    // How often the body of a "not modified" answer, which has none, was asked for.
    bodiless: 0,
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
      return {
        ok: false,
        status: 304,
        headers: new Headers({ etag }),
        json: async () => {
          state.bodiless++;
          return JSON.parse("");
        },
      };
    }
    return {
      ok: true,
      status: 200,
      headers: new Headers(state.tags ? { etag } : {}),
      json: async () => {
        if (state.cut) throw new TypeError("terminated");
        return list.body;
      },
    };
  }) as unknown as Fetch;
  const store = mapStore();
  const memory = options.memory === false ? undefined : githubMemory(store, { now: () => clock.t });
  const run = edgeRun(
    directConfig(edgeConfig),
    { githubToken: "gh-test", seen: (call) => seen.push(call), github: memory, now: () => clock.t },
    asThePlatformFetch(fetchImpl) as typeof fetchImpl,
  );
  const api = (...argv: string[]) => run(["fleet", "gh", "api", ...argv]);
  const change = (url: string, body: unknown) => {
    lists[url] = { rev: (lists[url]?.rev ?? 0) + 1, body };
  };
  /** The object put away and another started: nothing in memory, the same store. */
  const again = () => {
    const fresh = githubMemory(store, { now: () => clock.t });
    const rerun = edgeRun(
      directConfig(edgeConfig),
      { githubToken: "gh-test", seen: (call) => seen.push(call), github: fresh, now: () => clock.t },
      asThePlatformFetch(fetchImpl) as typeof fetchImpl,
    );
    return { memory: fresh, api: (...argv: string[]) => rerun(["fleet", "gh", "api", ...argv]) };
  };
  return { api, calls, seen, state, clock, memory, store, again, change };
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
    // "Not modified" has no body, and none was asked for.
    expect(g.state.bodiless).toBe(0);
  });

  test("a question with a time in it is asked once and never the same again: its answer is not kept, and keeps nothing else out", async () => {
    // The history behind the Trends screen asks "everything changed
    // since <time>", a page at a time, each with a later time. Kept,
    // a hundred such answers would push out the lists that are asked
    // for again and again.
    const g = tagging();
    await g.api(PULLS);
    const since = (time: string) =>
      `repos/developing-today/code/issues?state=all&sort=updated&direction=asc&since=${time}&per_page=100`;
    const first = await g.api(since("2026-11-01T00:00:00Z"));
    expect(first.code).toBe(0);
    await g.api(since("2026-11-02T00:00:00Z"));
    // Asked the same again, it is asked in full: no tag was kept for it.
    await g.api(since("2026-11-01T00:00:00Z"));
    expect(g.calls.slice(1).map((call) => call.tag)).toEqual([null, null, null]);
    // One answer is kept: the list that is asked for again.
    expect((await g.memory?.said())?.kept).toBe(1);
    expect(await g.memory?.get(at(since("2026-11-01T00:00:00Z")))).toBeUndefined();
    await g.api(PULLS);
    expect(g.calls[g.calls.length - 1].tag).toBe('W/"1"');
  });

  test("each address has its own answer, and a write is never asked for by a tag", async () => {
    const g = tagging();
    await g.api(PULLS);
    // (Changed twice before it is first read, so its tag is not the other list's.)
    g.change(at(ISSUES), []);
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
    expect((await g.memory?.said())?.kept).toBe(3);
  });

  test("an answer with no tag is not kept, one that is gone is let go, and with no memory every call is a full one", async () => {
    const g = tagging();
    await g.api(PULLS);
    // GitHub stops tagging: the full answer is used, and the old one is not kept under it.
    g.state.tags = false;
    await g.api(PULLS);
    await g.api(PULLS);
    expect(g.calls.map((call) => call.tag)).toEqual([null, 'W/"1"', null]);
    expect((await g.memory?.said())?.kept).toBe(0);
    // Gone from GitHub: what was kept under the address goes too.
    g.state.tags = true;
    await g.api(PULLS);
    expect((await g.memory?.said())?.kept).toBe(1);
    g.state.status = 404;
    expect((await g.api(PULLS)).code).toBe(1);
    expect((await g.memory?.said())?.kept).toBe(0);
    // A failure that says nothing of the list leaves what was kept alone.
    g.state.status = null;
    await g.api(PULLS);
    g.state.status = 502;
    expect(await g.api(PULLS)).toEqual({ code: 1, stdout: "", stderr: "GitHub answered http 502" });
    expect((await g.memory?.said())?.kept).toBe(1);
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

  test("an answer cut off part-way is not kept under its tag: the next read is a full one, and gets the list", async () => {
    const g = tagging();
    // The list is read whole, and kept.
    expect((await g.api(PULLS)).stdout).toBe('[{"number":1}]');
    // It changes, and the connection drops while the new one is being read: this once the
    // answer is an empty one (as it was before anything was kept), and nothing is kept of it.
    g.change(at(PULLS), [{ number: 1 }, { number: 2 }]);
    g.state.cut = true;
    expect(await g.api(PULLS)).toEqual({ code: 0, stdout: "", stderr: "" });
    expect((await g.memory?.said())?.kept).toBe(0);
    // So the next read carries no tag, GitHub cannot say "not modified" of an answer this
    // side never had, and the list comes whole.
    g.state.cut = false;
    expect((await g.api(PULLS)).stdout).toBe('[{"number":1},{"number":2}]');
    expect(g.calls.map((call) => call.tag)).toEqual([null, 'W/"1"', null]);
    expect((await g.api(PULLS)).stdout).toBe('[{"number":1},{"number":2}]');
    expect(g.calls[3].tag).toBe('W/"2"');
    // Cut off on the very first read: the same.
    const fresh = tagging();
    fresh.state.cut = true;
    expect(await fresh.api(ISSUES)).toEqual({ code: 0, stdout: "", stderr: "" });
    fresh.state.cut = false;
    expect((await fresh.api(ISSUES)).stdout).toBe('[{"number":1}]');
    expect(fresh.calls.map((call) => call.tag)).toEqual([null, null]);
  });

  test("turned away for calling too often with a time to ask again: the time is told, said at the front of why, and nothing is asked until it has come", async () => {
    const g = tagging();
    await g.api(PULLS);
    const back = g.clock.t + 40 * 60_000;
    g.state.limited = { "x-ratelimit-remaining": "0", "x-ratelimit-reset": String(back / 1000) };
    const turned = await g.api(PULLS);
    const said = '{"message":"API rate limit exceeded for user ID 1."}';
    expect(turned).toEqual({
      code: 1,
      stdout: "",
      stderr: `GitHub says to ask again at ${new Date(back).toISOString()}: ${said}`,
    });
    expect(g.seen[1]).toEqual({ to: "github", write: false, status: 403, limited: true, until: back });
    // Until then nothing goes out: not a read, not a write, though GitHub would answer now.
    g.state.limited = null;
    g.clock.t += 39 * 60_000;
    const asked = g.calls.length;
    const read = await g.api(ISSUES);
    const write = await g.api("-X", "PUT", "repos/developing-today/code/pulls/55/merge");
    expect(g.calls.length).toBe(asked);
    // Why is the time and GitHub's own words, so whoever reads it can tell which allowance it was.
    for (const held of [read, write]) {
      expect(held).toEqual({
        code: 1,
        stdout: "",
        stderr: `GitHub says to ask again at ${new Date(back).toISOString()}: not asked again since GitHub answered: ${said}`,
      });
    }
    // Each is recorded as the answer it would have had, and as not sent: nothing was written.
    expect(g.seen.slice(2)).toEqual([
      { to: "github", write: false, status: 429, limited: true, until: back, unsent: true },
      { to: "github", write: true, status: 429, limited: true, until: back, unsent: true },
    ]);
    // The time comes: it is asked again, and what was kept is still asked for by its tag.
    g.clock.t = back - 1;
    await g.api(PULLS);
    expect(g.calls.length).toBe(asked);
    g.clock.t = back;
    expect((await g.api(PULLS)).code).toBe(0);
    expect(g.calls[asked]).toEqual({ method: "GET", url: at(PULLS), tag: 'W/"1"' });
  });

  test("searches are counted apart: their allowance running out does not hush the rest, nor the rest theirs; going too fast hushes both", async () => {
    const SEARCH = "search/issues?q=repo%3Adeveloping-today%2Fcode+label%3Anow";
    const g = tagging();
    const start = g.clock.t;
    // The minute's searches are used up, back in forty seconds.
    g.state.limited = { "x-ratelimit-remaining": "0", "x-ratelimit-reset": String((start + 40_000) / 1000) };
    expect((await g.api(SEARCH)).code).toBe(1);
    g.state.limited = null;
    expect(await g.memory?.quiet()).toEqual({
      search: { until: start + 40_000, why: '{"message":"API rate limit exceeded for user ID 1."}' },
    });
    // The lists are asked for as before; another search is not.
    expect((await g.api(PULLS)).code).toBe(0);
    expect((await g.api(SEARCH)).stderr).toContain("not asked again");
    expect(g.calls.map((call) => call.url)).toEqual([at(SEARCH), at(PULLS)]);
    g.clock.t = start + 40_000;
    expect((await g.api(SEARCH)).code).toBe(0);
    // The hour's allowance used up, as a list is read: searches are still asked.
    const back = g.clock.t + 30 * 60_000;
    g.state.limited = { "x-ratelimit-remaining": "0", "x-ratelimit-reset": String(back / 1000) };
    expect((await g.api(PULLS)).code).toBe(1);
    g.state.limited = null;
    const asked = g.calls.length;
    expect((await g.api(SEARCH)).code).toBe(0);
    expect((await g.api(ISSUES)).stderr).toContain("not asked again");
    expect(g.calls.length).toBe(asked + 1);
    // GitHub names the allowance it means, and is believed over the address asked at.
    const named = tagging();
    named.state.limited = {
      "x-ratelimit-remaining": "0",
      "x-ratelimit-reset": String((named.clock.t + 600_000) / 1000),
      "x-ratelimit-resource": "core",
    };
    await named.api(SEARCH);
    expect(Object.keys((await named.memory?.quiet()) ?? {})).toEqual(["core"]);
    // One it names that this side does not know is taken as the call's own.
    const other = tagging();
    other.state.limited = {
      "x-ratelimit-remaining": "0",
      "x-ratelimit-reset": String((other.clock.t + 600_000) / 1000),
      "x-ratelimit-resource": "code_search",
    };
    await other.api(SEARCH);
    expect(Object.keys((await other.memory?.quiet()) ?? {})).toEqual(["search"]);
    // Going too fast is said in seconds, of the caller: both are hushed, and a nearer time
    // never shortens a further one.
    const fast = tagging();
    fast.state.limited = { "x-ratelimit-remaining": "0", "x-ratelimit-reset": String((fast.clock.t + 900_000) / 1000) };
    await fast.api(PULLS);
    fast.state.limited = { "retry-after": "60" };
    await fast.api(SEARCH);
    const waits = await fast.memory?.quiet();
    expect([waits?.core?.until, waits?.search?.until]).toEqual([fast.clock.t + 900_000, fast.clock.t + 60_000]);
  });

  test("an answer is an answer: one that says nothing is left of the hour hushes nothing, and neither does being turned away with no time", async () => {
    const g = tagging();
    // The last call the hour allows is answered in full, and its headers say nothing is left.
    const run = edgeRun(
      directConfig(edgeConfig),
      { githubToken: "gh-test", github: g.memory, now: () => g.clock.t },
      (async () => ({
        ok: true,
        status: 200,
        headers: new Headers({
          etag: '"a"',
          "x-ratelimit-remaining": "0",
          "x-ratelimit-reset": String((g.clock.t + 1_800_000) / 1000),
        }),
        json: async () => [],
      })) as unknown as Fetch,
    );
    expect((await run(["fleet", "gh", "api", PULLS])).code).toBe(0);
    expect(await g.memory?.quiet()).toEqual({});
    // Turned away with no time given: told as limited, and the next call is asked all the same.
    g.state.limited = {};
    expect((await g.api(ISSUES)).stderr).toBe('{"message":"API rate limit exceeded for user ID 1."}');
    expect(g.seen.at(-1)).toEqual({ to: "github", write: false, status: 403, limited: true });
    g.state.limited = null;
    expect((await g.api(ISSUES)).code).toBe(0);
    expect(await g.memory?.quiet()).toEqual({});
  });

  test("when to ask again: the seconds GitHub gives, or the turn of the hour when nothing is left of it; never the past, never much over an hour", () => {
    const now = Date.UTC(2026, 10, 23, 17, 0, 0);
    const said = (headers: Record<string, string>) => askAgainAt(new Headers(headers), now);
    const reset = (seconds: number) => ({
      "x-ratelimit-remaining": "0",
      "x-ratelimit-reset": String(now / 1000 + seconds),
    });
    expect(said({ "retry-after": "60" })).toEqual({ until: now + 60_000, whole: true });
    expect(said({ "retry-after": "0" })).toEqual({ until: now, whole: true });
    expect(said(reset(1500))).toEqual({ until: now + 1_500_000, whole: false });
    // Both: the seconds are for this caller, now.
    expect(said({ "retry-after": "30", ...reset(1500) })).toEqual({ until: now + 30_000, whole: true });
    // Something left of the allowance: the turn of the hour is not when to ask again.
    expect(said({ "x-ratelimit-remaining": "12", "x-ratelimit-reset": String(now / 1000 + 1500) })).toBeUndefined();
    expect(said({ "x-ratelimit-reset": String(now / 1000 + 1500) })).toBeUndefined();
    expect(said({})).toBeUndefined();
    expect(askAgainAt(undefined, now)).toBeUndefined();
    // A time that cannot be right is cut to a little over the hour; one already past is now.
    expect(said({ "retry-after": "86400" })?.until).toBe(now + QUIET_MOST_MS);
    expect(said(reset(7 * 3600))?.until).toBe(now + QUIET_MOST_MS);
    expect(said(reset(-30))?.until).toBe(now);
    // A date, as the header may also be written, is not read: no time is better than a wrong one.
    expect(said({ "retry-after": "Mon, 23 Nov 2026 18:00:00 GMT" })).toBeUndefined();
    // Which allowance a call draws on is by what is asked for.
    expect(
      ["search/issues?q=x", "/search/code", "repos/o/r/pulls", "repos/o/search/x", "searches"].map(drawsOn),
    ).toEqual(["search", "search", "core", "core", "core"]);
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
      const at = askAgainAt(new Headers(headers), now)?.until;
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

  test("what is kept outlives the object: another one over the same store asks by the same tag, keeps to the same wait and goes on counting", async () => {
    const g = tagging();
    expect((await g.api(PULLS)).stdout).toBe('[{"number":1}]');
    expect(await g.memory?.said()).toEqual({ kept: 1, answers: 1, unchanged: 0, since: g.clock.t });
    // The object is put away, as Cloudflare does with an idle one within seconds, and another
    // starts with nothing in memory. The list is still asked for by its tag, and GitHub's
    // "not modified" is still the answer that was kept.
    const second = g.again();
    expect((await second.api(PULLS)).stdout).toBe('[{"number":1}]');
    expect(g.calls.map((call) => call.tag)).toEqual([null, 'W/"1"']);
    expect(await second.memory.said()).toEqual({ kept: 1, answers: 2, unchanged: 1, since: g.clock.t });
    // Turned away with a time: a third object, started during the wait, asks nothing.
    const back = g.clock.t + 30 * 60_000;
    g.state.limited = { "x-ratelimit-remaining": "0", "x-ratelimit-reset": String(back / 1000) };
    expect((await second.api(PULLS)).code).toBe(1);
    g.state.limited = null;
    const third = g.again();
    g.clock.t += 10 * 60_000;
    const asked = g.calls.length;
    expect((await third.api(ISSUES)).stderr).toContain("not asked again since GitHub answered");
    expect(g.calls.length).toBe(asked);
    expect((await third.memory.quiet()).core?.until).toBe(back);
    // "Try again now" is remembered too: a fourth object asks at once.
    await third.memory.free();
    const fourth = g.again();
    expect((await fourth.api(ISSUES)).code).toBe(0);
    expect(await fourth.memory.quiet()).toEqual({});
    // What was turned away was an answer of GitHub's, and is counted; what was held back is not.
    expect(await fourth.memory.said()).toMatchObject({ kept: 2, answers: 4, unchanged: 1 });
  });

  test("a long answer is kept in pieces and read back whole by another object", async () => {
    const g = tagging();
    const long = Array.from({ length: 30 }, (_, n) => ({ number: n, body: `${n}:${"é".repeat(60_000)}` }));
    g.change(at(PULLS), long);
    const first = await g.api(PULLS);
    expect(first.stdout.length).toBeGreaterThan(2 * ANSWER_PIECE);
    const pieces = [...g.store.kept.keys()].filter((key) => key.startsWith("gh:a:"));
    expect(pieces.length).toBe(Math.ceil(first.stdout.length / ANSWER_PIECE));
    for (const key of pieces) {
      // Each piece says whose it is: the address, and the tag.
      const piece = g.store.kept.get(key) as { u: string; e: string; t: string };
      expect([piece.u, piece.e, piece.t.length <= ANSWER_PIECE]).toEqual([at(PULLS), 'W/"1"', true]);
    }
    const second = g.again();
    const again = await second.api(PULLS);
    expect(g.calls[1].tag).toBe('W/"1"');
    expect(again.stdout === first.stdout).toBe(true);
    // A shorter one in its place leaves no piece of the longer behind.
    g.change(at(PULLS), [{ number: 1 }]);
    await second.api(PULLS);
    expect([...g.store.kept.keys()].filter((key) => key.startsWith("gh:a:"))).toHaveLength(1);
    expect((await g.again().api(PULLS)).stdout).toBe('[{"number":1}]');
    // Where a piece ends is counted in characters as the text holds them, and can fall inside
    // one (the two halves of a character outside the first 65,536): the halves come back
    // together, whichever side of an edge each was on.
    const store = mapStore();
    for (const piece of [1, 2, 3, 4, 5, 7]) {
      const text = `a😀b😀😀c${"😀".repeat(9)}d`;
      const answer = { etag: 'W/"halves"', text };
      await githubMemory(store, { piece }).set("u", answer);
      const stored = [...store.kept.entries()].filter(([key]) => key.startsWith("gh:a:"));
      expect([piece, stored.length]).toEqual([piece, Math.ceil(text.length / piece)]);
      expect([piece, await githubMemory(store, { piece }).get("u")]).toEqual([piece, answer]);
    }
  });

  /** A store whose writes stop after so many, as when the object is put
   * away with an answer half kept; and which can be made unreadable. */
  function stopping(inner: MemoryStore) {
    const state = { left: Number.POSITIVE_INFINITY, hit: false, unread: false, writes: 0 };
    const gate = () => {
      if (state.left <= 0) {
        state.hit = true;
        throw new Error("the object was put away");
      }
      state.left--;
      state.writes++;
    };
    const store: MemoryStore = {
      get: async (key) => {
        if (state.unread) throw new Error("storage could not be read");
        return inner.get(key);
      },
      put: async (key, value) => {
        gate();
        await inner.put(key, value);
      },
      delete: async (key) => {
        gate();
        await inner.delete(key);
      },
    };
    return { state, store };
  }

  test("an answer half kept is never taken for one: it reads as nothing kept, and the read after it is a full one", async () => {
    const inner = mapStore();
    const s = stopping(inner);
    const old = { etag: 'W/"old"', text: "a".repeat(ANSWER_PIECE + 10) };
    const next = { etag: 'W/"new"', text: "b".repeat(2 * ANSWER_PIECE + 10) };
    for (let writes = 0; writes <= 5; writes++) {
      inner.kept.clear();
      s.state.left = Number.POSITIVE_INFINITY;
      await githubMemory(s.store).set("u", old);
      // The new answer is three pieces and then the index: stopped after `writes` of them.
      s.state.left = writes;
      await githubMemory(s.store).set("u", next);
      s.state.left = Number.POSITIVE_INFINITY;
      const got = await githubMemory(s.store).get("u");
      // Whole and its own, or nothing: never the old tag over the new words, nor a mixture.
      if (writes === 0) expect([writes, got]).toEqual([writes, old]);
      else if (writes < 4) expect([writes, got]).toEqual([writes, undefined]);
      else expect([writes, got]).toEqual([writes, next]);
      // And keeping it again afterwards works as if nothing had happened.
      await githubMemory(s.store).set("u", next);
      expect(await githubMemory(s.store).get("u")).toEqual(next);
    }
    // The other way: a short one over a long one, stopped at each of its writes (one piece,
    // the two left of the longer taken away from the far end, the index).
    for (let writes = 0; writes <= 5; writes++) {
      inner.kept.clear();
      s.state.left = Number.POSITIVE_INFINITY;
      await githubMemory(s.store).set("u", next);
      const short = { etag: 'W/"short"', text: "c" };
      s.state.left = writes;
      await githubMemory(s.store).set("u", short);
      s.state.left = Number.POSITIVE_INFINITY;
      const got = await githubMemory(s.store).get("u");
      if (writes === 0) expect([writes, got]).toEqual([writes, next]);
      else if (writes < 4) expect([writes, got]).toEqual([writes, undefined]);
      else expect([writes, got]).toEqual([writes, short]);
      // What is in the place is its first so many pieces with none missing, at every stop.
      const ns = [...inner.kept.keys()].filter((key) => key.startsWith("gh:a:0:")).map((key) => Number(key.slice(7)));
      expect([writes, ns.sort()]).toEqual([writes, ns.map((_, n) => n)]);
    }
  });

  test("what a stopped write left in a place is cleared by whoever next writes there or empties it", async () => {
    const pieces = (store: { kept: Map<string, unknown> }) =>
      [...store.kept.keys()].filter((key) => key.startsWith("gh:a:")).sort();
    const short = { etag: 'W/"short"', text: "ab" };
    const long = { etag: 'W/"long"', text: "abcdefghijklmnopqrstu" };
    // A long answer begun over a short one, the store stopping before the index: five pieces
    // in the place, and an index that knows of one.
    const begin = async () => {
      const inner = mapStore();
      const s = stopping(inner);
      await githubMemory(s.store, { piece: 4 }).set("u", short);
      s.state.left = 5;
      await githubMemory(s.store, { piece: 4 }).set("u", long);
      s.state.left = Number.POSITIVE_INFINITY;
      expect([s.state.hit, pieces(inner)]).toEqual([true, [0, 1, 2, 3, 4].map((n) => `gh:a:0:${n}`)]);
      return { inner, s };
    };
    // The next answer kept there is all there is in the place.
    const kept = await begin();
    await githubMemory(kept.s.store, { piece: 4 }).set("u", short);
    expect(pieces(kept.inner)).toEqual(["gh:a:0:0"]);
    expect(await githubMemory(kept.s.store, { piece: 4 }).get("u")).toEqual(short);
    // Let go instead, nothing is left of it: not its pieces, and not its line in the index.
    const gone = await begin();
    await githubMemory(gone.s.store, { piece: 4 }).delete("u");
    expect([pieces(gone.inner), await githubMemory(gone.s.store, { piece: 4 }).get("u")]).toEqual([[], undefined]);
    expect((await githubMemory(gone.s.store, { piece: 4 }).said()).kept).toBe(0);
    expect(gone.inner.kept.get("gh:index")).toEqual({ entries: {} });
    // A place with a piece gone from its middle (nothing here leaves one so; a store that
    // lost a value would) is still cleared of all the index knew it to hold.
    const holed = mapStore();
    await githubMemory(holed, { piece: 4 }).set("u", long);
    expect(pieces(holed)).toHaveLength(6);
    holed.kept.delete("gh:a:0:2");
    await githubMemory(holed, { piece: 4 }).set("u", short);
    expect(pieces(holed)).toEqual(["gh:a:0:0"]);
    await githubMemory(holed, { piece: 4 }).set("u", long);
    holed.kept.delete("gh:a:0:1");
    await githubMemory(holed, { piece: 4 }).delete("u");
    expect(pieces(holed)).toEqual([]);
    // Too big to keep, the same.
    const big = await begin();
    await githubMemory(big.s.store, { piece: 4, one: 3 }).set("u", long);
    expect(pieces(big.inner)).toEqual([]);
    // And in a place no index names (a first answer whose keeping was stopped): the next
    // address to take the place clears it.
    const inner = mapStore();
    const s = stopping(inner);
    s.state.left = 4;
    await githubMemory(s.store, { piece: 4 }).set("never", long);
    s.state.left = Number.POSITIVE_INFINITY;
    expect([s.state.hit, pieces(inner).length, inner.kept.has("gh:index")]).toEqual([true, 4, false]);
    await githubMemory(s.store, { piece: 4 }).set("other", short);
    expect(pieces(inner)).toEqual(["gh:a:0:0"]);
    expect(await githubMemory(s.store, { piece: 4 }).get("other")).toEqual(short);
  });

  test("a place taken over by another address is not read as the first one's, though the tag and the length are the same", async () => {
    const inner = mapStore();
    const s = stopping(inner);
    const memory = githubMemory(s.store, { most: 1 });
    const first = { etag: 'W/"same"', text: "the first address's own." };
    const other = { etag: 'W/"same"', text: "another's, as long as it" };
    expect(other.text.length).toBe(first.text.length);
    await memory.set("first", first);
    type Stored = { entries: Record<string, { id: number }> };
    const place = (inner.kept.get("gh:index") as Stored).entries.first.id;
    // Something else is kept, the first is let go to make room for it, and the store stops
    // before the index says so. The first's place is free as far as this object knows.
    s.state.left = 2;
    await memory.set("between", { etag: 'W/"between"', text: "x" });
    expect([s.state.hit, inner.kept.has(`gh:a:${place}:0`)]).toEqual([true, false]);
    // Another address takes the place, and the store stops again before the index says that.
    s.state.hit = false;
    s.state.left = 1;
    await memory.set("other", other);
    s.state.left = Number.POSITIVE_INFINITY;
    // What the store holds now: an index that still names the first address, and in its
    // place the other's words, under the same tag and of the same length.
    expect(s.state.hit).toBe(true);
    expect(Object.keys((inner.kept.get("gh:index") as Stored).entries)).toEqual(["first"]);
    expect((inner.kept.get(`gh:a:${place}:0`) as { u: string; e: string; t: string }).t).toBe(other.text);
    // Another object, starting from the store, reads nothing for the first: not the other's.
    expect(await githubMemory(s.store, { most: 1 }).get("first")).toBeUndefined();
    expect(await githubMemory(s.store, { most: 1 }).get("other")).toBeUndefined();
    // And the first kept again is the first's again.
    const next = githubMemory(s.store, { most: 1 });
    await next.set("first", first);
    expect(await githubMemory(s.store, { most: 1 }).get("first")).toEqual(first);
  });

  test("two changes to one answer at once are made one after the other: what is kept is the later, whole", async () => {
    const store = mapStore();
    const memory = githubMemory(store, { piece: 4 });
    const long = { etag: 'W/"long"', text: "l".repeat(30) };
    const short = { etag: 'W/"short"', text: "s".repeat(6) };
    for (const [one, two] of [
      [long, short],
      [short, long],
    ]) {
      const reads = await Promise.all([
        memory.set("u", one),
        memory.get("u"),
        memory.set("u", two),
        memory.get("u"),
        memory.delete("gone"),
      ]);
      // A read made meanwhile is nothing, or one of them whole.
      for (const got of [reads[1], reads[3]])
        expect([undefined, one, two]).toContainEqual(got as KeptAnswer | undefined);
      expect(await memory.get("u")).toEqual(two);
      expect(await githubMemory(store, { piece: 4 }).get("u")).toEqual(two);
      const pieces = [...store.kept.keys()].filter((key) => key.startsWith("gh:a:"));
      expect(pieces).toHaveLength(Math.ceil(two.text.length / 4));
    }
  });

  test("answers for several new addresses kept at once each get a place of their own", async () => {
    // As when an object's first look reads every list side by side, none of them kept before.
    const store = mapStore();
    const memory = githubMemory(store, { piece: 4 });
    const answers = Array.from({ length: 7 }, (_, n) => ({
      url: `https://api.github.com/list/${n}`,
      answer: { etag: `W/"${n}"`, text: String(n).repeat(3 + 5 * n) },
    }));
    await Promise.all(answers.map(({ url, answer }) => memory.set(url, answer)));
    const places = Object.values(
      (store.kept.get("gh:index") as { entries: Record<string, { id: number }> }).entries,
    ).map((entry) => entry.id);
    expect(places.sort((a, b) => a - b)).toEqual([0, 1, 2, 3, 4, 5, 6]);
    const again = githubMemory(store, { piece: 4 });
    for (const { url, answer } of answers) {
      expect(await memory.get(url)).toEqual(answer);
      expect(await again.get(url)).toEqual(answer);
    }
    // And kept again at once, one of them let go meanwhile: each is still its own.
    await Promise.all([
      ...answers.map(({ url, answer }) => memory.set(url, { ...answer, etag: `${answer.etag}b` })),
      memory.delete(answers[3].url),
    ]);
    const after = githubMemory(store, { piece: 4 });
    for (const [n, { url, answer }] of answers.entries()) {
      expect([n, await after.get(url)]).toEqual([n, n === 3 ? undefined : { ...answer, etag: `${answer.etag}b` }]);
    }
  });

  test("a store that cannot be read is a memory that forgot, and is not written over: when it can be read again, what it held is still there", async () => {
    const inner = mapStore();
    const s = stopping(inner);
    const clock = { t: 1_000_000 };
    const kept = { etag: 'W/"kept"', text: "kept before the trouble" };
    const before = githubMemory(s.store, { now: () => clock.t });
    await before.set("u", kept);
    await before.hush(["search"], { until: 4_500_000, why: "searching too fast" });
    await before.hush(["core"], { until: 4_000_000, why: "an earlier hour" });
    await before.count(true);
    const held = structuredClone([...inner.kept.entries()]);
    // Another object starts while the store cannot be read. Nothing is kept as far as it can
    // tell, and it does not write what it knows (which is nothing) over what is there.
    s.state.unread = true;
    const memory = githubMemory(s.store, { now: () => clock.t });
    expect(await memory.get("u")).toBeUndefined();
    expect(await memory.said()).toEqual({ kept: 0, answers: 0, unchanged: 0, since: clock.t });
    await memory.set("v", { etag: 'W/"v"', text: "not kept" });
    await memory.delete("u");
    await memory.count(false);
    await memory.free();
    // GitHub's wait is kept to by this object all the same, in its own mind.
    await memory.hush(["core"], { until: 4_800_000, why: "the hour is used up" });
    expect(await memory.quiet()).toEqual({ core: { until: 4_800_000, why: "the hour is used up" } });
    expect([s.state.hit, [...inner.kept.entries()]]).toEqual([false, held]);
    // The store can be read again: the same object finds what was kept, and both waits (of
    // two for one allowance, the later: what the store held does not shorten what was said since).
    s.state.unread = false;
    expect(await memory.get("u")).toEqual(kept);
    expect(await memory.quiet()).toEqual({
      core: { until: 4_800_000, why: "the hour is used up" },
      search: { until: 4_500_000, why: "searching too fast" },
    });
    expect(await memory.said()).toMatchObject({ kept: 1, answers: 1, unchanged: 1 });
    await memory.set("v", { etag: 'W/"v"', text: "kept now" });
    expect(await githubMemory(s.store).get("v")).toEqual({ etag: 'W/"v"', text: "kept now" });
    // A store that fails at everything fails no read of GitHub: the answer is GitHub's.
    const broken = githubMemory({
      get: async () => {
        throw new Error("storage is down");
      },
      put: async () => {
        throw new Error("storage is down");
      },
      delete: async () => {
        throw new Error("storage is down");
      },
    });
    const g = tagging();
    const run = edgeRun(
      directConfig(edgeConfig),
      { githubToken: "gh-test", github: broken, now: () => g.clock.t },
      asThePlatformFetch(async () => ({
        ok: true,
        status: 200,
        headers: new Headers({ etag: '"a"' }),
        json: async () => [7],
      })) as unknown as Fetch,
    );
    expect(await run(["fleet", "gh", "api", PULLS])).toEqual({ code: 0, stdout: "[7]", stderr: "" });
    expect(await run(["fleet", "gh", "api", PULLS])).toEqual({ code: 0, stdout: "[7]", stderr: "" });
    // What the store holds that is not what this writes (another's, or damaged) is nothing kept.
    for (const value of [
      null,
      7,
      "x",
      [],
      { entries: null },
      { entries: { u: {} } },
      // A line that names no pieces would be an empty answer under a tag: GitHub's "not
      // modified" to that tag would then be answered with nothing.
      { entries: { u: { id: 0, etag: 'W/"none"', pieces: 0, size: 0, at: 1 } } },
      { entries: { u: { id: 0.5, etag: 'W/"half"', pieces: 1, size: 1, at: 1 } } },
      { entries: { u: { id: 0, etag: 7, pieces: 1, size: 1, at: 1 } } },
      { entries: { u: { id: 0, etag: 'W/"x"', pieces: 1, size: "1", at: 1 } } },
      { entries: { u: { id: 0, etag: 'W/"x"', pieces: 1, size: 1 } } },
      // More pieces than any answer kept here has, or longer than any is: what was read back
      // would say how much to clear, and how much is held.
      { entries: { u: { id: 0, etag: 'W/"x"', pieces: 11, size: 1, at: 1 } } },
      { entries: { u: { id: 0, etag: 'W/"x"', pieces: 1, size: 6_000_001, at: 1 } } },
      { entries: { u: { id: 0, etag: 'W/"x"', pieces: 1, size: -1, at: 1 } } },
    ]) {
      const odd = mapStore(
        new Map<string, unknown>([
          ["gh:index", value],
          ["gh:quiet", value],
          ["gh:count", value],
        ]),
      );
      const from = githubMemory(odd, { now: () => clock.t });
      expect([value, await from.get("u"), await from.quiet(), await from.said()]).toEqual([
        value,
        undefined,
        {},
        { kept: 0, answers: 0, unchanged: 0, since: clock.t },
      ]);
    }
    // A piece that says it is the answer's and is not as long as the index says the answer is.
    const cut = mapStore();
    await githubMemory(cut).set("u", kept);
    const first = cut.kept.get("gh:a:0:0") as { u: string; e: string; t: string };
    expect([first.u, first.e, first.t]).toEqual(["u", kept.etag, kept.text]);
    first.t = first.t.slice(0, -1);
    expect(await githubMemory(cut).get("u")).toBeUndefined();
    // A wait read back that is further off than one can be set for is not GitHub's, and is not
    // kept to: not by this object, and not by any after it. (One cut down to the longest wait
    // would be cut down afresh by every object that started, and never end.) One that could
    // have been set is kept to, to the minute.
    const far = mapStore(
      new Map<string, unknown>([
        [
          "gh:quiet",
          {
            core: { until: clock.t + 400 * 24 * 60 * 60_000, why: "a time that cannot be right" },
            search: { until: clock.t + 65 * 60_000, why: "the longest there is" },
          },
        ],
      ]),
    );
    const started = clock.t;
    for (let object = 0; object < 4; object++) {
      expect([object, await githubMemory(far, { now: () => clock.t }).quiet()]).toEqual([
        object,
        { search: { until: started + 65 * 60_000, why: "the longest there is" } },
      ]);
      clock.t += 20 * 60_000;
    }
    clock.t = started;
    // The most an entry may say it has is what an answer of the longest kept would have.
    const most = mapStore();
    const full = { etag: 'W/"full"', text: "f".repeat(41) };
    await githubMemory(most, { piece: 4, one: 41 }).set("u", full);
    expect((most.kept.get("gh:index") as { entries: Record<string, { pieces: number }> }).entries.u.pieces).toBe(11);
    expect(await githubMemory(most, { piece: 4, one: 41 }).get("u")).toEqual(full);
    // Two addresses the index gives one place: the place is the first's, and the other's is nothing.
    const shared = mapStore();
    await githubMemory(shared).set("u", kept);
    const index = shared.kept.get("gh:index") as { entries: Record<string, object> };
    index.entries.w = { ...index.entries.u };
    const both = githubMemory(shared);
    expect([await both.get("u"), await both.get("w"), (await both.said()).kept]).toEqual([kept, undefined, 1]);
  });

  test("room is made by letting the oldest go, never the one just kept, and nothing of what went is left in the store", async () => {
    const store = mapStore();
    const clock = { t: 1_000_000 };
    const memory = githubMemory(store, { now: () => clock.t, most: 3, room: 500, one: 300 });
    const answer = (n: number, size = 20) => ({ etag: `W/"${n}"`, text: String(n % 10).repeat(size) });
    for (let n = 0; n < 5; n++) {
      clock.t += 1_000;
      await memory.set(`u${n}`, answer(n));
    }
    // Three at most: the two oldest went.
    expect(await Promise.all([0, 1, 2, 3, 4].map((n) => memory.get(`u${n}`)))).toEqual([
      undefined,
      undefined,
      answer(2),
      answer(3),
      answer(4),
    ]);
    // One asked for a good while after it was kept is marked as still wanted, and is not the
    // next to go; asked for sooner than that, it is not marked (a read is not a write).
    const indexPuts = () => (store.kept.get("gh:index") as { entries: Record<string, { at: number }> }).entries;
    clock.t += 5 * 60 * 60_000;
    expect(await memory.get("u2")).toEqual(answer(2));
    expect(indexPuts().u2.at).toBeLessThan(clock.t);
    clock.t += 2 * 60 * 60_000;
    expect(await memory.get("u2")).toEqual(answer(2));
    expect(indexPuts().u2.at).toBe(clock.t);
    clock.t += 1_000;
    await memory.set("u5", answer(5));
    expect(await Promise.all([2, 3, 4, 5].map((n) => memory.get(`u${n}`)))).toEqual([
      answer(2),
      undefined,
      answer(4),
      answer(5),
    ]);
    // Room in characters: a large one pushes out as many as it takes, but not itself.
    clock.t += 1_000;
    await memory.set("big", answer(6, 280));
    expect((await memory.said()).kept).toBeLessThanOrEqual(3);
    expect(await memory.get("big")).toEqual(answer(6, 280));
    // One too big to keep is not kept, and what was kept under its address goes.
    await memory.set("big", answer(7, 301));
    expect(await memory.get("big")).toBeUndefined();
    // The store holds the index, and the pieces of what the index names, and nothing else;
    // and the places are few, each used again once it is free.
    const index = store.kept.get("gh:index") as { entries: Record<string, { id: number; pieces: number }> };
    const named = Object.values(index.entries).flatMap((e) =>
      Array.from({ length: e.pieces }, (_, n) => `gh:a:${e.id}:${n}`),
    );
    expect([...store.kept.keys()].filter((key) => key.startsWith("gh:a:")).sort()).toEqual(named.sort());
    for (let n = 10; n < 40; n++) await memory.set(`u${n}`, answer(n));
    const places = Object.values((store.kept.get("gh:index") as typeof index).entries).map((e) => e.id);
    expect(Math.max(...places)).toBeLessThanOrEqual(3);
    expect([...store.kept.keys()].filter((key) => key.startsWith("gh:a:"))).toHaveLength(3);
  });

  test("property: through any run of keeping, asking, letting go, starting again and being stopped part-way, what is read back is an answer that was kept or nothing, and a change that finishes leaves nothing behind it", async () => {
    const next = rng(13570);
    const seen = { wholes: 0, nothings: 0, stops: 0, carried: 0, long: 0, livedOn: 0, strict: 0 };
    for (let round = 0; round < 60; round++) {
      const inner = mapStore();
      const s = stopping(inner);
      const clock = { t: 1_000_000 };
      // Every third round nothing is ever stopped: there, nothing at all may be left over.
      const calm = round % 3 === 0;
      const limits = {
        most: 2 + Math.floor(next() * 5),
        room: 60 + Math.floor(next() * 200),
        one: 90,
        piece: 1 + Math.floor(next() * 12),
      };
      const longest = Math.ceil(limits.one / limits.piece);
      const start = () => githubMemory(s.store, { now: () => clock.t, ...limits });
      let memory = start();
      // Whether a change has been stopped under this object: what it has in mind may then be
      // ahead of what the store holds, until another object starts from the store.
      let shaken = false;
      // The model. For each address: the answer last kept by a change that finished, and those
      // whose keeping was stopped part-way since (each may or may not have got far enough to
      // be read, and a later one that got nowhere leaves an earlier one as it was).
      const kept = new Map<string, KeptAnswer>();
      const begun = new Map<string, KeptAnswer[]>();
      const urls = Array.from({ length: 2 + Math.floor(next() * 7) }, (_, n) => `https://api.github.com/list/${n}`);
      let version = 0;
      type Stored = { entries: Record<string, { id: number; pieces: number; size: number }> };
      const index = () => (inner.kept.get("gh:index") ?? { entries: {} }) as Stored;
      const pieceKeys = () => [...inner.kept.keys()].filter((key) => key.startsWith("gh:a:"));
      /** What is in the store that the index it holds does not name. */
      const stray = () => {
        const named = new Set(
          Object.values(index().entries).flatMap((e) =>
            Array.from({ length: e.pieces }, (_, n) => `gh:a:${e.id}:${n}`),
          ),
        );
        return pieceKeys().filter((key) => !named.has(key));
      };
      const read = async (url: string, where: number[]) => {
        const got = await memory.get(url);
        if (got === undefined) {
          seen.nothings++;
          return got;
        }
        seen.wholes++;
        const tried = begun.get(url) ?? [];
        const fits = [kept.get(url), ...tried].some((a) => a?.etag === got.etag && a.text === got.text);
        expect([...where, got.etag, fits]).toEqual([...where, got.etag, true]);
        // One whose keeping was stopped and which can be read was kept after all, and what
        // was begun before it is gone under it.
        const which = tried.findIndex((a) => a.etag === got.etag);
        if (which >= 0) {
          kept.set(url, got);
          begun.set(url, tried.slice(which + 1));
        }
        return got;
      };
      for (let step = 0; step < 220; step++) {
        const where = [round, step];
        clock.t += Math.floor(next() * 3 * 60 * 60_000);
        const url = pick(next, urls);
        const roll = next();
        const strayBefore = stray().length;
        s.state.hit = false;
        if (!calm && roll < 0.9 && next() < 0.3) s.state.left = Math.floor(next() * next() * 30);
        let answer: KeptAnswer | null = null;
        if (roll < 0.45) {
          const size = pick(
            next,
            [0, 1, limits.piece - 1, limits.piece, limits.piece + 1, 3 * limits.piece + 1, 60, 90, 91].filter(
              (n) => n >= 0,
            ),
          );
          answer = { etag: `W/"${++version}"`, text: letters(next, "abcdefgh", size, size) };
          if (size > limits.piece && size <= limits.one) seen.long++;
          await memory.set(url, answer);
        } else if (roll < 0.8) {
          await read(url, where);
        } else if (roll < 0.9) {
          await memory.delete(url);
        } else {
          // Another object over the same store. Nothing having been stopped under the one
          // before, every address reads to it as it read to that one.
          const was = shaken ? null : await Promise.all(urls.map((u) => memory.get(u)));
          memory = start();
          shaken = false;
          if (was) expect([...where, await Promise.all(urls.map((u) => memory.get(u)))]).toEqual([...where, was]);
          for (const u of urls) await read(u, where);
        }
        const stopped = s.state.hit;
        s.state.left = Number.POSITIVE_INFINITY;
        if (stopped) {
          seen.stops++;
          if (answer && answer.text.length <= limits.one) begun.set(url, [...(begun.get(url) ?? []), answer]);
          // Stopped because the object was put away, as often as not; or the store failed
          // under an object that lives on.
          if (next() < 0.5) {
            memory = start();
            shaken = false;
          } else {
            seen.livedOn++;
            shaken = true;
          }
        } else if (answer && answer.text.length > limits.one) {
          kept.delete(url);
          begun.delete(url);
          expect([...where, await memory.get(url)]).toEqual([...where, undefined]);
        } else if (answer) {
          kept.set(url, answer);
          begun.delete(url);
          // The one just kept is there to be read, whatever went to make room for it: by this
          // object, and by the next.
          if (!shaken && next() < 0.3) {
            memory = start();
            seen.carried++;
          }
          expect([...where, await memory.get(url)]).toEqual([...where, answer]);
          if (!shaken) {
            // No more is kept than there is room for (the one just kept aside, if it is by
            // itself more than the room), nor more answers than the most.
            const sizes = Object.values(index().entries).map((e) => e.size);
            const total = sizes.reduce((sum, size) => sum + size, 0);
            expect([...where, total <= Math.max(limits.room, answer.text.length), sizes.length <= limits.most]).toEqual(
              [...where, true, true],
            );
          }
        } else if (roll >= 0.8 && roll < 0.9) {
          kept.delete(url);
          begun.delete(url);
          expect([...where, await memory.get(url)]).toEqual([...where, undefined]);
          if (!shaken && next() < 0.3) expect([...where, await start().get(url)]).toEqual([...where, undefined]);
        }
        // Whatever happened: each place holds its first so many pieces with none missing, a
        // place holds no more than the longest answer has, and the places are few.
        const places = new Map<number, number[]>();
        for (const key of pieceKeys()) {
          const [, id, n] = /^gh:a:(\d+):(\d+)$/.exec(key) ?? [];
          places.set(Number(id), [...(places.get(Number(id)) ?? []), Number(n)]);
        }
        for (const [id, ns] of places) {
          ns.sort((a, b) => a - b);
          expect([...where, id, ns]).toEqual([...where, id, ns.map((_, n) => n)]);
          expect([...where, id <= limits.most, ns.length <= longest]).toEqual([...where, true, true]);
        }
        // A change that finished, in an object nothing was stopped under, left nothing more
        // behind than there was; and where nothing is ever stopped, nothing is ever left.
        if (!stopped && !shaken) {
          seen.strict++;
          expect([...where, stray().length <= strayBefore]).toEqual([...where, true]);
        }
        if (calm) expect([...where, stray()]).toEqual([...where, []]);
      }
    }
    // The run did read answers back, find nothing, stop changes part-way (under objects put
    // away and under ones that lived on), carry answers to a new object, and keep long ones.
    expect([
      seen.wholes > 3_000,
      seen.nothings > 3_000,
      seen.stops > 400,
      seen.livedOn > 200,
      seen.carried > 700,
      seen.long > 1_500,
      seen.strict > 7_000,
    ]).toEqual([true, true, true, true, true, true, true]);
  });
});
