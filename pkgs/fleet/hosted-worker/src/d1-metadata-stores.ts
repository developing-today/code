// D1 homes for fleet-hosted's metadata store and backfill run store
// (#15456, a slice of the #14866 Cloudflare-first programme). Both
// sit behind their existing interfaces (MetadataStore in
// src/stamping.ts, BackfillRunStore in src/backfill.ts), so no route
// changes; buildDeps resolves them whenever the Worker's QUEUE_D1
// binding is present — the D1 database fleet-hosted is already bound
// to, which also holds the queue tables and fleet_sessions (#12584).
// Without the binding, the fixtures stay the fallback.
//
// Each row keeps the whole record as JSON beside the columns the
// store selects by, so a record reads back exactly as written. The
// DDL is CREATE TABLE IF NOT EXISTS, applied lazily on first use (the
// D1QueueStore posture), and the table names do not collide with the
// queue schema's. Ordering matches the fixtures: rows come back in
// insertion order (rowid), and a metadata re-insert of the same item
// updates its row in place, as the fixture's Map.set does.
//
// A row whose JSON does not decode to the record shape (hand-edited
// or foreign) fails closed rather than answering from data the
// service did not write.

import type { BackfillRunRecord, BackfillRunStore } from "./backfill.ts";
import { D1DatabaseClient, type D1Client, type D1DatabaseLike } from "./d1-queue-store.ts";
import { matchesMetadataFilter, type MetadataFilter, type MetadataRecord, type MetadataStore } from "./stamping.ts";

export const METADATA_TABLE_DDL = `CREATE TABLE IF NOT EXISTS fleet_metadata (
  repo TEXT NOT NULL,
  kind TEXT NOT NULL CHECK (kind IN ('issue', 'pr')),
  number INTEGER NOT NULL,
  binding_id TEXT NOT NULL,
  record_json TEXT NOT NULL,
  PRIMARY KEY (repo, kind, number)
)`;

export const METADATA_BINDING_INDEX_DDL =
  "CREATE INDEX IF NOT EXISTS fleet_metadata_binding ON fleet_metadata (binding_id)";

export const BACKFILL_RUNS_TABLE_DDL = `CREATE TABLE IF NOT EXISTS fleet_backfill_runs (
  seq INTEGER PRIMARY KEY AUTOINCREMENT,
  id TEXT NOT NULL,
  record_json TEXT NOT NULL
)`;

export const BACKFILL_RUNS_ID_INDEX_DDL =
  "CREATE INDEX IF NOT EXISTS fleet_backfill_runs_id ON fleet_backfill_runs (id)";

function fail(store: string, message: string): never {
  throw new Error(`${store} (d1): ${message}`);
}

