// Whole-Edge acceptance for #13490. Only the platform (KV, issuer,
// GitHub and Hrana) stands in; no console or queue-domain method is mocked.
// Oracles: shared enqueue verb's complete row, persisted held work, and
// transaction state (including rollback), not merely an HTTP success.
import { beforeEach, describe, expect, test } from "bun:test";
import { FixtureQueueStore } from "../../../pkgs/fleet/hosted-worker/src/queue-store.ts";
import { executeQueueVerb, nativeQueueEnv } from "../../../pkgs/fleet/hosted-worker/src/queue.ts";
import { Edge, type KV } from "../edge/src/edge.ts";
import { forgetPoke, type Io, route } from "../edge/src/entry.ts";
import { clearJwksCache } from "../src/auth.ts";
import { readConfig } from "../src/config.ts";
import { promptPacket } from "../src/work.ts";
import { asThePlatformFetch, signer } from "./support.ts";

const NOW = Date.parse("2026-11-24T12:00:00Z");
const AT = new Date(NOW).toISOString();
const OWNER = "owner@example.com";
const TEXT = "Carry out the owner's instruction\nKeep the acceptance checks.";
const ISSUER = {
  name: "cloudflare-access",
  issuer: "https://team.cloudflareaccess.com",
  jwksUri: "https://team.cloudflareaccess.com/cdn-cgi/access/certs",
  audience: "aud-1",
  source: "access" as const,
};
type Row = Record<string, any>;
type Statement = { sql: string; args?: { type: string; value?: string | number }[] };
type Condition = { type: "ok"; step: number } | { type: "not"; cond: Condition } | { type: "and"; conds: Condition[] };

const decode = (stmt: Statement) =>
  (stmt.args ?? []).map((a) =>
    a.type === "null" ? null : a.type === "integer" || a.type === "float" ? Number(a.value) : a.value,
  );
const cell = (value: unknown) =>
  value === null || value === undefined
    ? { type: "null" }
    : typeof value === "number"
      ? { type: "integer", value: String(value) }
      : { type: "text", value: String(value) };
const result = (rows: Row[], names = Object.keys(rows[0] ?? {})) => ({
  cols: names.map((name) => ({ name })),
  rows: rows.map((row) => names.map((name) => cell(row[name]))),
  affected_row_count: 0,
});
// Split SQL lists without splitting function arguments or quoted strings.
function list(sql: string): string[] {
  const out: string[] = [];
  let depth = 0,
    quote = false,
    start = 0;
  for (let i = 0; i < sql.length; i++) {
    if (sql[i] === "'") quote = !quote;
    if (!quote) {
      if (sql[i] === "(") depth++;
      if (sql[i] === ")") depth--;
      if (sql[i] === "," && depth === 0) {
        out.push(sql.slice(start, i).trim());
        start = i + 1;
      }
    }
  }
  return [...out, sql.slice(start).trim()];
}

/** Hrana wire adapter, interpreted in JS over arrays. Unlike a canned SQL
 * answer, SELECT projects whichever task columns the caller asks for.
 * The enqueue read-set and its guarded INSERT see the same live array. */
class QueueWire {
  tasks: Row[] = [];
  events: Row[] = [];
  calls: Statement[] = [];
  transactions: string[] = [];
  failure: "transport" | "step" | null = null;
  private saved: { tasks: Row[]; events: Row[] } | null = null;
  private changes = 0;

  private subset(args: unknown[]) {
    const [queue, key, kind, issue] = args;
    return this.tasks.filter((row) => {
      const payload = JSON.parse(row.payload);
      return (
        (row.queue === queue && row.dedupe_key === key) ||
        (["queued", "claimed", "parked", "handed_off", "blocked", "done"].includes(row.status) &&
          payload.kind === kind &&
          payload.issue?.number === issue)
      );
    });
  }

