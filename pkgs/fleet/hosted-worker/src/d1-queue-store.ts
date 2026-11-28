// The D1 QueueStore (#14866 — the Cloudflare-first programme's
// queue slice): the same broker-protocol backend as
// LibSQLQueueStore and HranaQueueStore, over a Cloudflare D1
// database — the target home for the fleet queues when the hosted
// door's data plane moves to Workers + D1. Same schema DDL, same
// row codecs, same conditional-UPDATE claim primitive (one
// statement, atomic at the DB; the losing door's UPDATE affects
// zero rows and the verb engine re-picks). No second queue, no
// schema fork: a D1 database created from LIBSQL_QUEUE_SCHEMA
// holds the native queue tables with their native names and
// shapes, so rows move between the Turso backend and D1 by export,
// never by translation.
//
// Why this store exists beside the other two: the QueueStore
// boundary is asynchronous (#14592 decision) because a remote DB
// answers over the network; D1 answers through the Worker binding,
// which is the same shape of boundary — prepare/bind, then an
// awaited all()/run() whose meta carries the affected-row count
// and the insert id. The store therefore takes any D1Client (one
// self-contained statement per execute, no connection-scoped
// state: insert ids and affected counts come from the statement's
// own meta, never from last_insert_rowid()/changes() on a held
// connection); D1DatabaseClient below is the production
// implementation over a D1Database binding, and tests inject the
// same seam the other stores' parity suites inject.

import type { Json } from "./queue-validate.ts";
import type { EventRow, HealthSnapshot, LaneRow, LedgerRow, LeaseRow, QueueStore, TaskRow } from "./queue-store.ts";
import {
  LIBSQL_QUEUE_SCHEMA,
  LEDGER_COLS,
  LEDGER_INSERT_COLS,
  TASK_COLS,
  TASK_INSERT_COLS,
  encodeTaskValue,
  failWhere,
  intOrNull,
  jsonValue,
  readEvent,
  readLedger,
  readTask,
  textOrNull,
  toStoredJson,
  type SqlParam,
} from "./libsql-queue-store.ts";

/** One statement against the D1 database: SQL plus positional args
 *  in the sync store's param shape (the client encodes them as D1
 *  bind values). */
export interface D1Statement {
  sql: string;
  args: SqlParam[];
}

/** One statement's outcome: decoded rows (column name → value, the
 *  same shapes the sync store's readers consume), the affected-row
 *  count (the conditional update's win/lose signal) and the insert
 *  id when the statement inserted. */
export interface D1Result {
  rows: Record<string, unknown>[];
  affectedRowCount: number;
  lastInsertRowId: number | null;
}

/** The async SQL surface this store needs. D1DatabaseClient is the
 *  production implementation over a D1Database binding; anything
 *  with this shape (a test peer over a local SQLite file, another
 *  transport) injects the same way the Hrana store takes its
 *  client. */
export interface D1Client {
  execute(stmt: D1Statement): Promise<D1Result>;
}

/** The slice of a statement's result meta this adapter reads
 *  (D1Meta: changes = rows the statement wrote, last_row_id = the
 *  rowid its INSERT assigned). */
export interface D1MetaLike {
  changes?: number;
  last_row_id?: number;
}

/** A prepared D1 statement, structurally: the subset of
 *  D1PreparedStatement the adapter calls. */
export interface D1PreparedStatementLike {
  bind(...values: unknown[]): D1PreparedStatementLike;
  all(): Promise<{ results: Record<string, unknown>[]; meta: D1MetaLike }>;
  run(): Promise<{ meta: D1MetaLike }>;
}

/** A D1 database binding, structurally: the subset of D1Database
 *  the adapter calls. The Worker runtime supplies the real
 *  binding; tests supply a double with this exact shape, so the
 *  bundle never imports a runtime-specific type. */
export interface D1DatabaseLike {
  prepare(sql: string): D1PreparedStatementLike;
}

