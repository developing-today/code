// The D1 SessionRegistry (#12584's production wiring): the hosted
// session registry's rows in the fleet-hosted Worker's bound D1
// database, behind the same SessionRegistry interface the fixture
// implements (src/sessions.ts), so no route changes. buildDeps
// resolves it whenever the QUEUE_D1 binding is present — the one
// D1 database fleet-hosted is bound to (wrangler.toml) — so session
// records outlive the isolate instead of vanishing with it.
//
// The table is fleet_sessions: one row per server-minted session id,
// keyed by that id, carrying exactly the SessionRecord fields
// (lane-start issues as a JSON array of the refs verbatim). DDL runs
// lazily on first use (CREATE TABLE IF NOT EXISTS), the same posture
// as D1QueueStore: against a fresh database it creates the table,
// against a provisioned one it is a no-op. The table name does not
// collide with the queue schema's tables, so the two stores share
// the database without a schema fork.
//
// Storage semantics match the fixture exactly: insert refuses a
// duplicate id (the INSERT's primary key does), get returns a fresh
// copy or null, and markClosed sets closed_at and returns the
// updated record, or null for an unknown id. A row that fails to
// decode (a hand-edited or foreign-shaped row) fails closed rather
// than resolving a session from data the service did not write.

import { D1DatabaseClient, type D1Client, type D1DatabaseLike } from "./d1-queue-store.ts";
import type { SessionRecord, SessionRegistry } from "./sessions.ts";

export const SESSION_TABLE_DDL = `CREATE TABLE IF NOT EXISTS fleet_sessions (
  session_id TEXT PRIMARY KEY NOT NULL,
  binding_id TEXT NOT NULL,
  lane TEXT,
  model TEXT,
  opened_at TEXT NOT NULL,
  closed_at TEXT,
  lane_start_issues TEXT NOT NULL DEFAULT '[]'
)`;

const SESSION_COLS = "session_id, binding_id, lane, model, opened_at, closed_at, lane_start_issues";

function fail(message: string): never {
  throw new Error(`session registry (d1): ${message}`);
}

function textOrNull(row: Record<string, unknown>, col: string): string | null {
  const value = row[col];
  if (value === null || value === undefined) return null;
  if (typeof value !== "string") fail(`column ${col} is ${typeof value}, expected text or null`);
  return value;
}

function text(row: Record<string, unknown>, col: string): string {
  return textOrNull(row, col) ?? fail(`column ${col} is null, expected text`);
}

/** Decode one fleet_sessions row into a SessionRecord, refusing any
 *  row whose shape the registry did not write. */
export function readSessionRow(row: Record<string, unknown>): SessionRecord {
  const rawIssues = text(row, "lane_start_issues");
  let issues: unknown;
  try {
    issues = JSON.parse(rawIssues);
  } catch {
    fail("column lane_start_issues is not JSON");
  }
  if (!Array.isArray(issues) || !issues.every((ref) => typeof ref === "string")) {
    fail("column lane_start_issues is not a JSON array of strings");
  }
  return {
    sessionId: text(row, "session_id"),
    bindingId: text(row, "binding_id"),
    lane: textOrNull(row, "lane"),
    model: textOrNull(row, "model"),
    openedAt: text(row, "opened_at"),
    closedAt: textOrNull(row, "closed_at"),
    laneStartIssues: issues as string[],
  };
}

export class D1SessionRegistry implements SessionRegistry {
  private schemaReady: Promise<void> | null = null;

  constructor(private readonly client: D1Client) {}

  private ready(): Promise<void> {
    this.schemaReady ??= this.client.execute({ sql: SESSION_TABLE_DDL, args: [] }).then(() => undefined);
    return this.schemaReady;
  }

  async insert(record: SessionRecord): Promise<void> {
    await this.ready();
    if (await this.get(record.sessionId)) {
      fail(`duplicate session id ${record.sessionId}`);
    }
    await this.client.execute({
      sql: `INSERT INTO fleet_sessions (${SESSION_COLS}) VALUES (?, ?, ?, ?, ?, ?, ?)`,
      args: [
        record.sessionId,
        record.bindingId,
        record.lane,
        record.model,
        record.openedAt,
        record.closedAt,
        JSON.stringify(record.laneStartIssues),
      ],
    });
  }

  async get(sessionId: string): Promise<SessionRecord | null> {
    await this.ready();
    const res = await this.client.execute({
      sql: `SELECT ${SESSION_COLS} FROM fleet_sessions WHERE session_id = ?`,
      args: [sessionId],
    });
    const row = res.rows[0];
    return row ? readSessionRow(row) : null;
  }

  async markClosed(sessionId: string, closedAt: string): Promise<SessionRecord | null> {
    await this.ready();
    const res = await this.client.execute({
      sql: "UPDATE fleet_sessions SET closed_at = ? WHERE session_id = ?",
      args: [closedAt, sessionId],
    });
    if (res.affectedRowCount === 0) return null;
    return this.get(sessionId);
  }
}

/** Open the registry over a bound D1 database (the Worker's
 *  production path; tests inject a D1DatabaseLike double). */
export function openD1SessionRegistry(db: D1DatabaseLike): D1SessionRegistry {
  return new D1SessionRegistry(new D1DatabaseClient(db));
}
