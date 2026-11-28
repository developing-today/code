// The remote QueueStore (#14592): the same broker-protocol backend
// as LibSQLQueueStore, over a remote Turso DB spoken to in Hrana —
// the production shape of the hosted door (#12589), whose whole
// point is a front door over the existing fleet-queue Turso DB
// (#6741), not a file beside the Worker. Same schema DDL, same row
// codecs, same conditional-UPDATE claim primitive (one statement,
// atomic at the DB; the losing door's UPDATE affects zero rows and
// the verb engine re-picks). No second queue, no schema fork.
//
// Why this store exists beside the sync one: the QueueStore
// boundary is asynchronous (#14592 decision, recorded on the
// issue) because a remote DB answers over the network and the
// hosted runtime cannot block on I/O. LibSQLQueueStore satisfies
// the boundary with immediately-resolving promises over an
// embedded connection; this store satisfies it with real network
// round trips. A synchronous facade over Hrana was the rejected
// alternative: it cannot exist in a Worker at all, and in Node it
// would need a worker-thread bridge — a second execution model
// for the same contract.
//
// Transport: Hrana over HTTP (the /v2/pipeline endpoint) via
// fetch — zero dependencies, Worker-native, and injectable: the
// store takes any HranaClient, so tests can drive it against a
// local sqld/turso-dev endpoint or a scripted peer. Each execute
// is one self-contained pipeline (statement + close): the store
// never relies on connection-scoped state — insert ids and
// affected-row counts come from the execute result itself, not
// from last_insert_rowid()/changes() on a held connection.

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

/** One statement against the remote DB: SQL plus positional args
 *  in the sync store's param shape (the transport encodes them as
 *  Hrana typed values). */
export interface HranaStatement {
  sql: string;
  args: SqlParam[];
}

/** One statement's outcome: decoded rows (column name → value, the
 *  same shapes the sync store's readers consume), the affected-row
 *  count (the conditional update's win/lose signal) and the insert
 *  id when the statement inserted. */
export interface HranaResult {
  rows: Record<string, unknown>[];
  affectedRowCount: number;
  lastInsertRowId: number | null;
}

/** The async SQL surface this store needs. HranaHttpClient is the
 *  production implementation; anything with this shape (a test
 *  peer, another transport) injects the same way the sync store
 *  takes its connection. */
export interface HranaClient {
  execute(stmt: HranaStatement): Promise<HranaResult>;
}

type HranaValue =
  | { type: "null" }
  | { type: "text"; value: string }
  | { type: "integer"; value: string }
  | { type: "float"; value: number }
  | { type: "blob"; value: string };

function encodeArg(v: SqlParam | ArrayBuffer | Uint8Array): HranaValue {
  if (v === null || v === undefined) return { type: "null" };
  if (typeof v === "string") return { type: "text", value: v };
  if (typeof v === "bigint") return { type: "integer", value: v.toString() };
  if (typeof v === "boolean") return { type: "integer", value: v ? "1" : "0" };
  if (typeof v === "number") {
    return Number.isInteger(v) ? { type: "integer", value: String(v) } : { type: "float", value: v };
  }
  const bytes = v instanceof Uint8Array ? v : new Uint8Array(v);
  let bin = "";
  for (const b of bytes) bin += String.fromCharCode(b);
  return { type: "blob", value: btoa(bin) };
}

function decodeValue(v: HranaValue): unknown {
  switch (v.type) {
    case "null":
      return null;
    case "text":
      return v.value;
    case "integer":
      return Number(v.value);
    case "float":
      return v.value;
    case "blob": {
      const bin = atob(v.value);
      const bytes = new Uint8Array(bin.length);
      for (let i = 0; i < bin.length; i++) bytes[i] = bin.charCodeAt(i);
      return bytes;
    }
  }
}

/** Hrana over HTTP against a libSQL server / Turso DB endpoint
 *  (https://… for Turso, http://127.0.0.1:<port> for a local
 *  sqld / `turso dev`). Stateless per call: one pipeline carrying
 *  the statement and a close, so no baton or stream state is held
 *  between statements — safe across Worker isolates. */