/** Encode one store param as a D1 bind value: D1 binds null,
 *  numbers, strings and byte arrays; booleans store as the
 *  integers the schema's columns already hold, and bigints (row
 *  ids, counts — always small) travel as numbers. */
function toD1Value(v: SqlParam | ArrayBuffer | Uint8Array): unknown {
  if (v === null || v === undefined) return null;
  if (typeof v === "boolean") return v ? 1 : 0;
  if (typeof v === "bigint") return Number(v);
  return v;
}

/** The production D1Client: one prepared statement per execute
 *  against the bound D1 database. Row-returning statements read
 *  through all(); everything else runs, and its meta supplies the
 *  affected-row count and insert id — the two signals the store's
 *  claim primitive and insert paths consume. Stateless per call,
 *  so no baton or session state is held between statements —
 *  safe across Worker isolates. */
export class D1DatabaseClient implements D1Client {
  constructor(private readonly db: D1DatabaseLike) {}

  async execute(stmt: D1Statement): Promise<D1Result> {
    const bound = this.db.prepare(stmt.sql).bind(...stmt.args.map(toD1Value));
    if (/^\s*(select|pragma|values|with)\b/i.test(stmt.sql)) {
      const { results } = await bound.all();
      return { rows: results, affectedRowCount: 0, lastInsertRowId: null };
    }
    const { meta } = await bound.run();
    return { rows: [], affectedRowCount: meta.changes ?? 0, lastInsertRowId: meta.last_row_id ?? null };
  }
}

export interface D1QueueStoreOpts {
  /** Clock for checkpoint updated_at (never read back through the
   *  interface; injected for deterministic tests). */
  nowIso?: () => string;
  /** What health reports: the queue project source the database
   *  serves. D1 has no netloc; describe() reports null there. */
  projectSource?: string;
}

/** The QueueStore over a Cloudflare D1 database. Method for method
 *  the same contract as LibSQLQueueStore and HranaQueueStore — the
 *  three stores are interchangeable behind the async boundary,
 *  which the parity tests prove against all three. Schema DDL runs
 *  lazily on first use (construction stays synchronous for the
 *  worker seam; the DDL is the shared IF NOT EXISTS set — against
 *  a fresh D1 database it creates the queue tables, against a
 *  database the deployment slice already provisioned it is a
 *  no-op). */
export class D1QueueStore implements QueueStore {
  private readonly nowIso: () => string;
  private readonly projectSource: string;
  private schemaReady: Promise<void> | null = null;

  constructor(
    private readonly client: D1Client,
    opts: D1QueueStoreOpts = {},
  ) {
    this.nowIso = opts.nowIso ?? (() => new Date().toISOString());
    this.projectSource = opts.projectSource ?? "fleet-queue";
  }

  private ready(): Promise<void> {
    this.schemaReady ??= (async () => {
      for (const stmt of LIBSQL_QUEUE_SCHEMA) {
        await this.client.execute({ sql: stmt, args: [] });
      }
      // The tasks schema-generation migration (#14985): ALTER in
      // the columns a live-generation (export-shaped) tasks table
      // predates. Recorded in libsql-queue-store.ts; fixture and
      // determinations in test/queue-live-generation-fixture.ts.
      const cols = await this.client.execute({ sql: "SELECT name FROM pragma_table_info('tasks')", args: [] });
      if (!cols.rows.some((col) => col["name"] === "manual_only")) {
        await this.client.execute({
          sql: "ALTER TABLE tasks ADD COLUMN manual_only INTEGER NOT NULL DEFAULT 0",
          args: [],
        });
      }
      if (!cols.rows.some((col) => col["name"] === "metadata")) {
        await this.client.execute({
          sql: "ALTER TABLE tasks ADD COLUMN metadata TEXT NOT NULL DEFAULT '{}'",
          args: [],
        });
      }
    })();
    return this.schemaReady;
  }

  private async exec(sql: string, ...args: SqlParam[]): Promise<D1Result> {
    await this.ready();
    return this.client.execute({ sql, args });
  }

