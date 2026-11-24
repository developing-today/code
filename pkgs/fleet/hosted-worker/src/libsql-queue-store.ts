// The production QueueStore (#12589, follow-on to PR #12904): the
// same broker-protocol verb engine as the fixture, over the existing
// fleet-queue Turso database (#6741) — no second queue, no schema
// fork. The DDL below is the queue-table subset of the native
// backend's schema (pkgs/fleet/crates/fleet-queue/src/turso_queue/
// core.rs SCHEMA: tasks, leases, lanes, workers, checkpoints,
// repo_task_lane_ledger, events plus their indexes); CREATE TABLE IF
// NOT EXISTS is idempotent against the live fleet-queue DB, where
// every table already exists with these names and shapes. Tables
// owned by other doors (the github-ci mirror, creation_outbox,
// placement_sessions, actions_exec_*, merge_intents) are NOT created
// here — the mirror in particular is a read model maintained by its
// own Worker (pkgs/github-ci/worker/mirror.sql); mirrorLabels reads
// it and throws when the read itself fails, exactly the "never gates
// work" contract the verb engine relies on (it warns and proceeds).
//
// The store is synchronous by the interface's design (see
// queue-store.ts): one embedded libSQL/SQLite connection whose
// conditional UPDATE is the claim atomicity primitive — the losing
// door's UPDATE matches zero rows and the verb engine re-picks,
// exactly as against the native door. The constructor takes the open
// connection (house pattern: SqlStorageLike, cf. pkgs/id/
// hosted-worker); tests inject node:sqlite, local runs a file-backed
// libSQL DB of the same schema. A remote Turso DB is served by
// HranaQueueStore (libsql-hrana-queue-store.ts, #14592) over the
// same schema and codecs — not a second schema.
//
// Value encoding mirrors the native arg_str/arg_json convention:
// scalars as stored, JSON columns (payload, progress, result,
// ready_condition, blocked_deps, ledger platform fields, event
// detail, checkpoint value) as canonical compact JSON text, parsed
// back on read. Guard comparison for object guards canonicalizes
// both sides through json(), so a row the native broker wrote
// (sorted keys) still matches a guard built from the row this store
// read back.

import type { Json } from "./queue-validate.ts";
import type {
  EventRow,
  LaneRow,
  LedgerRow,
  LeaseRow,
  QueueStore,
  TaskRow,
} from "./queue-store.ts";

/** The synchronous embedded-SQL surface this store needs: the house
 *  SqlStorageLike shape (pkgs/id/hosted-worker/src/bridge-store.ts).
 *  One connection serves the statement and the `SELECT changes()`
 *  that follows it — true for embedded libSQL/SQLite connections. */
export interface SyncSqlDb {
  exec(
    query: string,
    ...params: (string | number | bigint | boolean | null | ArrayBuffer | Uint8Array)[]
  ): { toArray(): Record<string, unknown>[] };
}

export type SqlParam = string | number | bigint | boolean | null;

/** Queue tables of the native fleet-queue schema (core.rs SCHEMA),
 *  verbatim names and shapes. IF NOT EXISTS: safe to run against the
 *  live fleet-queue DB, where these already exist. */
