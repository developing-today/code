// Oracles: the native SQLite schema, the hosted enqueue engine, and rows
// persisted by the actual Hrana SQL (not a mock's idea of deduplication).
import { Database } from "bun:sqlite";
import { describe, expect, test } from "bun:test";
import { readFileSync } from "node:fs";
import { executeQueueVerb, nativeQueueEnv } from "../../../pkgs/fleet/hosted-worker/src/queue.ts";
import { FixtureQueueStore } from "../../../pkgs/fleet/hosted-worker/src/queue-store.ts";
import { directClasses } from "../edge/src/direct.ts";
import { readConfig } from "../src/config.ts";
import type { EdgeConfig } from "../src/edge-proto.ts";
import type { Fetch } from "../src/ops.ts";
import { promptPacket } from "../src/work.ts";
import { asThePlatformFetch } from "./support.ts";

const at = "2026-11-24T12:00:00.000Z";
const now = Date.parse(at);
const cfg = readConfig({ HOME: "/h", PM_CONSOLE_OWNER_EMAILS: "owner@example.com" });
const edge = {
  ...cfg,
  owners: ["owner@example.com"],
  queue: "repo-health-code-pilot",
  repo: "developing-today/code",
} as unknown as EdgeConfig;
const packet = () => promptPacket(edge.repo, edge.queue, 456, "Please do the work", at);
type Cell = { type: string; value?: string };
type Statement = { sql: string; args?: Cell[] };
type Condition = { type: string; step?: number; cond?: Condition; conds?: Condition[] };
const cell = (value: unknown): Cell =>
  value === null ? { type: "null" } : { type: typeof value === "number" ? "integer" : "text", value: String(value) };

function harness(hooks: { beforeBatch?: () => Promise<void>; failSql?: (sql: string) => boolean } = {}) {
  const db = new Database(":memory:");
  const source = readFileSync(
    new URL("../../../pkgs/fleet/crates/fleet-queue/src/turso_queue/core.rs", import.meta.url),
    "utf8",
  );
  const schema = source.slice(source.indexOf("pub(crate) const SCHEMA:"));
  for (const match of schema.matchAll(/"((?:\\.|[^"\\])*)"/gs)) {
    const sql = match[1].replace(/\\\n\s*/g, "");
    if (sql.startsWith("CREATE TABLE") || sql.startsWith("CREATE INDEX") || sql.startsWith("CREATE UNIQUE INDEX")) {
      db.exec(sql);
    }
    if (match.index! > schema.indexOf("\n];")) break;
  }
  const calls: { url: string; headers: Headers; sql: string[] }[] = [];
  const fetchImpl = asThePlatformFetch(async (url: string, init: { body?: string; headers: HeadersInit }) => {
    const body = JSON.parse(init.body ?? "{}") as {
      requests: { type: string; stmt?: Statement; batch?: { steps: { stmt: Statement; condition?: Condition }[] } }[];
    };
    calls.push({
      url,
      headers: new Headers(init.headers),
      sql: body.requests.flatMap((r) => (r.stmt ? [r.stmt.sql] : [])),
    });
    const execute = ({ sql, args = [] }: Statement) => {
      try {
        if (hooks.failSql?.(sql)) throw new Error("injected SQL failure");
        const query = db.query(sql);
        const bound = args.map((arg) =>
          arg.type === "null" ? null : arg.type === "integer" ? Number(arg.value) : (arg.value ?? ""),
        );
        const rows = query.all(...bound) as Record<string, unknown>[];
        const columns = query.columnNames;
        const changes = (db.query("SELECT changes() AS n").get() as { n: number }).n;
        return {
          type: "ok",
          response: {
            type: "execute",
            result: {
              cols: columns.map((name) => ({ name })),
              rows: rows.map((row) => columns.map((name) => cell(row[name]))),
              affected_row_count: changes,
              last_insert_rowid: String((db.query("SELECT last_insert_rowid() AS n").get() as { n: number }).n),
            },
          },
        };
      } catch (error) {
        return { type: "error", error: { message: String(error) } };
      }
    };
    if (body.requests.some((request) => request.batch)) await hooks.beforeBatch?.();
    const results = body.requests.map((request) => {
      if (request.stmt) return execute(request.stmt);
      if (request.batch) {
        const step_results: unknown[] = [];
        const step_errors: unknown[] = [];
        const condition = (c?: Condition): boolean => {
          if (!c) return true;
          if (c.type === "ok") return step_results[c.step!] != null;
          if (c.type === "not") return !condition(c.cond);
          if (c.type === "and") return c.conds!.every(condition);
          throw new Error(`unsupported Hrana condition: ${c.type}`);
        };
        for (const step of request.batch.steps) {
          const result = condition(step.condition) ? execute(step.stmt) : null;
          step_results.push(result?.type === "ok" ? result.response?.result : null);
          step_errors.push(result?.type === "error" ? result.error : null);
        }
        return { type: "ok", response: { type: "batch", result: { step_results, step_errors } } };
      }
      return { type: "ok", response: { type: "close" } };
    });
    return { ok: true, status: 200, json: async () => ({ results }) };
  }) as unknown as Fetch;
  const direct = directClasses(edge, {
    queueToken: "queue-test",
    queueDb: "https://queue.example",
    fetchImpl,
    now: () => now,
  });
  const otherQueue = directClasses(
    { ...edge, queue: "other-pilot" },
    {
      queueToken: "queue-test",
      queueDb: "https://queue.example",
      fetchImpl,
      now: () => now,
    },
  );
  return { db, calls, direct, otherQueue };
}

