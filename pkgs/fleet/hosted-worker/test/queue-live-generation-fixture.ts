// The live-generation queue schema fixture (#14985, CF-2a of
// #14942): the queue tables exactly as the live fleet-queue plane
// carries them, so the parity suites can prove the stores run
// against a database shaped like the one the CF-2 cutover actually
// faces — not only against a database the current DDL creates.
//
// Provenance: read back from the deployed D1 database `fleet-queue`
// (sqlite_master, 2026-11-25), which holds the 2026-11-25T07:36:01Z
// export of the live Turso plane (#14942 step 1 parity record).
// The DDL below is that readback, reflowed; column sets, column
// order and defaults are the readback's own.
//
// The generation gap against LIBSQL_QUEUE_SCHEMA (the shared
// schema in src/libsql-queue-store.ts):
//  - tasks.manual_only — ABSENT here. The stores read and write it
//    on every task row.
//  - tasks.metadata — ABSENT here, same consequence.
//  - tasks.max_attempts — DEFAULT 4 here, DEFAULT 1000 in the
//    shared schema.
//  - The native-only tasks columns (labels, labels_synced_at,
//    resolution*, resolved_*) are absent here too; the TS stores
//    never reference them (determination below).
// Every other queue table and index matches the shared schema's
// column set (the ledger's column order differs; order is not
// load-bearing — the stores name columns explicitly everywhere).
//
// The migration path (recorded per #14985's acceptance): the
// owning schema paths migrate additively at open, idempotently —
// each TS store's schema init (LibSQLQueueStore's constructor,
// D1QueueStore.ready(), HranaQueueStore.ready()) runs the shared
// IF NOT EXISTS DDL, then ALTERs in manual_only and metadata when
// pragma_table_info shows them missing; the native broker's
// ensure_schema (pkgs/fleet/crates/fleet-queue/src/turso_queue/
// core.rs) does the same for its full column set. No export/import
// column-mapping step exists or is needed: an exported database
// becomes a database the stores speak the first time any store
// opens it, and the suites in live-generation-queue-parity.test.ts
// pin that this cannot silently stop being true.
//
// Determinations recorded for #14985:
//  - max_attempts default: NOT aligned by table rebuild. SQLite
//    has no ALTER COLUMN for a default; rebuilding the live tasks
//    table from a store-open path is destructive-class machinery
//    against the production queue for a default no store path
//    relies on — every store (the three TS stores and the native
//    broker) supplies max_attempts explicitly on insert, and the
//    row readers coalesce a stored NULL to 1000. Fresh databases
//    get DEFAULT 1000 from the shared DDL. The parity suite proves
//    the behavioural alignment: a task enqueued through a store
//    against this DEFAULT 4 table carries max_attempts 1000, while
//    a pre-existing exported row keeps its stored value untouched.
//  - Native-only columns: the native broker does NOT need them
//    present before cutover. Its ensure_schema adds labels,
//    labels_synced_at and the resolution*/resolved_* columns
//    additively at open, by the same mechanism as manual_only —
//    so whichever door opens the cut-over database first, the
//    columns its owner needs land then. The TS stores deliberately
//    do not add them: they are not the TS stores' columns, and the
//    hosted door adding native-owned columns would fork schema
//    ownership. The parity suite asserts they stay absent after a
//    TS store has opened and worked an export-shaped database.

/** The queue tables as the live plane carries them (see header):
 *  same statement order as LIBSQL_QUEUE_SCHEMA, live shapes. */
export const LIVE_GENERATION_QUEUE_SCHEMA: string[] = [
  `CREATE TABLE tasks (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    queue TEXT NOT NULL, title TEXT, body TEXT,
    payload TEXT NOT NULL DEFAULT '{}', dedupe_key TEXT,
    priority INTEGER, status TEXT NOT NULL DEFAULT 'queued',
    not_before TEXT, worker_id TEXT, claimed_at TEXT, lease_until TEXT,
    lane_id TEXT, attempts INTEGER NOT NULL DEFAULT 0,
    max_attempts INTEGER NOT NULL DEFAULT 4,
    progress TEXT NOT NULL DEFAULT '{}', result TEXT NOT NULL DEFAULT '{}',
    last_error TEXT, park_kind TEXT, wait_kind TEXT, ready_condition TEXT,
    last_checked_at TEXT, check_outcome TEXT,
    created_at TEXT NOT NULL, updated_at TEXT NOT NULL,
    completed_at TEXT,
    platform TEXT, claim_provider TEXT, claim_model TEXT, claim_variant TEXT,
    blocked_deps TEXT, dep_next_recheck_at TEXT, dep_blocked_until TEXT,
    dep_recheck_seconds INTEGER, claim_session_id TEXT, required_budget_minutes INTEGER)`,
  `CREATE INDEX tasks_queue_dedupe
    ON tasks(queue, dedupe_key) WHERE dedupe_key IS NOT NULL`,
  `CREATE INDEX tasks_claim_scan
    ON tasks(queue, status, priority, created_at)`,
  `CREATE TABLE leases (
    task_id INTEGER PRIMARY KEY, worker_id TEXT NOT NULL,
    lease_until TEXT NOT NULL, heartbeat_at TEXT NOT NULL)`,
  `CREATE TABLE lanes (
    lane_id TEXT PRIMARY KEY, worker_id TEXT NOT NULL,
    queue TEXT NOT NULL, enabled INTEGER NOT NULL DEFAULT 1)`,
  `CREATE TABLE workers (
    worker_id TEXT PRIMARY KEY, role TEXT NOT NULL DEFAULT '',
    display_name TEXT, status TEXT NOT NULL DEFAULT 'idle',
    last_heartbeat_at TEXT, created_at TEXT)`,
  `CREATE TABLE checkpoints (
    worker_id TEXT NOT NULL, "key" TEXT NOT NULL,
    value TEXT NOT NULL DEFAULT '{}', updated_at TEXT NOT NULL,
    PRIMARY KEY (worker_id, "key"))`,
  `CREATE TABLE repo_task_lane_ledger (
    id INTEGER PRIMARY KEY AUTOINCREMENT, queue TEXT NOT NULL,
    task_id INTEGER NOT NULL UNIQUE, assigned_lane TEXT, executed_lane TEXT,
    exclusion_reason TEXT, assigned_by TEXT, session_id TEXT,
    fallback_reason TEXT, assigned_at TEXT, executed_at TEXT,
    updated_at TEXT NOT NULL, opencode_reason TEXT, platform_limit TEXT,
    platform_limit_reason TEXT, executed_lane_id TEXT)`,
  `CREATE TABLE events (
    id INTEGER PRIMARY KEY AUTOINCREMENT, worker_id TEXT, task_id INTEGER,
    kind TEXT NOT NULL, detail TEXT NOT NULL DEFAULT '{}',
    created_at TEXT NOT NULL)`,
  `CREATE INDEX events_task ON events(task_id, id)`,
  `CREATE INDEX events_kind_ts ON events(kind, created_at)`,
  `CREATE INDEX events_created ON events(created_at)`,
];