export const LIBSQL_QUEUE_SCHEMA: string[] = [
  `CREATE TABLE IF NOT EXISTS tasks (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    queue TEXT NOT NULL, title TEXT, body TEXT,
    payload TEXT NOT NULL DEFAULT '{}', dedupe_key TEXT,
    priority INTEGER, status TEXT NOT NULL DEFAULT 'queued',
    not_before TEXT, worker_id TEXT, claimed_at TEXT, lease_until TEXT,
    lane_id TEXT, platform TEXT, attempts INTEGER NOT NULL DEFAULT 0,
    max_attempts INTEGER NOT NULL DEFAULT 1000,
    claim_provider TEXT, claim_model TEXT, claim_variant TEXT, claim_session_id TEXT,
    progress TEXT NOT NULL DEFAULT '{}', result TEXT NOT NULL DEFAULT '{}',
    last_error TEXT, park_kind TEXT, wait_kind TEXT, ready_condition TEXT,
    required_budget_minutes INTEGER,
    last_checked_at TEXT, check_outcome TEXT,
    created_at TEXT NOT NULL, updated_at TEXT NOT NULL,
    completed_at TEXT,
    blocked_deps TEXT, dep_next_recheck_at TEXT, dep_blocked_until TEXT,
    dep_recheck_seconds INTEGER)`,
  `CREATE INDEX IF NOT EXISTS tasks_queue_dedupe
    ON tasks(queue, dedupe_key) WHERE dedupe_key IS NOT NULL`,
  `CREATE INDEX IF NOT EXISTS tasks_claim_scan
    ON tasks(queue, status, priority, created_at)`,
  `CREATE TABLE IF NOT EXISTS leases (
    task_id INTEGER PRIMARY KEY, worker_id TEXT NOT NULL,
    lease_until TEXT NOT NULL, heartbeat_at TEXT NOT NULL)`,
  `CREATE TABLE IF NOT EXISTS lanes (
    lane_id TEXT PRIMARY KEY, worker_id TEXT NOT NULL,
    queue TEXT NOT NULL, enabled INTEGER NOT NULL DEFAULT 1)`,
  `CREATE TABLE IF NOT EXISTS workers (
    worker_id TEXT PRIMARY KEY, role TEXT NOT NULL DEFAULT '',
    display_name TEXT, status TEXT NOT NULL DEFAULT 'idle',
    last_heartbeat_at TEXT, created_at TEXT)`,
  `CREATE TABLE IF NOT EXISTS checkpoints (
    worker_id TEXT NOT NULL, key TEXT NOT NULL,
    value TEXT NOT NULL DEFAULT '{}', updated_at TEXT NOT NULL,
    PRIMARY KEY (worker_id, key))`,
  `CREATE TABLE IF NOT EXISTS repo_task_lane_ledger (
    id INTEGER PRIMARY KEY AUTOINCREMENT, queue TEXT NOT NULL,
    task_id INTEGER NOT NULL UNIQUE, assigned_lane TEXT, executed_lane TEXT, executed_lane_id TEXT,
    exclusion_reason TEXT, opencode_reason TEXT, platform_limit TEXT,
    platform_limit_reason TEXT, assigned_by TEXT, session_id TEXT,
    fallback_reason TEXT, assigned_at TEXT, executed_at TEXT,
    updated_at TEXT NOT NULL)`,
  `CREATE TABLE IF NOT EXISTS events (
    id INTEGER PRIMARY KEY AUTOINCREMENT, worker_id TEXT, task_id INTEGER,
    kind TEXT NOT NULL, detail TEXT NOT NULL DEFAULT '{}',
    created_at TEXT NOT NULL)`,
  `CREATE INDEX IF NOT EXISTS events_task ON events(task_id, id)`,
  `CREATE INDEX IF NOT EXISTS events_kind_ts ON events(kind, created_at)`,
  `CREATE INDEX IF NOT EXISTS events_created ON events(created_at)`,
];

// core.rs TASK_COLS / LEDGER_COLS order: the SELECT column order the
// row readers below rely on.
export const TASK_COLS =
  "id, queue, title, body, payload, dedupe_key, priority, status, " +
  "not_before, worker_id, claimed_at, lease_until, lane_id, platform, attempts, max_attempts, " +
  "claim_provider, claim_model, claim_variant, claim_session_id, " +
  "progress, result, last_error, park_kind, wait_kind, ready_condition, " +
  "required_budget_minutes, " +
  "last_checked_at, check_outcome, created_at, updated_at, completed_at, " +
  "blocked_deps, dep_next_recheck_at, dep_blocked_until, dep_recheck_seconds";
export const TASK_INSERT_COLS = TASK_COLS.split(", ").filter((c) => c !== "id");

export const LEDGER_COLS =
  "id, queue, task_id, assigned_lane, executed_lane, executed_lane_id, " +
  "exclusion_reason, opencode_reason, platform_limit, platform_limit_reason, " +
  "assigned_by, session_id, fallback_reason, " +
  "assigned_at, executed_at, updated_at";
