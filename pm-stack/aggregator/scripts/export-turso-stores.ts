#!/usr/bin/env bun
// Usage: export-turso-stores.ts --output-dir DIRECTORY
// Strict arguments before I/O. Exports only the fixed canonical PM stores.
// Output includes private subscription data; keep it inside the private backup.
const args = process.argv.slice(2);
if (args.length !== 2 || args[0] !== "--output-dir" || !args[1] || args[1].startsWith("-")) {
  console.error("Usage: export-turso-stores.ts --output-dir DIRECTORY");
  process.exit(2);
}
const { mkdir, writeFile, rename } = await import("node:fs/promises");
const { join } = await import("node:path");
const { getTursoStoreClient } = await import("../src/turso-stores.ts");
const db = await getTursoStoreClient();
if (!db) throw new Error("Turso store credential unavailable");
const tables = {
  agg_actions: "id", agg_activity_ingestion: "source", agg_activity_gaps: "id",
  agg_usage_snapshots: "snapshot_key", agg_store_documents: "store",
  alerts_incidents: "id", alerts_incident_events: "id", alerts_push_subscriptions: "id",
  alerts_notification_sends: "id", orch_runs: "run_id",
};
await mkdir(args[1], { recursive: true, mode: 0o700 });
for (const [table, key] of Object.entries(tables)) {
  const rows: Record<string, unknown>[] = [];
  let cursor: unknown = null;
  for (;;) {
    const value = typeof cursor === "number" ? String(cursor) : `'${String(cursor).replaceAll("'", "''")}'`;
    const batch = await db.query<Record<string, unknown>>(`select * from ${table} ${cursor === null ? "" : `where ${key} > ${value}`} order by ${key} limit 500`);
    rows.push(...batch);
    if (batch.length < 500) break;
    const next = batch.at(-1)![key];
    if (next === null || next === undefined || next === cursor) throw new Error("nonadvancing export cursor");
    cursor = next;
  }
  const dest = join(args[1], `${table}.json`);
  const temp = `${dest}.tmp-${process.pid}`;
  await writeFile(temp, JSON.stringify(rows), {mode:0o600, flag:"wx"});
  await rename(temp, dest);
  console.log(`${table}=${rows.length}`);
}