export class HranaHttpClient implements HranaClient {
  constructor(
    private readonly baseUrl: string,
    private readonly authToken: string | null = null,
  ) {}

  async execute(stmt: HranaStatement): Promise<HranaResult> {
    const res = await fetch(`${this.baseUrl}/v2/pipeline`, {
      method: "POST",
      headers: {
        "content-type": "application/json",
        ...(this.authToken ? { authorization: `Bearer ${this.authToken}` } : {}),
      },
      body: JSON.stringify({
        requests: [{ type: "execute", stmt: { sql: stmt.sql, args: stmt.args.map(encodeArg) } }, { type: "close" }],
      }),
    });
    if (!res.ok) {
      failWhere(`hrana pipeline HTTP ${res.status} for ${this.baseUrl}`);
    }
    const body = (await res.json()) as {
      results?: { type: string; error?: { message: string }; response?: { result?: unknown } }[];
    };
    const first = body.results?.[0];
    if (!first || first.type !== "ok") {
      failWhere(`hrana execute failed: ${first?.error?.message ?? "no result"}`);
    }
    const result = first.response?.result as {
      cols: { name: string }[];
      rows: HranaValue[][];
      affected_row_count: number;
      last_insert_row_id: string | null;
    };
    const names = result.cols.map((c) => c.name);
    const rows = result.rows.map((row) => {
      const rec: Record<string, unknown> = {};
      row.forEach((v, i) => {
        rec[names[i] as string] = decodeValue(v);
      });
      return rec;
    });
    return {
      rows,
      affectedRowCount: result.affected_row_count,
      lastInsertRowId: result.last_insert_row_id === null ? null : Number(result.last_insert_row_id),
    };
  }
}

export interface HranaQueueStoreOpts {
  /** Clock for checkpoint updated_at (never read back through the
   *  interface; injected for deterministic tests). */
  nowIso?: () => string;
  /** What health reports: the Turso project source and netloc the
   *  connection serves (the opener fills netloc from the URL). */
  projectSource?: string;
  netloc?: string | null;
}

/** The production QueueStore over a remote Turso DB. Method for
 *  method the same contract as LibSQLQueueStore — the two stores
 *  are interchangeable behind the async boundary, which the parity
 *  tests prove against both. Schema DDL runs lazily on first use
 *  (construction stays synchronous for the worker seam; the DDL
 *  is the shared IF NOT EXISTS set, idempotent against the live
 *  fleet-queue DB). */
export class HranaQueueStore implements QueueStore {
  private readonly nowIso: () => string;
  private readonly projectSource: string;
  private readonly netloc: string | null;
  private schemaReady: Promise<void> | null = null;

  constructor(
    private readonly client: HranaClient,
    opts: HranaQueueStoreOpts = {},
  ) {
    this.nowIso = opts.nowIso ?? (() => new Date().toISOString());
    this.projectSource = opts.projectSource ?? "fleet-queue";
    this.netloc = opts.netloc ?? null;
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

  private async exec(sql: string, ...args: SqlParam[]): Promise<HranaResult> {
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
    return { backend: "libsql", projectSource: this.projectSource, netloc: this.netloc };
  }
}

/** The remote opener: turn a Turso/libSQL URL (+ its auth token)
 *  into the queue store. `libsql://` URLs map to https, the
 *  scheme Turso issues; http(s) URLs pass through (a local
 *  sqld / `turso dev` endpoint in tests and local dev). The
 *  store constructs synchronously — schema DDL runs lazily on
 *  first use — so the worker seam wires it exactly like the
 *  file opener. */
export function openHranaQueueStore(
  url: string,
  authToken?: string | null,
  opts: HranaQueueStoreOpts = {},
): HranaQueueStore {
  const httpUrl = url.startsWith("libsql://") ? `https://${url.slice("libsql://".length)}` : url;
  const netloc = opts.netloc ?? new URL(httpUrl).host;
  return new HranaQueueStore(new HranaHttpClient(httpUrl, authToken ?? null), { ...opts, netloc });
}
