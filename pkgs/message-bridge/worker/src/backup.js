// Bridge state backup (#14951, CF-11 of the #14866 Cloudflare-first
// programme): the MessageStore Durable Object holds every message,
// credential hash, enroll token, note and OAuth record the bridge
// knows, and until now it had no export at all — the state existed in
// exactly one place with no way out. This module is the backup story:
//
//   - exportBridgeState(sql): a portable JSON dump of every table in
//     the DO's SQLite store — column order from PRAGMA, rows verbatim,
//     plus the AUTOINCREMENT high-water marks from sqlite_sequence, so
//     a restore never reissues an id.
//   - importBridgeState(sql, payload): the restore. Every exported
//     table is emptied and refilled row for row, then the sequence
//     marks are put back. A table the payload names but the store
//     does not have refuses the whole restore (schema drift is a
//     stop, not a partial copy).
//   - runBackup(env, store): the scheduled and manual path — dump the
//     store and land it in the BRIDGE_BACKUP R2 bucket as
//     exports/<stamp>.json, with exports/latest.json beside it and
//     the summary in the object metadata.
//
// The restore drill (recorded on #14951): deploy this same Worker as
// a scratch namespace bound to the same bucket, POST
// /api/<token>/backup-import there, and read the state back through
// its own API — the production namespace is never written to.

export const BACKUP_FORMAT = "message-bridge-backup/1";
export const LATEST_KEY = "exports/latest.json";

const stampOf = (iso) => iso.replace(/[:.]/g, "-");
export const exportKeyOf = (iso) => `exports/${stampOf(iso)}.json`;

function tableNames(sql) {
  return sql
    .exec("SELECT name FROM sqlite_master WHERE type = 'table' AND name NOT LIKE 'sqlite_%' ORDER BY name")
    .toArray()
    .map((r) => r.name);
}

function columnsOf(sql, table) {
  return sql
    .exec(`PRAGMA table_info("${table}")`)
    .toArray()
    .sort((a, b) => a.cid - b.cid)
    .map((c) => c.name);
}

/** Dump every table in the store. Throws on a value JSON cannot carry
 *  faithfully (a blob): a backup that silently mangles a column is
 *  worse than no backup. */
export function exportBridgeState(sql, now = new Date()) {
  const tables = tableNames(sql).map((name) => {
    const columns = columnsOf(sql, name);
    const rows = sql
      .exec(`SELECT * FROM "${name}"`)
      .toArray()
      .map((row) =>
        columns.map((c) => {
          const v = row[c] ?? null;
          if (v !== null && typeof v === "object") {
            throw new Error(`backup: table ${name} column ${c} holds a non-JSON value; refusing to export`);
          }
          return v;
        }),
      );
    return { name, columns, rows };
  });
  let sequence = [];
  if (tableNames(sql).length && sql.exec("SELECT name FROM sqlite_master WHERE name = 'sqlite_sequence'").toArray().length) {
    sequence = sql.exec("SELECT name, seq FROM sqlite_sequence ORDER BY name").toArray();
  }
  return { format: BACKUP_FORMAT, exported_at: now.toISOString(), tables, sequence };
}

export function summarizeBackup(payload) {
  const perTable = {};
  let rows = 0;
  for (const t of payload?.tables ?? []) {
    perTable[t.name] = t.rows.length;
    rows += t.rows.length;
  }
  return { tables: perTable, table_count: Object.keys(perTable).length, rows };
}

/** Restore a dump produced by exportBridgeState. Returns per-table
 *  row counts as imported. */
export function importBridgeState(sql, payload) {
  if (!payload || payload.format !== BACKUP_FORMAT || !Array.isArray(payload.tables)) {
    throw new Error(`backup: not a ${BACKUP_FORMAT} payload`);
  }
  const present = new Set(tableNames(sql));
  for (const t of payload.tables) {
    if (!present.has(t.name)) {
      throw new Error(`backup: payload names table ${t.name}, which this store does not have; refusing a partial restore`);
    }
    if (!Array.isArray(t.columns) || !Array.isArray(t.rows)) {
      throw new Error(`backup: table ${t.name} is malformed in the payload`);
    }
  }
  // Empty everything first, so a unique key in one table can never
  // collide with a not-yet-refilled row in another.
  for (const t of payload.tables) sql.exec(`DELETE FROM "${t.name}"`);
  const counts = {};
  for (const t of payload.tables) {
    const cols = t.columns.map((c) => `"${c}"`).join(", ");
    const marks = t.columns.map(() => "?").join(", ");
    for (const row of t.rows) {
      sql.exec(`INSERT INTO "${t.name}" (${cols}) VALUES (${marks})`, ...row);
    }
    counts[t.name] = t.rows.length;
  }
  if (Array.isArray(payload.sequence) && payload.sequence.length) {
    if (sql.exec("SELECT name FROM sqlite_master WHERE name = 'sqlite_sequence'").toArray().length) {
      for (const s of payload.sequence) {
        sql.exec("DELETE FROM sqlite_sequence WHERE name = ?", s.name);
        sql.exec("INSERT INTO sqlite_sequence (name, seq) VALUES (?, ?)", s.name, s.seq);
      }
    }
  }
  return counts;
}

/** One backup run: dump the store, land it in R2 under its stamp and
 *  as the latest export. Fails closed when the bucket is unbound. */
export async function runBackup(env, store) {
  if (!env?.BRIDGE_BACKUP) throw new Error("backup: BRIDGE_BACKUP R2 binding is not configured");
  const payload = await store.backupExport();
  const summary = summarizeBackup(payload);
  const body = JSON.stringify(payload);
  const key = exportKeyOf(payload.exported_at);
  const metadata = {
    exported_at: payload.exported_at,
    rows: String(summary.rows),
    tables: String(summary.table_count),
  };
  await env.BRIDGE_BACKUP.put(key, body, { customMetadata: metadata });
  await env.BRIDGE_BACKUP.put(LATEST_KEY, body, { customMetadata: metadata });
  return { key, bytes: body.length, exported_at: payload.exported_at, ...summary };
}

/** The latest export's summary from its R2 object metadata, or null. */
export async function latestBackup(env) {
  if (!env?.BRIDGE_BACKUP) return null;
  const head = await env.BRIDGE_BACKUP.head(LATEST_KEY);
  if (!head) return null;
  const md = head.customMetadata ?? {};
  return {
    key: LATEST_KEY,
    exported_at: md.exported_at ?? null,
    rows: md.rows !== undefined ? Number(md.rows) : null,
    tables: md.tables !== undefined ? Number(md.tables) : null,
    bytes: head.size ?? null,
  };
}

/** Read one export object (default: the latest) back into a payload. */
export async function readBackup(env, key = LATEST_KEY) {
  if (!env?.BRIDGE_BACKUP) throw new Error("backup: BRIDGE_BACKUP R2 binding is not configured");
  const obj = await env.BRIDGE_BACKUP.get(key);
  if (!obj) throw new Error(`backup: no export object at ${key}`);
  return JSON.parse(await obj.text());
}