  async insertTask(row: Omit<TaskRow, "id">): Promise<TaskRow> {
    const rec = row as unknown as Record<string, unknown>;
    const cols = TASK_INSERT_COLS;
    const placeholders = cols.map(() => "?").join(", ");
    const res = await this.exec(
      `INSERT INTO tasks (${cols.join(", ")}) VALUES (${placeholders})`,
      ...cols.map((c) => encodeTaskValue(c, rec[c])),
    );
    const id = res.lastInsertRowId ?? failWhere("insertTask lost the inserted row id");
    const got = await this.getTask(id);
    return got ?? failWhere("insertTask lost the inserted row");
  }

  async getTask(id: number): Promise<TaskRow | null> {
    const { rows } = await this.exec(`SELECT ${TASK_COLS} FROM tasks WHERE id = ?`, id);
    if (rows.length === 0) return null;
    return readTask(rows[0] as Record<string, unknown>);
  }

  async tasksByDedupe(queue: string, dedupeKey: string): Promise<TaskRow[]> {
    const { rows } = await this.exec(
      `SELECT ${TASK_COLS} FROM tasks WHERE queue = ? AND dedupe_key = ? ORDER BY id ASC`,
      queue,
      dedupeKey,
    );
    return rows.map((r) => readTask(r as Record<string, unknown>));
  }

  async allTasks(): Promise<TaskRow[]> {
    const { rows } = await this.exec(`SELECT ${TASK_COLS} FROM tasks ORDER BY id ASC`);
    return rows.map((r) => readTask(r as Record<string, unknown>));
  }

  async liveIdentityCandidates(kind: string, statuses: string[]): Promise<TaskRow[]> {
    if (statuses.length === 0) return [];
    const placeholders = statuses.map(() => "?").join(", ");
    const { rows } = await this.exec(
      `SELECT ${TASK_COLS} FROM tasks WHERE status IN (${placeholders}) AND json_extract(payload, '$.kind') = ? ORDER BY id ASC`,
      ...statuses,
      kind,
    );
    return rows.map((r) => readTask(r as Record<string, unknown>));
  }

  async claimedExpired(nowIso: string): Promise<TaskRow[]> {
    const { rows } = await this.exec(
      `SELECT ${TASK_COLS} FROM tasks WHERE status = 'claimed' AND lease_until IS NOT NULL AND lease_until < ? ORDER BY id ASC`,
      nowIso,
    );
    return rows.map((r) => readTask(r as Record<string, unknown>));
  }

