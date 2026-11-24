/**
 * Migration runner for the PM tenant store (#12825).
 *
 * Applies numbered SQL migrations to ONE tenant's Turso DB (component
 * E tenant plane) in version order, each in its own transaction, and
 * records every applied migration in `pm_schema_migrations` — that
 * table is the store's schema-version record. Re-running is a no-op
 * for already-applied versions; a version recorded with a different
 * name than the migration on disk is drift and fails loudly.
 *
 * The runner is runtime-agnostic: it talks to the minimal SqlDb
 * surface below. `sqlite.ts` adapts bun:sqlite for tests and local
 * tooling; the hosted service will adapt its libSQL client the same
 * way. This module never sees a database URL or token (E §2.3: no
 * tokens in any DB, and none in this code path either).
 */

export interface SqlDb {
  exec(sql: string): void;
  run(sql: string, params?: readonly (string | number | null)[]): void;
  all<T>(sql: string, params?: readonly (string | number | null)[]): T[];
  transaction<T>(fn: () => T): T;
}

export interface Migration {
  version: number;
  name: string;
  sql: string;
}

interface AppliedRow {
  version: number;
  name: string;
}

const BOOKKEEPING_DDL = `CREATE TABLE IF NOT EXISTS pm_schema_migrations (
  version INTEGER PRIMARY KEY,
  name TEXT NOT NULL,
  applied_at TEXT NOT NULL
)`;

export function applyMigrations(
  db: SqlDb,
  migrations: readonly Migration[],
  now: () => string = () => new Date().toISOString(),
): { applied: number[]; current: number[] } {
  const ordered = [...migrations].sort((a, b) => a.version - b.version);
  const seen = new Set<number>();
  for (const migration of ordered) {
    if (!Number.isInteger(migration.version) || migration.version < 1)
      throw new Error(`invalid migration version: ${migration.version}`);
    if (seen.has(migration.version)) throw new Error(`duplicate migration version: ${migration.version}`);
    seen.add(migration.version);
  }

  db.exec(BOOKKEEPING_DDL);
  const recorded = new Map(
    db.all<AppliedRow>("SELECT version, name FROM pm_schema_migrations ORDER BY version").map((row) => [
      row.version,
      row.name,
    ]),
  );

  const applied: number[] = [];
  for (const migration of ordered) {
    const recordedName = recorded.get(migration.version);
    if (recordedName !== undefined) {
      if (recordedName !== migration.name)
        throw new Error(
          `migration drift: version ${migration.version} is recorded as "${recordedName}" but the file on disk is "${migration.name}"`,
        );
      continue;
    }
    db.transaction(() => {
      db.exec(migration.sql);
      db.run("INSERT INTO pm_schema_migrations (version, name, applied_at) VALUES (?, ?, ?)", [
        migration.version,
        migration.name,
        now(),
      ]);
    });
    applied.push(migration.version);
  }
  return { applied, current: currentSchemaVersion(db) };
}

/** Recorded schema version of a tenant DB: the highest applied migration, or [] when none has run. */
export function currentSchemaVersion(db: SqlDb): number[] {
  db.exec(BOOKKEEPING_DDL);
  return db
    .all<{ version: number }>("SELECT version FROM pm_schema_migrations ORDER BY version")
    .map((row) => row.version);
}