export const LEDGER_INSERT_COLS = LEDGER_COLS.split(", ").filter((c) => c !== "id");

const TASK_JSON_COLS = new Set(["payload", "progress", "result", "ready_condition", "blocked_deps"]);
const TASK_INT_COLS = new Set([
  "id",
  "priority",
  "attempts",
  "max_attempts",
  "required_budget_minutes",
  "dep_recheck_seconds",
]);

export function failWhere(what: string): never {
  throw new Error(`libsql-queue-store: ${what}`);
}

export function textOrNull(v: unknown, col: string): string | null {
  if (v === null || v === undefined) return null;
  if (typeof v === "string") return v;
  failWhere(`text column ${col} holds ${typeof v}`);
}

export function intOrNull(v: unknown, col: string): number | null {
  if (v === null || v === undefined) return null;
  if (typeof v === "number" && Number.isFinite(v)) return Math.trunc(v);
  if (typeof v === "bigint") return Number(v);
  failWhere(`integer column ${col} holds ${typeof v}`);
}

export function jsonValue(v: unknown, col: string): Json {
  if (v === null || v === undefined) return null;
  if (typeof v !== "string") failWhere(`JSON column ${col} holds ${typeof v}`);
  return JSON.parse(v) as Json;
}

/** Canonical compact JSON for storage (native arg_json: Value::to_string). */
export function toStoredJson(v: Json): string | null {
  if (v === null || v === undefined) return null;
  return JSON.stringify(v);
}

export function encodeTaskValue(col: string, v: unknown): SqlParam {
  if (TASK_JSON_COLS.has(col)) return toStoredJson(v as Json);
  if (TASK_INT_COLS.has(col)) return intOrNull(v, col);
  return textOrNull(v, col);
}

export function readTask(r: Record<string, unknown>): TaskRow {
  const need = (col: string): unknown => {
    if (!(col in r)) failWhere(`task row lacks column ${col}`);
    return r[col];
  };
  return {
    id: intOrNull(need("id"), "id") ?? failWhere("task row has null id"),
    queue: textOrNull(need("queue"), "queue") ?? "",
    title: textOrNull(need("title"), "title"),
    body: textOrNull(need("body"), "body"),
    payload: jsonValue(need("payload"), "payload"),
    dedupe_key: textOrNull(need("dedupe_key"), "dedupe_key"),
    priority: intOrNull(need("priority"), "priority"),
    status: textOrNull(need("status"), "status") ?? "",
    not_before: textOrNull(need("not_before"), "not_before"),
    worker_id: textOrNull(need("worker_id"), "worker_id"),
    claimed_at: textOrNull(need("claimed_at"), "claimed_at"),
    lease_until: textOrNull(need("lease_until"), "lease_until"),
    lane_id: textOrNull(need("lane_id"), "lane_id"),
    platform: textOrNull(need("platform"), "platform"),
    attempts: intOrNull(need("attempts"), "attempts") ?? 0,
    max_attempts: intOrNull(need("max_attempts"), "max_attempts") ?? 1000,
    claim_provider: textOrNull(need("claim_provider"), "claim_provider"),
    claim_model: textOrNull(need("claim_model"), "claim_model"),
    claim_variant: textOrNull(need("claim_variant"), "claim_variant"),
    claim_session_id: textOrNull(need("claim_session_id"), "claim_session_id"),
    progress: jsonValue(need("progress"), "progress"),
    result: jsonValue(need("result"), "result"),
    last_error: textOrNull(need("last_error"), "last_error"),
    park_kind: textOrNull(need("park_kind"), "park_kind"),
    wait_kind: textOrNull(need("wait_kind"), "wait_kind"),
    ready_condition: jsonValue(need("ready_condition"), "ready_condition"),
    required_budget_minutes: intOrNull(need("required_budget_minutes"), "required_budget_minutes"),
    last_checked_at: textOrNull(need("last_checked_at"), "last_checked_at"),
    check_outcome: textOrNull(need("check_outcome"), "check_outcome"),
    created_at: textOrNull(need("created_at"), "created_at") ?? "",
    updated_at: textOrNull(need("updated_at"), "updated_at") ?? "",
    completed_at: textOrNull(need("completed_at"), "completed_at"),
    blocked_deps: jsonValue(need("blocked_deps"), "blocked_deps"),
    dep_next_recheck_at: textOrNull(need("dep_next_recheck_at"), "dep_next_recheck_at"),
    dep_blocked_until: textOrNull(need("dep_blocked_until"), "dep_blocked_until"),
    dep_recheck_seconds: intOrNull(need("dep_recheck_seconds"), "dep_recheck_seconds"),
  };
}