  async healthSnapshot(queueFilter: string | null, since24Iso: string, since7Iso: string): Promise<HealthSnapshot> {
    const qf = queueFilter !== null ? " AND queue = ?" : "";
    const qArgs: SqlParam[] = queueFilter !== null ? [queueFilter] : [];
    const num = (v: unknown): number => (typeof v === "number" ? v : typeof v === "bigint" ? Number(v) : 0);
    const str = (v: unknown): string => (typeof v === "string" ? v : "");
    const counts = (
      await this.exec(
        `SELECT queue, status, COUNT(*) AS n FROM tasks WHERE 1 = 1${qf} GROUP BY queue, status`,
        ...qArgs,
      )
    ).rows;
    const atCap = (
      await this.exec(
        `SELECT queue, COUNT(*) AS n FROM tasks WHERE status = 'queued' AND attempts >= max_attempts${qf} GROUP BY queue`,
        ...qArgs,
      )
    ).rows;
    const oldest = (
      await this.exec(
        `SELECT queue, MIN(created_at) AS oldest FROM tasks WHERE status = 'queued'${qf} GROUP BY queue`,
        ...qArgs,
      )
    ).rows;
    const dep = (
      await this.exec(
        `SELECT queue, COUNT(*) AS n FROM tasks WHERE status = 'blocked' AND blocked_deps IS NOT NULL${qf} GROUP BY queue`,
        ...qArgs,
      )
    ).rows;
    const parked = (
      await this.exec(
        `SELECT queue, not_before FROM tasks WHERE status = 'parked' AND not_before IS NOT NULL${qf}`,
        ...qArgs,
      )
    ).rows;
    const t7 = (
      await this.exec(
        `SELECT t.queue AS queue, COUNT(*) AS n FROM events e JOIN tasks t ON t.id = e.task_id WHERE e.kind IN ('completed', 'failed') AND e.created_at > ?${queueFilter !== null ? " AND t.queue = ?" : ""} GROUP BY t.queue`,
        ...(queueFilter !== null ? [since7Iso, queueFilter] : [since7Iso]),
      )
    ).rows;
    const t24 = (
      await this.exec(
        `SELECT t.queue AS queue, COUNT(*) AS n FROM events e JOIN tasks t ON t.id = e.task_id WHERE e.kind IN ('completed', 'failed') AND e.created_at > ?${queueFilter !== null ? " AND t.queue = ?" : ""} GROUP BY t.queue`,
        ...(queueFilter !== null ? [since24Iso, queueFilter] : [since24Iso]),
      )
    ).rows;
    return {
      statusCounts: counts.map((r) => ({ queue: str(r["queue"]), status: str(r["status"]), count: num(r["n"]) })),
      atCapQueued: atCap.map((r) => ({ queue: str(r["queue"]), count: num(r["n"]) })),
      oldestQueuedAt: oldest.map((r) => ({
        queue: str(r["queue"]),
        oldest: typeof r["oldest"] === "string" ? (r["oldest"] as string) : null,
      })),
      depBlocked: dep.map((r) => ({ queue: str(r["queue"]), count: num(r["n"]) })),
      parkedNotBefore: parked.map((r) => ({ queue: str(r["queue"]), not_before: str(r["not_before"]) })),
      terminal7: t7.map((r) => ({ queue: str(r["queue"]), count: num(r["n"]) })),
      terminal24: t24.map((r) => ({ queue: str(r["queue"]), count: num(r["n"]) })),
    };
  }

  async conditionalUpdateTask(id: number, guard: Partial<TaskRow>, set: Partial<TaskRow>): Promise<boolean> {
    const grec = guard as Record<string, unknown>;
    const srec = set as Record<string, unknown>;
    const setCols = Object.keys(srec);
    if (setCols.length === 0) return (await this.getTask(id)) !== null;
    const setSql = setCols.map((c) => `${c} = ?`).join(", ");
    const setArgs = setCols.map((c) => encodeTaskValue(c, srec[c]));
    const guardCols = Object.keys(grec);
    const conds: string[] = ["id = ?"];
    const condArgs: SqlParam[] = [id];
    for (const c of guardCols) {
      const v = grec[c] ?? null;
      if (v === null) {
        conds.push(`${c} IS NULL`);
      } else if (typeof v === "number" || typeof v === "bigint" || typeof v === "string") {
        conds.push(`${c} = ?`);
        condArgs.push(v as string | number | bigint);
      } else {
        // Object/array/boolean guard (JSON columns): canonicalize
        // both sides through json() so key order and spacing never
        // decide a claim race — the sync store's exact rule.
        conds.push(`json(${c}) = json(?)`);
        condArgs.push(JSON.stringify(v));
      }
    }
    const res = await this.exec(`UPDATE tasks SET ${setSql} WHERE ${conds.join(" AND ")}`, ...setArgs, ...condArgs);
    return res.affectedRowCount > 0;
  }

  async updateTask(id: number, set: Partial<TaskRow>): Promise<void> {
    const rec = set as Record<string, unknown>;
    const cols = Object.keys(rec);
    if (cols.length === 0) return;
    const setSql = cols.map((c) => `${c} = ?`).join(", ");
    await this.exec(`UPDATE tasks SET ${setSql} WHERE id = ?`, ...cols.map((c) => encodeTaskValue(c, rec[c])), id);
  }

  async getLedger(taskId: number): Promise<LedgerRow | null> {
    const { rows } = await this.exec(`SELECT ${LEDGER_COLS} FROM repo_task_lane_ledger WHERE task_id = ?`, taskId);
    if (rows.length === 0) return null;
    return readLedger(rows[0] as Record<string, unknown>);
  }

