// The D1 binding double shared by the D1 store tests: the documented
// D1Database API (prepare → bind → all/run, meta carrying changes +
// last_row_id) over a real node:sqlite file, so the production
// D1DatabaseClient is what the tests exercise — the same double
// d1-queue-parity.test.ts uses for D1QueueStore.

import { mkdtempSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { DatabaseSync } from "node:sqlite";
import type { D1DatabaseLike } from "../src/d1-queue-store.ts";

class FakeD1PreparedStatement {
  private values: unknown[] = [];

  constructor(
    private readonly db: DatabaseSync,
    private readonly sqlText: string,
  ) {}

  bind(...values: unknown[]): FakeD1PreparedStatement {
    this.values = values;
    return this;
  }

  async all(): Promise<{ results: Record<string, unknown>[]; meta: { changes: number; last_row_id: number } }> {
    const rows = this.db.prepare(this.sqlText).all(...(this.values as never[])) as Record<string, unknown>[];
    return { results: rows, meta: { changes: 0, last_row_id: 0 } };
  }

  async run(): Promise<{ meta: { changes: number; last_row_id: number } }> {
    const info = this.db.prepare(this.sqlText).run(...(this.values as never[])) as {
      changes: number | bigint;
      lastInsertRowid: number | bigint;
    };
    return { meta: { changes: Number(info.changes), last_row_id: Number(info.lastInsertRowid) } };
  }
}

export class FakeD1Database implements D1DatabaseLike {
  constructor(readonly db: DatabaseSync) {}

  prepare(sqlText: string): FakeD1PreparedStatement {
    return new FakeD1PreparedStatement(this.db, sqlText);
  }
}

/** Reopen the actual file, closing the previous connection. No JS store or
 * connection is reused after restart; reads use only SQLite's committed bytes. */
export function withRestartableDb(fn: (restart: () => FakeD1Database) => Promise<void>): () => Promise<void> {
  return async () => {
    const dir = mkdtempSync(join(tmpdir(), "fleet-d1-restart-"));
    let db: DatabaseSync | undefined;
    const restart = () => {
      db?.close();
      db = new DatabaseSync(join(dir, "fleet-hosted.db"));
      return new FakeD1Database(db);
    };
    try {
      await fn(restart);
    } finally {
      db?.close();
      rmSync(dir, { recursive: true, force: true });
    }
  };
}

export function withDb(fn: (d1: FakeD1Database) => Promise<void>): () => Promise<void> {
  return async () => {
    const dir = mkdtempSync(join(tmpdir(), "fleet-d1-stores-"));
    const db = new DatabaseSync(join(dir, "fleet-hosted.db"));
    try {
      await fn(new FakeD1Database(db));
    } finally {
      db.close();
      rmSync(dir, { recursive: true, force: true });
    }
  };
}