export function readLedger(r: Record<string, unknown>): LedgerRow {
  const text = (col: string): string | null => textOrNull(r[col], col);
  return {
    id: intOrNull(r["id"], "id") ?? failWhere("ledger row has null id"),
    queue: text("queue") ?? "",
    task_id: intOrNull(r["task_id"], "task_id") ?? failWhere("ledger row has null task_id"),
    assigned_lane: text("assigned_lane"),
    executed_lane: text("executed_lane"),
    executed_lane_id: text("executed_lane_id"),
    exclusion_reason: text("exclusion_reason"),
    opencode_reason: text("opencode_reason"),
    platform_limit: text("platform_limit"),
    platform_limit_reason: text("platform_limit_reason"),
    assigned_by: text("assigned_by"),
    session_id: text("session_id"),
    fallback_reason: text("fallback_reason"),
    assigned_at: text("assigned_at"),
    executed_at: text("executed_at"),
    updated_at: text("updated_at"),
  };
}

export function readEvent(r: Record<string, unknown>): EventRow {
  return {
    id: intOrNull(r["id"], "id") ?? failWhere("event row has null id"),
    worker_id: textOrNull(r["worker_id"], "worker_id"),
    task_id: intOrNull(r["task_id"], "task_id"),
    kind: textOrNull(r["kind"], "kind") ?? "",
    detail: jsonValue(r["detail"], "detail"),
    created_at: textOrNull(r["created_at"], "created_at") ?? "",
  };
}

export interface LibSQLQueueStoreOpts {
  /** Clock for checkpoint updated_at (never read back through the
   *  interface; injected for deterministic tests). */
  nowIso?: () => string;
  /** What health reports: the Turso project source and netloc the
   *  connection serves (native health names project_netloc /
   *  project_source the same way). */
  projectSource?: string;
  netloc?: string | null;
}

/** The production QueueStore over an open embedded libSQL/SQLite
 *  connection holding the fleet-queue schema (file-backed local
 *  libSQL for tests/local dev; the same tables as the fleet-queue
 *  Turso DB, never a second queue). Every mutation that must be
 *  atomic is one conditional UPDATE — the single-writer serialization
 *  is the connection's own, exactly the guarantee the native door
 *  gets from libSQL. */
export class LibSQLQueueStore implements QueueStore {
  private readonly nowIso: () => string;
  private readonly projectSource: string;
  private readonly netloc: string | null;

  constructor(
    private readonly db: SyncSqlDb,
    opts: LibSQLQueueStoreOpts = {},
  ) {
    this.nowIso = opts.nowIso ?? (() => new Date().toISOString());
    this.projectSource = opts.projectSource ?? "fleet-queue";
    this.netloc = opts.netloc ?? null;
    for (const stmt of LIBSQL_QUEUE_SCHEMA) {
      db.exec(stmt);
    }
  }

  /** Rows the last statement changed (same-connection SELECT
   *  changes(), the embedded equivalent of the native affected). */
  private changes(): number {
    const rows = this.db.exec("SELECT changes() AS n").toArray();
    const n = rows[0]?.["n"];
    return typeof n === "number" ? n : typeof n === "bigint" ? Number(n) : 0;
  }

