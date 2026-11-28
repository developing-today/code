// Migration snapshot and staged target (#3404 slice 3).
//
// A migration copies one user object to a new home in another
// jurisdiction. The source hands out a snapshot only once its fence
// (slice 1) stands for that migration, so the snapshot is everything
// the source committed and nothing can commit after it. The target
// imports the snapshot into a fresh object that stays non-serving
// ("staged") until the coordinator activates it after the directory
// switch (slice 2). Both sides compute the same digest over the same
// canonical form, so the target proves it holds exactly what the
// source exported.
//
// What moves: the user's data tables (KV, blob references, and the
// bridge chain store). What does not: the object's identity and
// bookkeeping meta (the target binds its own, in its own
// jurisdiction), live bridge sessions (they end at the freeze), and
// blob bytes, which live in the jurisdiction's R2 bucket and are
// copied by the coordinator; activation checks every referenced blob
// is present in the target bucket before the target serves.

import { FailClosed } from "./routing.ts";
import { isMigrationId } from "./migration-fence.ts";

/** The tables a migration carries, in a fixed order. */
export const MIGRATED_TABLES = [
  "kv",
  "blobs",
  "bridge_events",
  "bridge_chains",
  "bridge_forks",
  "bridge_proof_ids",
  "bridge_proof_headers",
  "bridge_waiting",
  "bridge_waiting_by_dep",
  "bridge_meta",
] as const;
export type MigratedTable = (typeof MIGRATED_TABLES)[number];

/** Largest snapshot body the target reads. */
export const MAX_SNAPSHOT_BYTES = 96 * 1024 * 1024;

/** One cell: SQL NULL, an integer, text, or bytes as base64. */
export type SnapshotCell = null | number | string | { b64: string };
/** One row: column name to cell, keys sorted. */
export type SnapshotRow = Record<string, SnapshotCell>;

export interface MigrationSnapshot {
  v: 1;
  migration_id: string;
  routing_epoch: number;
  binding_id: string;
  client_side_encryption: boolean;
  schema_version: number;
  /** The source's mutation count at the freeze. */
  watermark: number;
  tables: Record<MigratedTable, SnapshotRow[]>;
  /** SHA-256 hex over the canonical form (see snapshotDigest). */
  digest: string;
}

/** The staged-target record kept in the target's meta. */
export interface StagedTarget {
  v: 1;
  migration_id: string;
  routing_epoch: number;
  digest: string;
  watermark: number;
}

interface SqlLike {
  exec(query: string, ...params: (string | number | null | Uint8Array)[]): { toArray(): Record<string, unknown>[] };
}

import { base64ToBytes, bytesToBase64 } from "../../../hosted-primitives/src/encoding.ts";

const COLUMN = /^[a-z_][a-z0-9_]{0,63}$/;

function encodeCell(value: unknown): SnapshotCell {
  if (value === null || typeof value === "string") return value;
  if (typeof value === "number" && Number.isSafeInteger(value)) return value;
  if (
    typeof value === "bigint" &&
    value >= BigInt(Number.MIN_SAFE_INTEGER) &&
    value <= BigInt(Number.MAX_SAFE_INTEGER)
  ) {
    return Number(value);
  }
  if (value instanceof Uint8Array) return { b64: bytesToBase64(value) };
  if (value instanceof ArrayBuffer) return { b64: bytesToBase64(new Uint8Array(value)) };
  throw new FailClosed(500, "snapshot_unencodable", "a stored cell is not an integer, text, bytes or null");
}

function encodeRow(row: Record<string, unknown>): SnapshotRow {
  const out: SnapshotRow = {};
  for (const key of Object.keys(row).sort()) out[key] = encodeCell(row[key]);
  return out;
}

/** Canonical JSON: object keys sorted at every level. */
function canonicalJson(value: unknown): string {
  if (value === null || typeof value !== "object") return JSON.stringify(value);
  if (Array.isArray(value)) return `[${value.map(canonicalJson).join(",")}]`;
  const obj = value as Record<string, unknown>;
  return `{${Object.keys(obj)
    .sort()
    .map((k) => `${JSON.stringify(k)}:${canonicalJson(obj[k])}`)
    .join(",")}}`;
}

/** Read the migrated tables, each row sorted by its canonical form so
 * both sides order them the same however they were inserted. */
export function readMigratedTables(sql: SqlLike): Record<MigratedTable, SnapshotRow[]> {
  const tables = {} as Record<MigratedTable, SnapshotRow[]>;
  for (const table of MIGRATED_TABLES) {
    const rows = sql.exec(`SELECT * FROM ${table}`).toArray().map(encodeRow);
    tables[table] = rows
      .map((row) => ({ row, key: canonicalJson(row) }))
      .sort((a, b) => (a.key < b.key ? -1 : a.key > b.key ? 1 : 0))
      .map(({ row }) => row);
  }
  return tables;
}

/** Whether every migrated table is empty. */
export function migratedTablesEmpty(sql: SqlLike): boolean {
  return MIGRATED_TABLES.every((t) => sql.exec(`SELECT 1 AS x FROM ${t} LIMIT 1`).toArray().length === 0);
}

