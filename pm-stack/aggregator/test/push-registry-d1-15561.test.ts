// Oracle: separate SQLite databases reveal which backend receives each operation.
import { describe, expect, test } from "bun:test";
import { Database } from "bun:sqlite";
import { AlertsStore, createAlertsStoreGetter, validateNewSubscription } from "../src/alerts.ts";
import { decodeStoreRow, storeStatement } from "../src/turso-stores.ts";

function client(db: Database) {
  return { ref: "test", async query<T>(input: string): Promise<T[]> {
    const { sql, args } = storeStatement(input);
    return db.query(sql).all(...args.map(a => a.value)).map(row => decodeStoreRow(row as Record<string, unknown>)) as T[];
  } };
}

describe("push registry backend switch", () => {
  test("registry and stages use D1 while send ledger stays on the existing client", async () => {
    const ledger = new Database(":memory:");
    const registry = new Database(":memory:");
    ledger.exec("create table alerts_notification_sends (class text, object_ref text, incident_id text, outcome text, sent_at text)");
    const store = new AlertsStore(client(ledger), client(registry));
    const parsed = validateNewSubscription({ endpoint: "https://push.example/one", keys: { p256dh: "p", auth: "a" } });
    if (!parsed.ok) throw new Error(parsed.error);
    await store.upsertSubscription(parsed.value);
    expect((await store.listSubscriptions()).length).toBe(1);
    await store.itemStages().enterReview("item-1");
    await store.logSend({ class: "test", objectRef: "one", outcome: "sent" });
    expect(registry.query("select count(*) as n from push_endpoints").get()).toEqual({ n: 1 });
    expect(registry.query("select count(*) as n from push_item_stages").get()).toEqual({ n: 1 });
    expect(ledger.query("select count(*) as n from alerts_notification_sends").get()).toEqual({ n: 1 });
    expect(ledger.query("select name from sqlite_master where name = 'push_endpoints'").get()).toBeNull();
    ledger.close(); registry.close();
  });

  test("opt-in never falls back to the ledger client if D1 is unavailable", async () => {
    const db = new Database(":memory:");
    const get = createAlertsStoreGetter(async () => client(db), async () => null, { PM_PUSH_REGISTRY_BACKEND: "d1" });
    await expect(get()).rejects.toThrow("D1 push registry unavailable");
    db.close();
  });
});