  private execute(stmt: Statement): Row {
    this.calls.push(stmt);
    const { sql } = stmt;
    const args = decode(stmt);
    if (sql === "BEGIN IMMEDIATE") {
      this.transactions.push(sql);
      this.saved = structuredClone({ tasks: this.tasks, events: this.events });
      return result([]);
    }
    if (sql === "COMMIT" || sql === "ROLLBACK") {
      if (sql === "COMMIT" && this.failure === "step") throw new Error("injected COMMIT step failure");
      this.transactions.push(sql);
      if (sql === "ROLLBACK" && this.saved) Object.assign(this, this.saved);
      this.saved = null;
      return result([]);
    }
    if (sql.startsWith("INSERT INTO tasks")) {
      const fields = list(/^INSERT INTO tasks \((.*?)\) SELECT/s.exec(sql)![1]);
      const guard = args.slice(fields.length);
      const matching = this.subset(guard);
      let unchanged = matching.length === guard[4];
      if (matching.length) {
        const columns = ["id", ...fields];
        const snapshot = guard.slice(9);
        unchanged &&= matching.every((row) =>
          matching.some((_, index) =>
            columns.every((name, col) => row[name] === snapshot[index * columns.length + col]),
          ),
        );
      }
      this.changes = unchanged ? 1 : 0;
      if (unchanged)
        this.tasks.push({
          id: Math.max(0, ...this.tasks.map((row) => row.id)) + 1,
          ...Object.fromEntries(fields.map((name, index) => [name, args[index]])),
        });
      return { ...result([]), affected_row_count: this.changes };
    }
    if (sql.startsWith("INSERT INTO events")) {
      if (!this.changes) return result([]);
      let i = 0;
      const worker_id = args[i++];
      const latest = (queue: unknown, key: unknown) =>
        Math.max(...this.tasks.filter((row) => row.queue === queue && row.dedupe_key === key).map((row) => row.id));
      const task_id = sql.includes("SELECT ?, (SELECT max(id)") ? latest(args[i++], args[i++]) : args[i++];
      const kind = args[i++];
      const detail = JSON.parse(String(args[i++]));
      if (sql.includes("json_set(")) detail.successor_task_id = latest(args[i++], args[i++]);
      this.events.push({ id: this.events.length + 1, worker_id, task_id, kind, detail, created_at: args[i++] });
      this.changes = 1;
      return { ...result([]), affected_row_count: 1 };
    }
    if (sql.startsWith("UPDATE tasks SET ")) {
      const names = list(/^UPDATE tasks SET (.*?) WHERE id = \?/s.exec(sql)![1]).map((set) =>
        set.replace(/ = \?$/, ""),
      );
      const id = args[names.length];
      const guard = args.slice(names.length + 1);
      const row = this.tasks.find((candidate) => candidate.id === id);
      const matched = row !== undefined && names.every((name, index) => row[name] === guard[index]);
      if (matched) names.forEach((name, index) => (row![name] = args[index]));
      this.changes = matched ? 1 : 0;
      return { ...result([]), affected_row_count: this.changes };
    }
    if (sql.startsWith("DELETE FROM leases")) {
      this.changes = 0;
      return { ...result([]), affected_row_count: 0 };
    }
    if (sql.includes("FROM github_mirror_issues")) return result([], ["labels"]);
    const select = /^SELECT (.*?) FROM tasks\b/s.exec(sql);
    if (select) {
      const byId = sql.includes("WHERE id = ?");
      let rows = sql.includes("json_valid(payload)")
        ? this.subset(args)
        : byId
          ? this.tasks.filter((row) => row.id === args[0] && row.queue === args[1])
          : [...this.tasks];
      if (!sql.includes("json_valid(payload)") && !byId) {
        if (sql.includes("queue = ?")) rows = rows.filter((row) => row.queue === args[0]);
        const status = /status = '([^']+)'/.exec(sql)?.[1];
        if (status) rows = rows.filter((row) => row.status === status);
        if (sql.includes("dedupe_key IN")) rows = rows.filter((row) => args.slice(1).includes(row.dedupe_key));
      }
      const projections = list(select[1]);
      if (sql.includes("GROUP BY status")) {
        const statuses = [...new Set(rows.map((row) => row.status))];
        return result(statuses.map((status) => ({ status, n: rows.filter((row) => row.status === status).length })));
      }
      if (sql.includes("GROUP BY"))
        return result(
          [],
          projections.map((p) => / AS (\w+)$/i.exec(p)?.[1] ?? p),
        );
      const fields = projections.map((p) => {
        const alias = /^(.*?) AS (\w+)$/i.exec(p);
        return { expression: alias?.[1] ?? p, name: alias?.[2] ?? p };
      });
      return result(
        rows.map((row) =>
          Object.fromEntries(
            fields.flatMap(({ expression, name }) => {
              if (expression === "*") return Object.entries(row);
              const json = /^json_extract\((\w+), '\$\.(.*?)'\)$/.exec(expression);
              const value = json
                ? json[2].split(".").reduce((v, key) => v?.[key], JSON.parse(row[json[1]]))
                : row[expression];
              return [[name, value ?? null]];
            }),
          ),
        ),
        fields.some((f) => f.expression === "*") ? undefined : fields.map((f) => f.name),
      );
    }
    throw new Error(`unexpected queue SQL: ${sql}`);
  }

  answer(body: string) {
    if (this.failure === "transport") throw new Error("queue connection reset");
    const request = JSON.parse(body).requests[0];
    let answer: Row;
    if (request.type === "execute") answer = this.execute(request.stmt);
    else {
      const step_results: (Row | null)[] = [],
        step_errors: (Row | null)[] = [];
      const allows = (condition: Condition): boolean =>
        condition.type === "ok"
          ? step_results[condition.step] != null
          : condition.type === "not"
            ? !allows(condition.cond)
            : condition.conds.every(allows);
      for (const step of request.batch.steps) {
        let got: Row | null = null,
          error: Row | null = null;
        if (!step.condition || allows(step.condition)) {
          try {
            got = this.execute(step.stmt);
          } catch (e) {
            error = { message: (e as Error).message };
          }
        }
        step_results.push(got);
        step_errors.push(error);
      }
      answer = { step_results, step_errors };
    }
    return new Response(JSON.stringify({ results: [{ type: "ok", response: { result: answer } }] }));
  }
}