describe("the edge queue door", () => {
  test("native filing-recheck threshold leaves hour-old stamps to a machine", async () => {
    const { db, direct, calls } = harness();
    try {
      const old = promptPacket(edge.repo, edge.queue, 456, "Please do the work", new Date(now - 3600_001).toISOString());
      expect((await direct.queueEnqueue(old)).code).not.toBe(0);
      expect(db.query("SELECT COUNT(*) AS n FROM tasks").get()).toEqual({ n: 0 });
      expect(calls).toHaveLength(0);
      const boundary = promptPacket(edge.repo, edge.queue, 456, "Please do the work", new Date(now - 3600_000).toISOString());
      expect((await direct.queueEnqueue(boundary)).code).toBe(0);
      expect(db.query("SELECT COUNT(*) AS n FROM tasks").get()).toEqual({ n: 1 });
    } finally {
      db.close();
    }
  });

  test("matches the shared engine row and deduplicates a second filing in real SQLite", async () => {
    const { db, direct, calls } = harness();
    try {
      expect((await direct.queueEnqueue(packet())).code).toBe(0);
      expect((await direct.queueEnqueue(packet())).code).toBe(0);
      expect(db.query("SELECT COUNT(*) AS n FROM tasks").get()).toEqual({ n: 1 });
      expect(db.query("SELECT COUNT(*) AS n FROM events").get()).toEqual({ n: 0 });
      const oracle = new FixtureQueueStore();
      await executeQueueVerb(
        oracle,
        "enqueue",
        { queue: edge.queue, packet: packet() },
        nativeQueueEnv(() => now / 1000),
      );
      const expected = oracle.allTasks()[0];
      const row = db.query("SELECT * FROM tasks").get() as Record<string, unknown>;
      for (const key of ["payload", "progress", "result", "ready_condition", "blocked_deps"]) {
        if (typeof row[key] === "string") row[key] = JSON.parse(row[key] as string);
      }
      expect(row).toEqual(expected);
      expect(db.query("SELECT blocked_deps, ready_condition FROM tasks").get()).toEqual({
        blocked_deps: null,
        ready_condition: null,
      });
      for (const call of calls) {
        expect(call.url).toBe("https://queue.example/v2/pipeline");
        expect(call.headers.get("authorization")).toBe("Bearer queue-test");
      }
    } finally {
      db.close();
    }
  });

  test("a transport rejection returns a failure that the caller can hold for a machine", async () => {
    const direct = directClasses(edge, {
      queueToken: "queue-test",
      queueDb: "https://queue.example",
      fetchImpl: async () => {
        throw new Error("network down");
      },
      now: () => now,
    });
    expect((await direct.queueEnqueue(packet())).code).not.toBe(0);
  });

  test("live identity under a different queue absorbs the filing", async () => {
    const { db, direct } = harness();
    try {
      expect((await direct.queueEnqueue(packet())).code).toBe(0);
      db.exec("UPDATE tasks SET queue = 'other-pilot'");
      expect((await direct.queueEnqueue(packet())).code).toBe(0);
      expect(db.query("SELECT COUNT(*) AS n FROM tasks").get()).toEqual({ n: 1 });
    } finally {
      db.close();
    }
  });

  test("failed predecessors revive with atomic, correctly remapped linkage", async () => {
    const { db, direct } = harness();
    try {
      expect((await direct.queueEnqueue(packet())).code).toBe(0);
      db.exec("UPDATE tasks SET status = 'failed', completed_at = '2026-11-24T11:59:00Z'");
      expect((await direct.queueEnqueue(packet())).code).toBe(0);
      expect(db.query("SELECT COUNT(*) AS n FROM tasks").get()).toEqual({ n: 2 });
      const events = db.query("SELECT task_id, kind, detail FROM events ORDER BY id").all() as {
        task_id: number;
        kind: string;
        detail: string;
      }[];
      expect(events.map(({ task_id, kind }) => ({ task_id, kind }))).toEqual([
        { task_id: 2, kind: "task_revived" },
        { task_id: 1, kind: "revived_by" },
      ]);
      expect(JSON.parse(events[1].detail).successor_task_id).toBe(2);
      const progress = db.query("SELECT progress FROM tasks WHERE id = 2").get() as { progress: string };
      expect(JSON.parse(progress.progress).revived_from.task_id).toBe(1);
    } finally {
      db.close();
    }
  });

  test("event failure rolls back the new task and all linkage events", async () => {
    const { db, direct } = harness({
      failSql: (sql) => sql.startsWith("INSERT INTO events") && sql.includes("json_set"),
    });
    try {
      expect((await direct.queueEnqueue(packet())).code).toBe(0);
      db.exec("UPDATE tasks SET status = 'failed'");
      expect((await direct.queueEnqueue(packet())).code).not.toBe(0);
      expect(db.query("SELECT COUNT(*) AS n FROM tasks").get()).toEqual({ n: 1 });
      expect(db.query("SELECT COUNT(*) AS n FROM events").get()).toEqual({ n: 0 });
    } finally {
      db.close();
    }
  });

  test("completed work is left for native handling rather than recreated", async () => {
    const { db, direct } = harness();
    try {
      expect((await direct.queueEnqueue(packet())).code).toBe(0);
      db.exec("UPDATE tasks SET status = 'done'");
      expect((await direct.queueEnqueue(packet())).code).not.toBe(0);
      expect(db.query("SELECT COUNT(*) AS n FROM tasks").get()).toEqual({ n: 1 });
    } finally {
      db.close();
    }
  });

  test("a concurrent same-identity writer wins safely even in a different queue", async () => {
    let raced = false;
    const { db, direct, otherQueue } = harness({
      beforeBatch: async () => {
        if (raced) return;
        raced = true;
        expect((await otherQueue.queueEnqueue(packet())).code).toBe(0);
      },
    });
    try {
      expect((await direct.queueEnqueue(packet())).code).toBe(0);
      expect(db.query("SELECT COUNT(*) AS n FROM tasks").get()).toEqual({ n: 1 });
      expect(db.query("SELECT queue FROM tasks").get()).toEqual({ queue: "other-pilot" });
    } finally {
      db.close();
    }
  });
});