function isObject(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

function parseJsonColumn(store: string, row: Record<string, unknown>): unknown {
  const raw = row["record_json"];
  if (typeof raw !== "string")
    fail(store, `column record_json is ${raw === null ? "null" : typeof raw}, expected text`);
  try {
    return JSON.parse(raw);
  } catch {
    return fail(store, "column record_json is not JSON");
  }
}

/** Decode one fleet_metadata row, refusing a record shape the store
 *  did not write. */
export function readMetadataRow(row: Record<string, unknown>): MetadataRecord {
  const store = "metadata store";
  const rec = parseJsonColumn(store, row);
  if (
    !isObject(rec) ||
    typeof rec["repo"] !== "string" ||
    (rec["kind"] !== "issue" && rec["kind"] !== "pr") ||
    !Number.isInteger(rec["number"]) ||
    typeof rec["url"] !== "string" ||
    typeof rec["bindingId"] !== "string" ||
    !isObject(rec["metadata"]) ||
    !isObject(rec["extracted"]) ||
    !Array.isArray(rec["extracted"]["links"]) ||
    typeof rec["insertedAt"] !== "string"
  ) {
    fail(store, "record_json is not a metadata record");
  }
  return rec as unknown as MetadataRecord;
}

/** Decode one fleet_backfill_runs row, refusing a record shape the
 *  store did not write. */
export function readBackfillRunRow(row: Record<string, unknown>): BackfillRunRecord {
  const store = "backfill run store";
  const rec = parseJsonColumn(store, row);
  if (
    !isObject(rec) ||
    typeof rec["id"] !== "string" ||
    !Array.isArray(rec["scope"]) ||
    typeof rec["startedAt"] !== "string" ||
    typeof rec["finishedAt"] !== "string" ||
    !isObject(rec["counts"]) ||
    !Array.isArray(rec["failures"])
  ) {
    fail(store, "record_json is not a backfill run record");
  }
  return rec as unknown as BackfillRunRecord;
}

/** Lazy one-time DDL for a store: the statements run in order on the
 *  first call, and every later call awaits the same promise. */
function lazySchema(client: D1Client, ddl: string[]): () => Promise<void> {
  let ready: Promise<void> | null = null;
  return () => {
    ready ??= (async () => {
      for (const sql of ddl) await client.execute({ sql, args: [] });
    })();
    return ready;
  };
}

export class D1MetadataStore implements MetadataStore {
  private readonly ready: () => Promise<void>;

  constructor(private readonly client: D1Client) {
    this.ready = lazySchema(client, [METADATA_TABLE_DDL, METADATA_BINDING_INDEX_DDL]);
  }

  async insert(record: MetadataRecord): Promise<void> {
    await this.ready();
    await this.client.execute({
      sql: `INSERT INTO fleet_metadata (repo, kind, number, binding_id, record_json) VALUES (?, ?, ?, ?, ?)
            ON CONFLICT (repo, kind, number) DO UPDATE SET
              binding_id = excluded.binding_id, record_json = excluded.record_json`,
      args: [record.repo, record.kind, record.number, record.bindingId, JSON.stringify(record)],
    });
  }

  async get(repo: string, kind: "issue" | "pr", number: number): Promise<MetadataRecord | null> {
    await this.ready();
    const res = await this.client.execute({
      sql: "SELECT record_json FROM fleet_metadata WHERE repo = ? AND kind = ? AND number = ?",
      args: [repo, kind, number],
    });
    const row = res.rows[0];
    return row ? readMetadataRow(row) : null;
  }

  async list(bindingId: string, filter: MetadataFilter): Promise<MetadataRecord[]> {
    await this.ready();
    const res = await this.client.execute({
      sql: "SELECT record_json FROM fleet_metadata WHERE binding_id = ? ORDER BY rowid",
      args: [bindingId],
    });
    return res.rows.map(readMetadataRow).filter((record) => matchesMetadataFilter(record, filter));
  }

  async listRepo(repo: string): Promise<MetadataRecord[]> {
    await this.ready();
    const res = await this.client.execute({
      sql: "SELECT record_json FROM fleet_metadata WHERE repo = ? ORDER BY rowid",
      args: [repo],
    });
    return res.rows.map(readMetadataRow);
  }
}

export class D1BackfillRunStore implements BackfillRunStore {
  private readonly ready: () => Promise<void>;

  constructor(private readonly client: D1Client) {
    this.ready = lazySchema(client, [BACKFILL_RUNS_TABLE_DDL, BACKFILL_RUNS_ID_INDEX_DDL]);
  }

  async record(run: BackfillRunRecord): Promise<void> {
    await this.ready();
    await this.client.execute({
      sql: "INSERT INTO fleet_backfill_runs (id, record_json) VALUES (?, ?)",
      args: [run.id, JSON.stringify(run)],
    });
  }

  async list(): Promise<BackfillRunRecord[]> {
    await this.ready();
    const res = await this.client.execute({
      sql: "SELECT record_json FROM fleet_backfill_runs ORDER BY seq",
      args: [],
    });
    return res.rows.map(readBackfillRunRow);
  }

  async get(id: string): Promise<BackfillRunRecord | null> {
    await this.ready();
    const res = await this.client.execute({
      sql: "SELECT record_json FROM fleet_backfill_runs WHERE id = ? ORDER BY seq LIMIT 1",
      args: [id],
    });
    const row = res.rows[0];
    return row ? readBackfillRunRow(row) : null;
  }
}

/** Open both stores over a bound D1 database (the Worker's
 *  production path; tests inject a D1DatabaseLike double). */
export function openD1MetadataStore(db: D1DatabaseLike): D1MetadataStore {
  return new D1MetadataStore(new D1DatabaseClient(db));
}

export function openD1BackfillRunStore(db: D1DatabaseLike): D1BackfillRunStore {
  return new D1BackfillRunStore(new D1DatabaseClient(db));
}