async function setup() {
  const access = await signer("edge-queue-door");
  const cfg = readConfig({ HOME: "/h", PM_CONSOLE_OWNER_EMAILS: OWNER });
  const stored = new Map<string, unknown>([
    [
      "cfg",
      {
        owners: [OWNER],
        certNames: [],
        issuers: [ISSUER],
        repo: cfg.repo,
        queue: cfg.queue,
        androidPackage: cfg.androidPackage,
        androidFingerprints: [],
        androidApp: true,
        queueDbUrl: cfg.queueDbUrl,
        queueTokenName: cfg.queueTokenName,
        promptLabel: cfg.promptLabel,
        needsRepos: cfg.needsRepos,
      },
    ],
  ]);
  const kv: KV = {
    get: async (key) => structuredClone(stored.get(key)),
    put: async (key, value) => {
      stored.set(key, structuredClone(value));
    },
    delete: async (key) => {
      stored.delete(key);
    },
  };
  const queue = new QueueWire();
  const github: { method: string; url: string }[] = [];
  const outward = async (url: string | URL | Request, init?: RequestInit) => {
    const u = String(url),
      method = init?.method ?? "GET";
    if (u === ISSUER.jwksUri) return Response.json({ keys: [access.jwk] });
    if (u.startsWith(cfg.queueDbUrl.replace(/\/+$/, ""))) return queue.answer(String(init?.body));
    if (u.startsWith("https://api.github.com/")) {
      github.push({ method, url: u });
      if (method === "POST" && new URL(u).pathname.endsWith("/issues")) {
        return Response.json(
          { number: 7, title: TEXT, html_url: `https://github.com/${cfg.repo}/issues/7` },
          { status: 201 },
        );
      }
      if (new URL(u).pathname === "/search/issues") return Response.json({ items: [], total_count: 0 });
      return Response.json([]);
    }
    throw new Error(`unexpected fetch ${method} ${u}`);
  };
  const later: Promise<unknown>[] = [];
  const edge = new Edge({
    kv,
    now: () => NOW,
    fetchImpl: asThePlatformFetch(outward) as never,
    direct: {
      enabled: true,
      githubToken: "github-test",
      queueToken: "queue-test",
      fetchImpl: asThePlatformFetch(outward) as never,
    },
    later: (work) => {
      later.push(work);
    },
  });
  const io: Io = {
    state: (req) => edge.fetch(req),
    asset: async () => new Response("", { status: 404 }),
    host: async () => null,
    act: async () => "unreached",
    publicHost: "pm.example",
    version: "test",
    later: (work) => {
      later.push(work);
    },
    now: () => NOW,
  };
  const token = await access.sign({
    iss: ISSUER.issuer,
    aud: [ISSUER.audience],
    exp: NOW / 1000 + 600,
    iat: NOW / 1000 - 5,
    sub: "u1",
    email: OWNER,
  });
  const act = (kind: string, args: Row) =>
    route(
      new Request("https://pm.example/console/api/act", {
        method: "POST",
        headers: {
          "cf-ray": "1",
          "cf-access-jwt-assertion": token,
          "content-type": "application/json",
          "x-pm-console": "1",
        },
        body: JSON.stringify({ kind, args }),
      }),
      io,
    );
  const send = () => act("ask.send", { text: TEXT });
  const held = () => {
    const index = stored.get("held:index") as string[] | undefined;
    if (Array.isArray(index)) {
      return index.flatMap((id) => {
        const record = stored.get(`held:${id}`) as { kind: string; args: Row } | undefined;
        return record ? [record] : [];
      });
    }
    return (stored.get("held") ?? []) as { kind: string; args: Row }[];
  };
  return { cfg, queue, github, stored, held, send, act, later };
}

