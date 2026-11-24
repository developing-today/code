// The fleet queue's store as the broker makes it: a real SQLite with
// the broker's own tables, read from its source (turso_queue/core.rs)
// and from the GitHub mirror's (pkgs/github-ci/worker/mirror.sql), and
// answered in the store's HTTP shape (integers as text). Tests that
// file a task use it as the store, so a task is filed into the same
// tables, by the same columns, as the broker's.
import { Database } from "bun:sqlite";
import { readFileSync } from "node:fs";
import { join } from "node:path";

const ROOT = join(import.meta.dir, "../../..");
export const BROKER_SRC = join(ROOT, "pkgs/fleet/crates/fleet-queue/src/turso_queue");

/** A SQL string as the broker's Rust source has it: from the opening
 * quote that starts with `start` to its closing quote, with the line
 * continuations (`\` and the next line's indent) joined. */
export function rustSql(source: string, start: string): string {
  const at = source.indexOf(`"${start}`);
  if (at < 0) throw new Error(`not in the broker's source: ${start}`);
  const end = source.indexOf('"', at + 1);
  return source.slice(at + 1, end).replace(/\\\n\s*/g, "");
}

/** A store with the broker's tables (and the mirror's, unless not wanted). */
export function brokerDb(options: { mirror?: boolean } = {}): Database {
  const core = readFileSync(join(BROKER_SRC, "core.rs"), "utf8");
  const db = new Database(":memory:");
  for (const start of [
    "CREATE TABLE IF NOT EXISTS tasks (",
    "CREATE INDEX IF NOT EXISTS tasks_queue_dedupe",
    "CREATE INDEX IF NOT EXISTS tasks_claim_scan",
    "CREATE TABLE IF NOT EXISTS events (",
    "CREATE INDEX IF NOT EXISTS events_task",
  ]) {
    db.run(rustSql(core, start));
  }
  if (options.mirror !== false) {
    const mirror = readFileSync(join(ROOT, "pkgs/github-ci/worker/mirror.sql"), "utf8");
    const at = mirror.indexOf("CREATE TABLE IF NOT EXISTS github_mirror_issues");
    db.run(mirror.slice(at, mirror.indexOf(");", at) + 2));
  }
  return db;
}

/** A value as the queue's store sends it over HTTP. */
export const asTheStore = (value: unknown) =>
  value === null || value === undefined
    ? { type: "null" }
    : typeof value === "number" && Number.isInteger(value)
      ? { type: "integer", value: String(value) }
      : { type: "text", value: String(value) };

/** One statement against the store, answered as the store answers it
 * (or as the store's error). */
export function storeAnswer(db: Database, sql: string, args: { type: string; value?: string }[]) {
  const values = args.map((a) => (a.type === "null" ? null : a.type === "integer" ? Number(a.value) : a.value));
  try {
    const rows = db.query(sql).all(...(values as never[])) as Record<string, unknown>[];
    const cols = Object.keys(rows[0] ?? {}).map((name) => ({ name }));
    return {
      type: "ok",
      response: { result: { cols, rows: rows.map((row) => cols.map((c) => asTheStore(row[c.name]))) } },
    };
  } catch (error) {
    return { type: "error", error: { message: (error as Error).message } };
  }
}

/** Whether a statement is one that filing a task makes (src/filing.ts). */
export const isFiling = (sql: string) =>
  /^(INSERT INTO (tasks|events)|UPDATE tasks SET progress)\b/.test(sql) ||
  /^SELECT (id, status, priority(, completed_at)?|1 AS none|labels) FROM (tasks|github_mirror_issues)\b/.test(sql) ||
  sql === "SELECT 1 AS none WHERE 0";
