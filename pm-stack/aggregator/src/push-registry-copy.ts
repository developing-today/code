// Push registry copy + hash-verify for the CF-5e (#15561) D1 refresh.
//
// The five registry tables move Turso fleet-platform -> D1 fleet-platform
// (#14968 copy pattern: counts + hashes, evidence recorded on the issue).
// copyPushRegistry reads every row of a table (keyset-paginated on the
// registry key) and reproduces it in the destination; the rows travel
// through the driver-facing SqlClient contract, so the same code runs
// unchanged against the Turso transport and the D1 HTTP transport.
// hashPushRegistry fingerprints each table as a SHA-256 over canonical
// per-row value vectors; verifyPushRegistryCopy compares source and
// destination fingerprints. The source is read-only; the copy is a
// delete+insert refresh run by the operator command, never by the live
// sender.
import { createHash } from "node:crypto";
import { PUSH_REGISTRY_TABLES } from "./push-registry.ts";
import type { SqlClient } from "./sql-client.ts";
import { sqlStr } from "./sql-client.ts";

export interface PushRegistryCopyTable {
  table: string;
  key: string;
  columns: readonly string[];
}

/** Copy metadata for every registry table, pinned by test to the exact
 * PUSH_REGISTRY_TABLES set. Column order is the insert order; a column
 * missing from the store fails the copy loudly, never silently. */
export const PUSH_REGISTRY_COPY_TABLES: readonly PushRegistryCopyTable[] = [
  {
    table: "push_endpoints",
    key: "id",
    columns: [
      "id",
      "transport",
      "client",
      "device_id",
      "endpoint",
      "p256dh",
      "auth",
      "label",
      "user_agent",
      "created_at",
      "last_seen_at",
      "gone_count",
    ],
  },
  {
    table: "push_preferences",
    key: "endpoint_id",
    columns: ["endpoint_id", "classes", "quiet", "delivery", "updated_at"],
  },
  {
    table: "push_item_stages",
    key: "item_ref",
    columns: ["item_ref", "stage", "priority", "blocked_by_count", "entered_stage_at", "updated_at"],
  },
  {
    table: "push_item_stage_events",
    key: "id",
    columns: ["id", "item_ref", "from_stage", "to_stage", "reason", "at"],
  },
  {
    table: "push_registry_meta",
    key: "key",
    columns: ["key", "value", "at"],
  },
];

for (const entry of PUSH_REGISTRY_COPY_TABLES) {
  if (PUSH_REGISTRY_TABLES[entry.table] !== entry.key || entry.columns[0] !== entry.key) {
    throw new Error(`push registry copy metadata disagrees with the registry contract: ${entry.table}`);
  }
}

/** Render a decoded row value as a SQLite literal. Rows arrive decoded
 * (JSON text columns parsed to values); JSON-ish values go back as plain
 * JSON text, never `::jsonb` — the D1/SQLite side has no cast syntax and
 * both transports share the same literal translation. */
function copyLiteral(value: unknown): string {
  if (value === null || value === undefined) return "null";
  if (typeof value === "number") {
    if (!Number.isInteger(value) || !Number.isSafeInteger(value)) {
      throw new Error("refusing non-integer number in registry copy");
    }
    return String(value);
  }
  if (typeof value === "boolean") return value ? "1" : "0";
  if (typeof value === "string") return sqlStr(value);
  return sqlStr(JSON.stringify(value));
}

async function readAllRows(
  client: SqlClient,
  table: string,
  key: string,
  pageSize = 500,
): Promise<Record<string, unknown>[]> {
  const rows: Record<string, unknown>[] = [];
  let cursor: string | number | null = null;
  for (;;) {
    const clause: string = cursor === null ? "" : `where ${key} > ${copyLiteral(cursor)} `;
    const page: Record<string, unknown>[] = await client.query<Record<string, unknown>>(
      `select * from ${table} ${clause}order by ${key} limit ${pageSize}`,
    );
    rows.push(...page);
    if (page.length < pageSize) break;
    const next: unknown = page[page.length - 1]?.[key];
    if (next === null || next === undefined || next === cursor) {
      throw new Error(`nonadvancing copy cursor for ${table}`);
    }
    if (typeof next !== "string" && typeof next !== "number") {
      throw new Error(`unusable copy cursor for ${table}`);
    }
    cursor = next;
  }
  return rows;
}

