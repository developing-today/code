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
    if (sql.includes("FROM github_mirror_issues")) return result([], ["labels"]);
    const select = /^SELECT (.*?) FROM tasks\b/s.exec(sql);
    if (select) {
      let rows = sql.includes("json_valid(payload)") ? this.subset(args) : [...this.tasks];
      if (!sql.includes("json_valid(payload)")) {
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
  const send = () =>
    route(
      new Request("https://pm.example/console/api/act", {
        method: "POST",
        headers: {
          "cf-ray": "1",
          "cf-access-jwt-assertion": token,
          "content-type": "application/json",
          "x-pm-console": "1",
        },
        body: JSON.stringify({ kind: "ask.send", args: { text: TEXT } }),
      }),
      io,
    );
  const held = () => (stored.get("held") ?? []) as { kind: string; args: Row }[];
  return { cfg, queue, github, stored, held, send, later };
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
    const jsonFields = new Set(["payload", "progress", "result", "ready_condition", "blocked_deps"]);
    const rows = w.queue.tasks.map((row) =>
      Object.fromEntries(
        Object.entries(row).map(([name, value]) => [
          name,
          jsonFields.has(name) && typeof value === "string" ? JSON.parse(value) : value,
        ]),
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
      expect(w.stored.get("acts")).toMatchObject([{ kind: "ask.send", status: "done" }]);
      await Promise.all(w.later);
    });
  }
});