/** The digest both sides compute: SHA-256 over the canonical JSON of
 * the identity the data belongs to and the tables themselves. */
export async function snapshotDigest(
  bindingId: string,
  clientSideEncryption: boolean,
  schemaVersion: number,
  tables: Record<MigratedTable, SnapshotRow[]>,
): Promise<string> {
  const text = canonicalJson({
    binding_id: bindingId,
    client_side_encryption: clientSideEncryption,
    schema_version: schemaVersion,
    tables,
  });
  const digest = new Uint8Array(await crypto.subtle.digest("SHA-256", new TextEncoder().encode(text)));
  return Array.from(digest, (b) => b.toString(16).padStart(2, "0")).join("");
}

function invalid(message: string): never {
  throw new FailClosed(400, "invalid_snapshot", message);
}

function isCell(value: unknown): value is SnapshotCell {
  if (value === null || typeof value === "string") return true;
  if (typeof value === "number") return Number.isSafeInteger(value);
  if (typeof value === "object" && value !== null && !Array.isArray(value)) {
    const keys = Object.keys(value);
    return keys.length === 1 && keys[0] === "b64" && typeof (value as { b64: unknown }).b64 === "string";
  }
  return false;
}

/** Parse an imported snapshot's shape. The digest is checked
 * separately, against a recomputation. */
export function parseSnapshot(body: unknown): MigrationSnapshot {
  if (typeof body !== "object" || body === null || Array.isArray(body)) invalid("snapshot must be a JSON object");
  const s = body as Record<string, unknown>;
  if (s.v !== 1) invalid("unsupported snapshot version");
  if (!isMigrationId(s.migration_id)) invalid("migration_id is malformed");
  if (typeof s.routing_epoch !== "number" || !Number.isSafeInteger(s.routing_epoch) || s.routing_epoch < 1) {
    invalid("routing_epoch must be a positive integer");
  }
  if (typeof s.binding_id !== "string") invalid("binding_id is missing");
  if (typeof s.client_side_encryption !== "boolean") invalid("client_side_encryption is missing");
  if (typeof s.schema_version !== "number" || !Number.isSafeInteger(s.schema_version))
    invalid("schema_version is missing");
  if (typeof s.watermark !== "number" || !Number.isSafeInteger(s.watermark) || s.watermark < 0) {
    invalid("watermark must be a non-negative integer");
  }
  if (typeof s.digest !== "string" || !/^[0-9a-f]{64}$/.test(s.digest)) invalid("digest must be 64 hex digits");
  if (typeof s.tables !== "object" || s.tables === null || Array.isArray(s.tables)) invalid("tables must be an object");
  const tables = s.tables as Record<string, unknown>;
  for (const name of Object.keys(tables)) {
    if (!(MIGRATED_TABLES as readonly string[]).includes(name)) invalid(`unknown table ${name}`);
  }
  for (const name of MIGRATED_TABLES) {
    const rows = tables[name];
    if (!Array.isArray(rows)) invalid(`table ${name} is missing`);
    for (const row of rows) {
      if (typeof row !== "object" || row === null || Array.isArray(row)) invalid(`table ${name} has a non-object row`);
      for (const [column, cell] of Object.entries(row as Record<string, unknown>)) {
        if (!COLUMN.test(column)) invalid(`table ${name} has a malformed column name`);
        if (!isCell(cell)) invalid(`table ${name} has a malformed cell`);
      }
    }
  }
  return s as unknown as MigrationSnapshot;
}

/** Insert a parsed snapshot's rows. Column names were checked by
 * parseSnapshot and table names come from the fixed list. */
export function writeMigratedTables(sql: SqlLike, tables: Record<MigratedTable, SnapshotRow[]>): void {
  for (const table of MIGRATED_TABLES) {
    for (const row of tables[table]) {
      const columns = Object.keys(row);
      if (columns.length === 0) continue;
      const values = columns.map((c) => {
        const cell = row[c];
        return cell !== null && typeof cell === "object" ? base64ToBytes(cell.b64) : cell;
      });
      sql.exec(`INSERT INTO ${table} (${columns.join(", ")}) VALUES (${columns.map(() => "?").join(", ")})`, ...values);
    }
  }
}

/** Remove every migrated row (a discarded or failed import). */
export function clearMigratedTables(sql: SqlLike): void {
  for (const table of MIGRATED_TABLES) sql.exec(`DELETE FROM ${table}`);
}

/** Read the staged-target record; a corrupt one fails closed. */
export function readStagedTarget(raw: string | null): StagedTarget | null {
  if (raw === null) return null;
  try {
    const t = JSON.parse(raw) as StagedTarget;
    if (
      t.v === 1 &&
      isMigrationId(t.migration_id) &&
      Number.isSafeInteger(t.routing_epoch) &&
      t.routing_epoch >= 1 &&
      typeof t.digest === "string" &&
      /^[0-9a-f]{64}$/.test(t.digest) &&
      Number.isSafeInteger(t.watermark)
    ) {
      return t;
    }
  } catch {
    // fall through to the refusal
  }
  throw new FailClosed(500, "staged_target_corrupt", "the staged-target record is unreadable; failing closed");
}