  async insertLedger(row: Omit<LedgerRow, "id">): Promise<LedgerRow> {
    const rec = row as unknown as Record<string, unknown>;
    const placeholders = LEDGER_INSERT_COLS.map(() => "?").join(", ");
    const res = await this.exec(
      `INSERT INTO repo_task_lane_ledger (${LEDGER_INSERT_COLS.join(", ")}) VALUES (${placeholders})`,
      ...LEDGER_INSERT_COLS.map((c) => (c === "task_id" ? intOrNull(rec[c], c) : textOrNull(rec[c], c))),
    );
    const id = res.lastInsertRowId ?? failWhere("insertLedger lost the inserted row id");
    const { rows } = await this.exec(`SELECT ${LEDGER_COLS} FROM repo_task_lane_ledger WHERE id = ?`, id);
    if (rows.length === 0) failWhere("insertLedger lost the inserted row");
    return readLedger(rows[0] as Record<string, unknown>);
  }

  async updateLedger(taskId: number, set: Partial<LedgerRow>): Promise<void> {
    const rec = set as Record<string, unknown>;
    const cols = Object.keys(rec);
    if (cols.length === 0) return;
    const setSql = cols.map((c) => `${c} = ?`).join(", ");
    await this.exec(
      `UPDATE repo_task_lane_ledger SET ${setSql} WHERE task_id = ?`,
      ...cols.map((c) => (c === "task_id" ? intOrNull(rec[c], c) : textOrNull(rec[c], c))),
      taskId,
    );
  }

  async upsertLease(row: LeaseRow, opts: { keepWorker: boolean }): Promise<void> {
    if (opts.keepWorker) {
      // The heartbeat path (lifecycle.rs): the holder keeps its
      // worker; only the lease times move.
      await this.exec(
        `INSERT INTO leases (task_id, worker_id, lease_until, heartbeat_at)
         VALUES (?, ?, ?, ?)
         ON CONFLICT(task_id) DO UPDATE SET lease_until = excluded.lease_until,
           heartbeat_at = excluded.heartbeat_at`,
        row.task_id,
        row.worker_id,
        row.lease_until,
        row.heartbeat_at,
      );
    } else {
      await this.exec(
        `INSERT INTO leases (task_id, worker_id, lease_until, heartbeat_at)
         VALUES (?, ?, ?, ?)
         ON CONFLICT(task_id) DO UPDATE SET worker_id = excluded.worker_id,
           lease_until = excluded.lease_until, heartbeat_at = excluded.heartbeat_at`,
        row.task_id,
        row.worker_id,
        row.lease_until,
        row.heartbeat_at,
      );
    }
  }

  async deleteLease(taskId: number): Promise<void> {
    await this.exec("DELETE FROM leases WHERE task_id = ?", taskId);
  }

  async deleteExpiredLeases(nowIso: string): Promise<void> {
    await this.exec("DELETE FROM leases WHERE lease_until < ?", nowIso);
  }

  async getLane(laneId: string): Promise<LaneRow | null> {
    const { rows } = await this.exec("SELECT lane_id, worker_id, queue, enabled FROM lanes WHERE lane_id = ?", laneId);
    if (rows.length === 0) return null;
    const r = rows[0] as Record<string, unknown>;
    return {
      lane_id: textOrNull(r["lane_id"], "lane_id") ?? "",
      worker_id: textOrNull(r["worker_id"], "worker_id"),
      queue: textOrNull(r["queue"], "queue"),
      enabled: intOrNull(r["enabled"], "enabled") ?? 0,
    };
  }

  async insertLane(row: LaneRow): Promise<void> {
    await this.exec(
      "INSERT INTO lanes (lane_id, worker_id, queue, enabled) VALUES (?, ?, ?, ?)",
      row.lane_id,
      row.worker_id,
      row.queue,
      row.enabled,
    );
  }