const JSON_FIELDS = new Set(["payload", "metadata", "progress", "result", "ready_condition", "blocked_deps"]);
const normalize = (rows: Row[]) =>
  rows.map((row) =>
    Object.fromEntries(
      Object.entries(row).map(([name, value]) => {
        if (name === "manual_only") return [name, value === 1 || value === true];
        if (JSON_FIELDS.has(name) && typeof value === "string") return [name, JSON.parse(value)];
        return [name, value];
      }),
    ),
  );

/** The shared verb run on the same starting state, for the row and event oracle. */
async function oracleFor(
  w: Awaited<ReturnType<typeof setup>>,
  prepare: (store: FixtureQueueStore) => Promise<void>,
  verb: string,
  request: Record<string, unknown>,
) {
  const oracle = new FixtureQueueStore();
  const env = nativeQueueEnv(() => NOW / 1000);
  const enqueued = await executeQueueVerb(
    oracle,
    "enqueue",
    { queue: w.cfg.queue, packet: promptPacket(w.cfg.repo, w.cfg.queue, 7, TEXT, AT) },
    env,
  );
  expect(enqueued.ok).toBe(true);
  await prepare(oracle);
  const response = await executeQueueVerb(oracle, verb, request, env);
  return { response, oracle };
}

beforeEach(() => {
  clearJwksCache();
  forgetPoke();
});