  async insertTask(row: Omit<TaskRow, "id">): Promise<TaskRow> {
    const rec = row as unknown as Record<string, unknown>;
    const cols = TASK_INSERT_COLS;
    const placeholders = cols.map(() => "?").join(", ");
    this.db.exec(
      `INSERT INTO tasks (${cols.join(", ")}) VALUES (${placeholders})`,
      ...cols.map((c) => encodeTaskValue(c, rec[c]) as string | number | bigint | boolean | null),
    );
    const got = this.db
      .exec(`SELECT ${TASK_COLS} FROM tasks WHERE id = last_insert_rowid()`)
      .toArray();
    if (got.length === 0) failWhere("insertTask lost the inserted row");
    return readTask(got[0] as Record<string, unknown>);
  }

  async getTask(id: number): Promise<TaskRow | null> {
    const rows = this.db.exec(`SELECT ${TASK_COLS} FROM tasks WHERE id = ?`, id).toArray();
    if (rows.length === 0) return null;
    return readTask(rows[0] as Record<string, unknown>);
  }

  async tasksByDedupe(queue: string, dedupeKey: string): Promise<TaskRow[]> {
    return this.db
      .exec(`SELECT ${TASK_COLS} FROM tasks WHERE queue = ? AND dedupe_key = ? ORDER BY id ASC`, queue, dedupeKey)
      .toArray()
      .map((r) => readTask(r as Record<string, unknown>));
  }

  async allTasks(): Promise<TaskRow[]> {
    return this.db
      .exec(`SELECT ${TASK_COLS} FROM tasks ORDER BY id ASC`)
      .toArray()
      .map((r) => readTask(r as Record<string, unknown>));
  }

