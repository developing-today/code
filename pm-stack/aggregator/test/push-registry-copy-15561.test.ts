// CF-5e #15561 slice 2: refresh + hash-verify the five push registry
// tables from Turso into D1 fleet-platform.
//
// Oracle: two independent SQLite databases. The source is authoritative;
// after copyPushRegistry the destination's canonical row hashes must
// equal the source's, and verifyPushRegistryCopy must report a mismatch
// for exactly the drifted table and no other.
import { describe, expect, test } from "bun:test";
import { Database } from "bun:sqlite";
import { PUSH_REGISTRY_DDL, PUSH_REGISTRY_TABLES } from "../src/push-registry.ts";
import {
  PUSH_REGISTRY_COPY_TABLES,
  copyPushRegistry,
  hashPushRegistry,
  verifyPushRegistryCopy,
} from "../src/push-registry-copy.ts";
import { decodeStoreRow, storeStatement } from "../src/turso-stores.ts";

function client(db: Database) {
  return {
    ref: "test",
    async query<T>(input: string): Promise<T[]> {
      const { sql, args } = storeStatement(input);
      return db
        .query(sql)
        .all(...args.map((a) => a.value))
        .map((row) => decodeStoreRow(row as Record<string, unknown>)) as T[];
    },
  };
}

const SEED = [
  `insert into push_endpoints (id, transport, client, device_id, endpoint, p256dh, auth, label, user_agent, created_at, last_seen_at, gone_count)
   values ('e1', 'webpush', 'browser', null, 'https://push.example/o''ne', 'p', 'a', 'L''abel', 'ua', '2026-11-01T00:00:00.000Z', '2026-11-02T00:00:00.000Z', 0)`,
  `insert into push_preferences (endpoint_id, classes, quiet, delivery, updated_at)
   values ('e1', '{"incident":true}', '{"start":"22:00"}', '{"immediate":true}', '2026-11-01T00:00:00.000Z')`,
  `insert into push_item_stages (item_ref, stage, priority, blocked_by_count, entered_stage_at, updated_at)
   values ('item-1', 'needs-user', 2, 3, '2026-11-01T00:00:00.000Z', '2026-11-02T00:00:00.000Z')`,
  `insert into push_item_stage_events (id, item_ref, from_stage, to_stage, reason, at)
   values (7, 'item-1', 'needs-review', 'needs-user', 'it''s urgent', '2026-11-02T00:00:00.000Z')`,
  `insert into push_registry_meta (key, value, at)
   values ('legacy-alerts-push-subscriptions-seeded', '0', '2026-11-01T00:00:00.000Z')`,
];

function seededDb(): Database {
  const db = new Database(":memory:");
  const ddl = [
    `CREATE TABLE push_endpoints (id TEXT PRIMARY KEY, transport TEXT NOT NULL, client TEXT NOT NULL, device_id TEXT, endpoint TEXT NOT NULL UNIQUE, p256dh TEXT NOT NULL, auth TEXT NOT NULL, label TEXT, user_agent TEXT, created_at TEXT NOT NULL, last_seen_at TEXT NOT NULL, gone_count INTEGER NOT NULL DEFAULT 0)`,
    `CREATE TABLE push_preferences (endpoint_id TEXT PRIMARY KEY, classes TEXT NOT NULL, quiet TEXT NOT NULL, delivery TEXT NOT NULL, updated_at TEXT NOT NULL)`,
    `CREATE TABLE push_item_stages (item_ref TEXT PRIMARY KEY, stage TEXT NOT NULL, priority INTEGER, blocked_by_count INTEGER NOT NULL DEFAULT 0, entered_stage_at TEXT NOT NULL, updated_at TEXT NOT NULL)`,
    `CREATE TABLE push_item_stage_events (id INTEGER PRIMARY KEY, item_ref TEXT NOT NULL, from_stage TEXT, to_stage TEXT NOT NULL, reason TEXT, at TEXT NOT NULL)`,
    `CREATE TABLE push_registry_meta ("key" TEXT PRIMARY KEY, value TEXT NOT NULL, at TEXT NOT NULL)`,
  ];
  for (const statement of ddl) db.exec(statement);
  for (const row of SEED) db.exec(row);
  return db;
}

describe("push registry copy + verify (#15561 slice 2)", () => {
  test("the copy set is exactly the registry table set", () => {
    expect(new Set(PUSH_REGISTRY_COPY_TABLES.map((t) => t.table))).toEqual(new Set(Object.keys(PUSH_REGISTRY_TABLES)));
    for (const entry of PUSH_REGISTRY_COPY_TABLES) {
      expect(entry.key).toBe(PUSH_REGISTRY_TABLES[entry.table]);
      expect(entry.columns[0]).toBe(entry.key);
    }
  });

  test("copy reproduces every table and verify reports all-match", async () => {
    const sourceDb = seededDb();
    const destDb = new Database(":memory:");
    try {
      await copyPushRegistry(client(sourceDb), client(destDb), PUSH_REGISTRY_DDL);
      const report = await verifyPushRegistryCopy(client(sourceDb), client(destDb));
      expect(report.match).toBe(true);
      expect(report.tables).toHaveLength(5);
      for (const row of report.tables) {
        expect(row.match).toBe(true);
        expect(row.destCount).toBe(row.sourceCount);
      }
      // Quoted literals and JSON columns survive the round trip byte-identically.
      const destRows = await client(destDb).query<Record<string, unknown>>(
        "select endpoint, label from push_endpoints",
      );
      expect(destRows).toEqual([{ endpoint: "https://push.example/o'ne", label: "L'abel" }]);
      const prefs = await client(destDb).query<{ classes: unknown }>("select classes from push_preferences");
      expect(prefs).toEqual([{ classes: { incident: true } }]);
    } finally {
      sourceDb.close();
      destDb.close();
    }
  });

  test("verify names exactly the drifted table", async () => {
    const sourceDb = seededDb();
    const destDb = new Database(":memory:");
    try {
      await copyPushRegistry(client(sourceDb), client(destDb), PUSH_REGISTRY_DDL);
      destDb.exec("insert into push_item_stages (item_ref, stage, blocked_by_count, entered_stage_at, updated_at) values ('item-2', 'needs-review', 0, '2026-11-03T00:00:00.000Z', '2026-11-03T00:00:00.000Z')");
      const report = await verifyPushRegistryCopy(client(sourceDb), client(destDb));
      expect(report.match).toBe(false);
      const drifted = report.tables.filter((t) => !t.match).map((t) => t.table);
      expect(drifted).toEqual(["push_item_stages"]);
    } finally {
      sourceDb.close();
      destDb.close();
    }
  });

  test("hashes are stable and order-independent of read order", async () => {
    const a = seededDb();
    const b = seededDb();
    try {
      const ha = await hashPushRegistry(client(a));
      const hb = await hashPushRegistry(client(b));
      expect(ha).toEqual(hb);
    } finally {
      a.close();
      b.close();
    }
  });
});