  async touchWorker(workerId: string, working: boolean, nowIso: string): Promise<void> {
    // work_identity.rs touch_worker: unknown workers enter with an
    // empty role; the touch moves only status and heartbeat, never
    // role, display_name or created_at.
    await this.exec(
      `INSERT INTO workers (worker_id, role, status, last_heartbeat_at, created_at)
       VALUES (?, '', ?, ?, ?)
       ON CONFLICT(worker_id) DO UPDATE SET
         last_heartbeat_at = excluded.last_heartbeat_at,
         status = excluded.status`,
      workerId,
      working ? "working" : "idle",
      nowIso,
      nowIso,
    );
  }

  async upsertCheckpoint(workerId: string, key: string, value: Json): Promise<void> {
    await this.exec(
      `INSERT INTO checkpoints (worker_id, key, value, updated_at)
       VALUES (?, ?, ?, ?)
       ON CONFLICT(worker_id, key) DO UPDATE SET
         value = excluded.value, updated_at = excluded.updated_at`,
      workerId,
      key,
      toStoredJson(value) ?? "{}",
      this.nowIso(),
    );
  }

  async appendEvent(
    workerId: string | null,
    taskId: number | null,
    kind: string,
    detail: Json,
    createdAt: string,
  ): Promise<EventRow> {
    const res = await this.exec(
      "INSERT INTO events (worker_id, task_id, kind, detail, created_at) VALUES (?, ?, ?, ?, ?)",
      workerId,
      taskId,
      kind,
      toStoredJson(detail) ?? "{}",
      createdAt,
    );
    const id = res.lastInsertRowId ?? failWhere("appendEvent lost the inserted row id");
    const { rows } = await this.exec(
      "SELECT id, worker_id, task_id, kind, detail, created_at FROM events WHERE id = ?",
      id,
    );
    if (rows.length === 0) failWhere("appendEvent lost the inserted row");
    return readEvent(rows[0] as Record<string, unknown>);
  }

  async allEvents(): Promise<EventRow[]> {
    const { rows } = await this.exec(
      "SELECT id, worker_id, task_id, kind, detail, created_at FROM events ORDER BY id ASC",
    );
    return rows.map((r) => readEvent(r as Record<string, unknown>));
  }

  async eventsForTask(taskId: number): Promise<EventRow[]> {
    const { rows } = await this.exec(
      "SELECT id, worker_id, task_id, kind, detail, created_at FROM events WHERE task_id = ? ORDER BY id ASC",
      taskId,
    );
    return rows.map((r) => readEvent(r as Record<string, unknown>));
  }

  async deleteEventsBefore(cutoffIso: string): Promise<number> {
    const res = await this.exec("DELETE FROM events WHERE created_at < ?", cutoffIso);
    return res.affectedRowCount;
  }

  async mirrorLabels(repo: string, issueNumber: number): Promise<string[] | null> {
    // github_mirror_issues is the github-ci Worker's read model, not
    // this store's table: no row → null; a failing read (no table, no
    // database) throws so the caller warns and proceeds unenriched.
    const { rows } = await this.exec(
      "SELECT labels FROM github_mirror_issues WHERE repo = ? AND number = ?",
      repo,
      issueNumber,
    );
    if (rows.length === 0) return null;
    const parsed = jsonValue(rows[0]["labels"], "labels");
    return Array.isArray(parsed) ? parsed.filter((l): l is string => typeof l === "string") : [];
  }

  async describe(): Promise<{ backend: string; projectSource: string; netloc: string | null }> {
    return { backend: "d1", projectSource: this.projectSource, netloc: null };
  }
}

/** The D1 opener: turn a D1Database binding into the queue store.
 *  The store constructs synchronously — schema DDL runs lazily on
 *  first use — so the worker seam wires it exactly like the other
 *  openers. */
export function openD1QueueStore(db: D1DatabaseLike, opts: D1QueueStoreOpts = {}): D1QueueStore {
  return new D1QueueStore(new D1DatabaseClient(db), opts);
}