describe("Edge prompt queue door (#13490)", () => {
  test("ask.send acknowledges and persists the shared enqueue row without held machine work", async () => {
    const w = await setup();
    const response = await w.send();
    expect(response.status).toBe(200);
    expect(await response.json()).toMatchObject({ ok: true, result: { issue: 7, queued: true } });
    expect(w.held().filter((h) => h.kind === "prompt.enqueue")).toEqual([]);
    const oracle = new FixtureQueueStore();
    const enqueued = await executeQueueVerb(
      oracle,
      "enqueue",
      {
        queue: w.cfg.queue,
        packet: promptPacket(w.cfg.repo, w.cfg.queue, 7, TEXT, AT),
      },
      nativeQueueEnv(() => NOW / 1000),
    );
    expect(enqueued.ok).toBe(true);
    const jsonFields = new Set(["payload", "metadata", "progress", "result", "ready_condition", "blocked_deps"]);
    const rows = w.queue.tasks.map((row) =>
      Object.fromEntries(
        Object.entries(row).map(([name, value]) => {
          if (name === "manual_only") return [name, value === 1 || value === true];
          if (jsonFields.has(name) && typeof value === "string") return [name, JSON.parse(value)];
          return [name, value];
        }),
      ),
    );
    expect(rows).toEqual(await oracle.allTasks());
    expect(rows).toHaveLength(1);
    expect(w.queue.events).toEqual(await oracle.allEvents());
    expect(w.queue.transactions).toEqual(["BEGIN IMMEDIATE", "COMMIT"]);
    await Promise.all(w.later);
  });

  test("a second acknowledged ask for the same GitHub issue keeps exactly one task", async () => {
    const w = await setup();
    expect((await w.send()).status).toBe(200);
    const first = structuredClone(w.queue.tasks);
    expect((await w.send()).status).toBe(200);
    expect(w.github.filter((call) => call.method === "POST" && call.url.endsWith("/issues"))).toHaveLength(2);
    expect(w.queue.tasks).toHaveLength(1);
    expect(w.queue.tasks).toEqual(first);
    expect(w.held()).toEqual([]);
    await Promise.all(w.later);
  });

  for (const failure of ["transport", "step"] as const) {
    test(`queue ${failure} failure acknowledges the filed ask and holds only prompt.enqueue`, async () => {
      const w = await setup();
      w.queue.failure = failure;
      const response = await w.send();
      expect(response.status).toBe(200);
      expect(await response.json()).toMatchObject({ ok: true, result: { issue: 7, queued: true } });
      expect(w.held()).toMatchObject([{ kind: "prompt.enqueue", args: { issue: 7, text: TEXT, at: AT } }]);
      expect(w.held()).toHaveLength(1);
      expect(w.queue.tasks).toEqual([]);
      expect(w.queue.events).toEqual([]);
      if (failure === "step") expect(w.queue.transactions).toEqual(["BEGIN IMMEDIATE", "ROLLBACK"]);
      const actsIndex = w.stored.get("acts:index") as string[] | undefined;
      const acts = Array.isArray(actsIndex)
        ? actsIndex.flatMap((id) => {
            const record = w.stored.get(`act:${id}`) as { kind: string; status: string } | undefined;
            return record ? [record] : [];
          })
        : ((w.stored.get("acts") ?? []) as { kind: string; status: string }[]);
      expect(acts).toMatchObject([{ kind: "ask.send", status: "done" }]);
      await Promise.all(w.later);
    });
  }
});

