/**
 * bun:sqlite adapter for the tenant store's SqlDb surface — the
 * test/local-tooling backend. A tenant DB is SQLite-shaped wherever
 * it lives (Turso/libSQL speaks the same dialect), so the migration
 * suite runs against this adapter unchanged. Foreign keys are ON:
 * the schema's references are part of the contract, not decoration.
 */
import { Database } from "bun:sqlite";
import type { SqlDb } from "./migrate.ts";

export function bunSqliteDb(db: Database): SqlDb {
  return {
    exec: (sql) => db.exec(sql),
    run: (sql, params = []) => {
      db.query(sql).run(...(params as (string | number | null)[]));
    },
    all: <T>(sql: string, params: readonly (string | number | null)[] = []) =>
      db.query(sql).all(...(params as (string | number | null)[])) as T[],
    transaction: <T>(fn: () => T): T => db.transaction(fn)(),
  };
}

/** Open a tenant DB at `path` (":memory:" for tests) with its SqlDb adapter. */
export function openTenantDb(path = ":memory:"): { db: Database; sql: SqlDb } {
  const db = new Database(path);
  db.exec("PRAGMA foreign_keys = ON");
  return { db, sql: bunSqliteDb(db) };
}