/**
 * Refresh the destination from the source: delete every row of each
 * registry table, then re-insert the source rows in key order. The DDL
 * (PUSH_REGISTRY_DDL) is idempotent and applied first so a fresh
 * destination learns the schema; on a seeded store it is a no-op.
 */
export async function copyPushRegistry(
  source: SqlClient,
  dest: SqlClient,
  ddl: readonly string[],
): Promise<Record<string, number>> {
  for (const statement of ddl) await dest.query(statement);
  const counts: Record<string, number> = {};
  for (const { table, columns, key } of PUSH_REGISTRY_COPY_TABLES) {
    const rows = await readAllRows(source, table, key);
    counts[table] = rows.length;
    await dest.query(`delete from ${table}`);
    for (const row of rows) {
      const values = columns.map((column) => copyLiteral(row[column])).join(", ");
      await dest.query(`insert into ${table} (${columns.join(", ")}) values (${values})`);
    }
  }
  return counts;
}

function canonicalValue(value: unknown): string {
  if (value === null || value === undefined) return "null";
  if (typeof value === "number") {
    if (!Number.isFinite(value)) throw new Error("refusing non-finite number in registry fingerprint");
    return `n:${value}`;
  }
  if (typeof value === "string") return `s:${value}`;
  if (typeof value === "boolean") return value ? "b:1" : "b:0";
  return `j:${canonicalJson(value)}`;
}

function canonicalJson(value: unknown): string {
  if (value === null || value === undefined) return "null";
  if (typeof value === "number") {
    if (!Number.isFinite(value)) throw new Error("refusing non-finite number in registry fingerprint");
    return String(value);
  }
  if (typeof value === "string") return JSON.stringify(value);
  if (typeof value === "boolean") return value ? "true" : "false";
  if (Array.isArray(value)) return `[${value.map(canonicalJson).join(",")}]`;
  if (value !== null && typeof value === "object") {
    const record = value as Record<string, unknown>;
    const keys = Object.keys(record).sort();
    return `{${keys.map((key) => `${JSON.stringify(key)}:${canonicalJson(record[key])}`).join(",")}}}`;
  }
  throw new Error("unsupported value in registry fingerprint");
}

export interface PushRegistryTableHash {
  table: string;
  count: number;
  hash: string;
}

/** Per-table count + SHA-256 over the canonical row vectors in key
 * order; identical data fingerprints identically on either transport. */
export async function hashPushRegistry(client: SqlClient): Promise<PushRegistryTableHash[]> {
  const out: PushRegistryTableHash[] = [];
  for (const { table, columns, key } of PUSH_REGISTRY_COPY_TABLES) {
    const rows = await readAllRows(client, table, key);
    const hasher = createHash("sha256");
    hasher.update(`${table}|${key}|${columns.join(",")}|${rows.length}\u0000`);
    for (const row of rows) {
      hasher.update(`\u0001${JSON.stringify(columns.map((column) => canonicalValue(row[column])))}\u0002`);
    }
    out.push({ table, count: rows.length, hash: hasher.digest("hex") });
  }
  return out;
}

export interface PushRegistryVerifyTable {
  table: string;
  match: boolean;
  sourceCount: number;
  destCount: number;
  sourceHash: string;
  destHash: string;
}

export interface PushRegistryVerification {
  match: boolean;
  tables: PushRegistryVerifyTable[];
}

/** Compare source and destination fingerprints; a missing destination
 * table or row fails closed, never a silent pass. */
export async function verifyPushRegistryCopy(source: SqlClient, dest: SqlClient): Promise<PushRegistryVerification> {
  const [sourceHashes, destHashes] = await Promise.all([hashPushRegistry(source), hashPushRegistry(dest)]);
  const destByTable = new Map(destHashes.map((entry) => [entry.table, entry]));
  const tables: PushRegistryVerifyTable[] = sourceHashes.map((src) => {
    const dst = destByTable.get(src.table);
    return {
      table: src.table,
      match: dst !== undefined && dst.hash === src.hash && dst.count === src.count,
      sourceCount: src.count,
      destCount: dst?.count ?? -1,
      sourceHash: src.hash,
      destHash: dst?.hash ?? "",
    };
  });
  return { match: tables.every((entry) => entry.match), tables };
}