describe("Edge task cancel and unblock door (#13490)", () => {
  const cancel = { task: 1, reason: "owner stopped it" };
  const cancelRequest = { task_id: 1, worker: "pm-console", reason: "owner stopped it" };
  /** Seeds the queued task through the door, then clears the wire's record of it. */
  async function seeded() {
    const w = await setup();
    expect((await w.send()).status).toBe(200);
    w.queue.transactions.splice(0);
    return w;
  }

  test("task.cancel cancels the task and writes its row and events in one transaction, with no held work", async () => {
    const w = await seeded();
    const { response: expected, oracle } = await oracleFor(w, async () => {}, "cancel", cancelRequest);
    expect(expected.ok).toBe(true);
    const response = await w.act("task.cancel", cancel);
    expect(response.status).toBe(200);
    expect(await response.json()).toEqual({ ok: true, result: { task: 1, action: "cancel" } });
    expect(w.queue.tasks).toHaveLength(1);
    expect(w.queue.tasks[0]).toMatchObject({ status: "cancelled", worker_id: null, lease_until: null });
    expect(normalize(w.queue.tasks)).toEqual(await oracle.allTasks());
    expect(w.queue.events).toEqual(await oracle.allEvents());
    expect(w.queue.events.map((event) => event.kind)).toEqual(["enqueued", "cancelled"]);
    expect(w.queue.transactions).toEqual(["BEGIN IMMEDIATE", "COMMIT"]);
    expect(w.held()).toEqual([]);
    await Promise.all(w.later);
  });

  test("task.unblock returns a blocked task to queued and writes its row and events in one transaction", async () => {
    const w = await seeded();
    w.queue.tasks[0].status = "blocked";
    const { response: expected, oracle } = await oracleFor(
      w,
      (store) => store.updateTask(1, { status: "blocked" }),
      "unblock",
      { task_id: 1, worker: "pm-console", reason: "owner cleared it" },
    );
    expect(expected.ok).toBe(true);
    const response = await w.act("task.unblock", { task: 1, reason: "owner cleared it" });
    expect(response.status).toBe(200);
    expect(await response.json()).toEqual({ ok: true, result: { task: 1, action: "unblock" } });
    expect(w.queue.tasks[0]).toMatchObject({ status: "queued", blocked_deps: null });
    expect(normalize(w.queue.tasks)).toEqual(await oracle.allTasks());
    expect(w.queue.events).toEqual(await oracle.allEvents());
    expect(w.queue.events.map((event) => event.kind)).toEqual(["enqueued", "unblocked"]);
    expect(w.queue.transactions).toEqual(["BEGIN IMMEDIATE", "COMMIT"]);
    await Promise.all(w.later);
  });

  test("task.unblock of a task that is not blocked is refused and writes nothing", async () => {
    const w = await seeded();
    const before = structuredClone(w.queue.tasks);
    const events = structuredClone(w.queue.events);
    const response = await w.act("task.unblock", { task: 1, reason: "not blocked" });
    expect(response.status).not.toBe(200);
    expect(JSON.stringify(await response.json())).toContain("unblock_task: task 1 is queued, not blocked");
    expect(w.queue.tasks).toEqual(before);
    expect(w.queue.events).toEqual(events);
    expect(w.queue.transactions).toEqual([]);
    await Promise.all(w.later);
  });

  test("a repeated task.cancel answers the same and writes nothing the second time", async () => {
    const w = await seeded();
    expect(await (await w.act("task.cancel", cancel)).json()).toEqual({
      ok: true,
      result: { task: 1, action: "cancel" },
    });
    const once = structuredClone(w.queue.tasks);
    const events = structuredClone(w.queue.events);
    expect(await (await w.act("task.cancel", cancel)).json()).toEqual({
      ok: true,
      result: { task: 1, action: "cancel" },
    });
    expect(w.queue.tasks).toEqual(once);
    expect(w.queue.events).toEqual(events);
    expect(w.queue.transactions).toEqual(["BEGIN IMMEDIATE", "COMMIT"]);
    await Promise.all(w.later);
  });

  test("a task in another queue is not this queue's to cancel", async () => {
    const w = await seeded();
    w.queue.tasks[0].queue = "elsewhere";
    const before = structuredClone(w.queue.tasks);
    const events = structuredClone(w.queue.events);
    const response = await w.act("task.cancel", cancel);
    expect(response.status).not.toBe(200);
    expect(JSON.stringify(await response.json())).toContain("cancel_repo_task: task 1 does not exist");
    expect(w.queue.tasks).toEqual(before);
    expect(w.queue.events).toEqual(events);
    expect(w.queue.transactions).toEqual([]);
    await Promise.all(w.later);
  });

  test("a queue transport failure on task.cancel leaves the task and its events as they were", async () => {
    const w = await seeded();
    const before = structuredClone(w.queue.tasks);
    const events = structuredClone(w.queue.events);
    w.queue.failure = "transport";
    const response = await w.act("task.cancel", cancel);
    expect(response.status).not.toBe(200);
    // The refusal names the queue door's own cancel path, not a handoff:
    // the store was reached and answered nothing.
    expect(JSON.stringify(await response.json())).toContain("queue cancel");
    expect(w.queue.tasks).toEqual(before);
    expect(w.queue.events).toEqual(events);
    expect(w.queue.transactions).toEqual([]);
    await Promise.all(w.later);
  });

  test("a failed COMMIT on task.cancel rolls the whole change back", async () => {
    const w = await seeded();
    const before = structuredClone(w.queue.tasks);
    const events = structuredClone(w.queue.events);
    w.queue.failure = "step";
    const response = await w.act("task.cancel", cancel);
    expect(response.status).not.toBe(200);
    // The refusal names the queue door's own cancel path: the batch was
    // attempted and rolled back, not handed to a machine.
    expect(JSON.stringify(await response.json())).toContain("queue cancel");
    expect(w.queue.tasks).toEqual(before);
    expect(w.queue.events).toEqual(events);
    expect(w.queue.transactions).toEqual(["BEGIN IMMEDIATE", "ROLLBACK"]);
    await Promise.all(w.later);
  });
});