  async conditionalUpdateTask(id: number, guard: Partial<TaskRow>, set: Partial<TaskRow>): Promise<boolean> {
    const grec = guard as Record<string, unknown>;
    const srec = set as Record<string, unknown>;
    const setCols = Object.keys(srec);
    if (setCols.length === 0) return this.getTask(id) !== null;
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
        // decide a claim race.
        conds.push(`json(${c}) = json(?)`);
        condArgs.push(JSON.stringify(v));
      }
    }
    this.db.exec(`UPDATE tasks SET ${setSql} WHERE ${conds.join(" AND ")}`, ...setArgs, ...condArgs);
    return this.changes() > 0;
  }

  async updateTask(id: number, set: Partial<TaskRow>): Promise<void> {
    const rec = set as Record<string, unknown>;
    const cols = Object.keys(rec);
    if (cols.length === 0) return;
    const setSql = cols.map((c) => `${c} = ?`).join(", ");
    this.db.exec(
      `UPDATE tasks SET ${setSql} WHERE id = ?`,
      ...cols.map((c) => encodeTaskValue(c, rec[c])),
      id,
    );
  }

  async getLedger(taskId: number): Promise<LedgerRow | null> {
    const rows = this.db.exec(`SELECT ${LEDGER_COLS} FROM repo_task_lane_ledger WHERE task_id = ?`, taskId).toArray();
    if (rows.length === 0) return null;
    return readLedger(rows[0] as Record<string, unknown>);
  }

  async insertLedger(row: Omit<LedgerRow, "id">): Promise<LedgerRow> {
    const rec = row as unknown as Record<string, unknown>;
    const placeholders = LEDGER_INSERT_COLS.map(() => "?").join(", ");
    this.db.exec(
      `INSERT INTO repo_task_lane_ledger (${LEDGER_INSERT_COLS.join(", ")}) VALUES (${placeholders})`,
      ...LEDGER_INSERT_COLS.map((c) => (c === "task_id" ? intOrNull(rec[c], c) : textOrNull(rec[c], c))),
    );
    const got = this.db
      .exec(`SELECT ${LEDGER_COLS} FROM repo_task_lane_ledger WHERE id = last_insert_rowid()`)
      .toArray();
    if (got.length === 0) failWhere("insertLedger lost the inserted row");
    return readLedger(got[0] as Record<string, unknown>);
  }

  async updateLedger(taskId: number, set: Partial<LedgerRow>): Promise<void> {
    const rec = set as Record<string, unknown>;
    const cols = Object.keys(rec);
    if (cols.length === 0) return;
    const setSql = cols.map((c) => `${c} = ?`).join(", ");
    this.db.exec(
      `UPDATE repo_task_lane_ledger SET ${setSql} WHERE task_id = ?`,
      ...cols.map((c) => (c === "task_id" ? intOrNull(rec[c], c) : textOrNull(rec[c], c))),
      taskId,
    );
  }

  async upsertLease(row: LeaseRow, opts: { keepWorker: boolean }): Promise<void> {
    if (opts.keepWorker) {
      // The heartbeat path (lifecycle.rs): the holder keeps its
      // worker; only the lease times move.
      this.db.exec(
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
      this.db.exec(
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
    this.db.exec("DELETE FROM leases WHERE task_id = ?", taskId);
  }

  async deleteExpiredLeases(nowIso: string): Promise<void> {
    this.db.exec("DELETE FROM leases WHERE lease_until < ?", nowIso);
  }

  async getLane(laneId: string): Promise<LaneRow | null> {
    const rows = this.db.exec("SELECT lane_id, worker_id, queue, enabled FROM lanes WHERE lane_id = ?", laneId).toArray();
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
    this.db.exec("INSERT INTO lanes (lane_id, worker_id, queue, enabled) VALUES (?, ?, ?, ?)", row.lane_id, row.worker_id, row.queue, row.enabled);
  }

  async touchWorker(workerId: string, working: boolean, nowIso: string): Promise<void> {
    // work_identity.rs touch_worker: unknown workers enter with an
    // empty role; the touch moves only status and heartbeat, never
    // role, display_name or created_at.
    this.db.exec(
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
    this.db.exec(
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

  async appendEvent(workerId: string | null, taskId: number | null, kind: string, detail: Json, createdAt: string): Promise<EventRow> {
    this.db.exec(
      "INSERT INTO events (worker_id, task_id, kind, detail, created_at) VALUES (?, ?, ?, ?, ?)",
      workerId,
      taskId,
      kind,
      toStoredJson(detail) ?? "{}",
      createdAt,
    );
    const got = this.db
      .exec("SELECT id, worker_id, task_id, kind, detail, created_at FROM events WHERE id = last_insert_rowid()")
      .toArray();
    if (got.length === 0) failWhere("appendEvent lost the inserted row");
    return readEvent(got[0] as Record<string, unknown>);
  }

  async allEvents(): Promise<EventRow[]> {
    return this.db
      .exec("SELECT id, worker_id, task_id, kind, detail, created_at FROM events ORDER BY id ASC")
      .toArray()
      .map((r) => readEvent(r as Record<string, unknown>));
  }

  async eventsForTask(taskId: number): Promise<EventRow[]> {
    return this.db
      .exec("SELECT id, worker_id, task_id, kind, detail, created_at FROM events WHERE task_id = ? ORDER BY id ASC", taskId)
      .toArray()
      .map((r) => readEvent(r as Record<string, unknown>));
  }

  async deleteEventsBefore(cutoffIso: string): Promise<number> {
    this.db.exec("DELETE FROM events WHERE created_at < ?", cutoffIso);
    return this.changes();
  }

  async mirrorLabels(repo: string, issueNumber: number): Promise<string[] | null> {
    // github_mirror_issues is the github-ci Worker's read model, not
    // this store's table: no row → null; a failing read (no table, no
    // database) throws so the caller warns and proceeds unenriched.
    const rows = this.db.exec("SELECT labels FROM github_mirror_issues WHERE repo = ? AND number = ?", repo, issueNumber).toArray();
    if (rows.length === 0) return null;
    const parsed = jsonValue(rows[0]["labels"], "labels");
    return Array.isArray(parsed) ? parsed.filter((l): l is string => typeof l === "string") : [];
  }

  async describe(): Promise<{ backend: string; projectSource: string; netloc: string | null }> {
    return { backend: "libsql", projectSource: this.projectSource, netloc: this.netloc };
  }
}
