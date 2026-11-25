// #14084 S1 — the one push registry and the staged pipeline.
//
// Oracle: SQLite executes every statement through the same dialect
// bridge the Turso client uses (storeStatement + decodeStoreRow), so
// these tests run the store's real SQL, not a string snapshot.
import { describe, expect, test } from "bun:test";
import { Database } from "bun:sqlite";
import { decodeStoreRow, storeStatement } from "../src/turso-stores.ts";
import { AlertsStore, classEnabled, validateNewSubscription, type NewSubscription } from "../src/alerts.ts";
import {
  DEFAULT_DELIVERY,
  ensurePushRegistry,
  ItemStageStore,
  LEGACY_SEED_MARKER,
  mergeDelivery,
  PUSH_REGISTRY_TABLES,
  SEND_BACK_OPTION,
  sortForAnswering,
  type ItemStageRow,
} from "../src/push-registry.ts";
import { NOTIFICATION_CLASSES, NOTIFICATION_CLASS_DEFAULTS } from "../src/notifications.ts";

function localClient(db: Database) {
  return {
    ref: "local-test",
    async query<T>(input: string): Promise<T[]> {
      const { sql, args } = storeStatement(input);
      return db
        .query(sql)
        .all(...args.map((a) => a.value))
        .map((r) => decodeStoreRow(r as Record<string, unknown>)) as T[];
    },
  };
}

const LEGACY_DDL = `create table alerts_push_subscriptions(id text default 'sub', endpoint text unique, p256dh text, auth text, classes text, quiet text, user_agent text, last_seen_at text, created_at text, gone_count integer default 0)`;

function subscription(body: Record<string, unknown>): NewSubscription {
  const parsed = validateNewSubscription({ keys: { p256dh: "p", auth: "a" }, ...body });
  if (!parsed.ok) throw new Error(parsed.error);
  return parsed.value;
}

const tableNames = (db: Database) =>
  (db.query("select name from sqlite_master where type = 'table'").all() as { name: string }[]).map((r) => r.name);

describe("registry schema", () => {
  test("ensure creates every registry table, idempotently, with no defaulted timestamp", async () => {
    const db = new Database(":memory:");
    const c = localClient(db);
    await ensurePushRegistry(c);
    await ensurePushRegistry(c);
    for (const table of Object.keys(PUSH_REGISTRY_TABLES)) expect(tableNames(db)).toContain(table);
    // Writer-stamped times (the #14061 lesson): no timestamp column has a default.
    for (const table of Object.keys(PUSH_REGISTRY_TABLES)) {
      const cols = db.query(`pragma table_info(${table})`).all() as { name: string; dflt_value: unknown }[];
      for (const col of cols.filter((c) => /_at$|^at$/.test(c.name)))
        expect([table, col.dflt_value]).toEqual([table, null]);
    }
    // No legacy table here: the seed marker still lands, recording zero.
    const meta = db.query("select key, value from push_registry_meta").all();
    expect(meta).toEqual([{ key: LEGACY_SEED_MARKER, value: "0" }]);
    db.close();
  });

  test("legacy rows seed once and an unsubscribed endpoint is never resurrected", async () => {
    const db = new Database(":memory:");
    db.exec(LEGACY_DDL);
    db.query(
      "insert into alerts_push_subscriptions (endpoint, p256dh, auth, classes, quiet, created_at, last_seen_at) values (?, ?, ?, ?, ?, ?, ?)",
    ).run(
      "https://push.example/legacy",
      "lp",
      "la",
      JSON.stringify({ incident: true, "needs-drew": false }),
      JSON.stringify({ start: "22:00", end: "06:00", tz: "UTC" }),
      "2026-11-10T00:00:00.000Z",
      "2026-11-11T00:00:00.000Z",
    );
    const store = new AlertsStore(localClient(db));
    const [row] = await store.listSubscriptions();
    expect(row.endpoint).toBe("https://push.example/legacy");
    expect(row.client).toBe("browser");
    expect(row.transport).toBe("webpush");
    expect(row.created_at).toBe("2026-11-10T00:00:00.000Z");
    // The legacy class id reads as the class of record.
    expect(row.classes).toEqual({ incident: true, "needs-user": false });
    expect(classEnabled(row, "needs-user")).toBe(false);
    expect(row.quiet.start).toBe("22:00");
    expect(row.delivery).toEqual(DEFAULT_DELIVERY);

    expect(await store.deleteSubscription(row.endpoint)).toBe(true);
    // A fresh process (new store instance) re-runs ensure: the marker
    // stops the seed from bringing the deleted endpoint back.
    const restarted = new AlertsStore(localClient(db));
    expect(await restarted.listSubscriptions()).toEqual([]);
    db.close();
  });
});

describe("AlertsStore over the registry", () => {
  test("subscribe, re-subscribe, preferences merge, delivery kept unless sent", async () => {
    const db = new Database(":memory:");
    const store = new AlertsStore(localClient(db));
    const first = await store.upsertSubscription(
      subscription({
        endpoint: "https://push.example/app",
        client: "android-app",
        deviceId: "device-1",
        label: "Phone",
        userAgent: "ua-1",
        delivery: { rateCap: { max: 4, perMinutes: 60 }, grouping: true, groupReminderMinutes: 240 },
      }),
    );
    expect(first.client).toBe("android-app");
    expect(first.device_id).toBe("device-1");
    expect(first.label).toBe("Phone");
    expect(first.classes).toEqual(NOTIFICATION_CLASS_DEFAULTS);
    expect(first.delivery).toEqual({
      immediate: true,
      rateCap: { max: 4, perMinutes: 60 },
      grouping: true,
      groupReminderMinutes: 240,
    });
    expect(first.created_at).toMatch(/^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}\.\d{3}Z$/);

    // The preferences page posts only the class it changed; delivery
    // and the device link are not sent and must survive.
    await store.noteSubscriptionGone(first.endpoint);
    const second = await store.upsertSubscription(
      subscription({ endpoint: "https://push.example/app", classes: { "quota-wall": false } }),
    );
    expect(second.id).toBe(first.id);
    expect(second.gone_count).toBe(0);
    expect(second.classes["quota-wall"]).toBe(false);
    expect(second.classes.incident).toBe(true);
    expect(second.device_id).toBe("device-1");
    expect(second.label).toBe("Phone");
    expect(second.delivery).toEqual(first.delivery);

    const third = await store.upsertSubscription(
      subscription({ endpoint: "https://push.example/app", delivery: { immediate: true } }),
    );
    expect(third.delivery).toEqual(DEFAULT_DELIVERY);
    expect(third.classes["quota-wall"]).toBe(false);

    await store.upsertSubscription(subscription({ endpoint: "https://push.example/browser" }));
    const all = await store.listSubscriptions();
    // A resubscribe that does not name its client keeps the stored one.
    expect(all.map((s) => `${s.endpoint} ${s.client}`).sort()).toEqual([
      "https://push.example/app android-app",
      "https://push.example/browser browser",
    ]);
    db.close();
  });

  test("two gone responses prune; a live send resets the streak", async () => {
    const db = new Database(":memory:");
    const store = new AlertsStore(localClient(db));
    await store.upsertSubscription(subscription({ endpoint: "https://push.example/g" }));
    expect(await store.noteSubscriptionGone("https://push.example/g")).toBe("marked");
    await store.noteSubscriptionAlive("https://push.example/g");
    expect((await store.listSubscriptions())[0].gone_count).toBe(0);
    expect(await store.noteSubscriptionGone("https://push.example/g")).toBe("marked");
    expect(await store.noteSubscriptionGone("https://push.example/g")).toBe("pruned");
    expect(await store.listSubscriptions()).toEqual([]);
    // The preference row goes with its endpoint.
    expect(db.query("select count(*) as n from push_preferences").get()).toEqual({ n: 0 });
    db.close();
  });
});

describe("taxonomy", () => {
  test("the answerable class is needs-user and no needs-drew class id remains", () => {
    const ids = NOTIFICATION_CLASSES.map((c) => c.id);
    expect(ids).toContain("needs-user");
    expect(ids).not.toContain("needs-drew");
    expect(Object.keys(NOTIFICATION_CLASS_DEFAULTS)).not.toContain("needs-drew");
    expect(NOTIFICATION_CLASSES.find((c) => c.id === "needs-user")!.label).toBe("Needs user");
  });
});

describe("delivery settings", () => {
  test("send-immediately is the default; invalid fields fall back per field", () => {
    expect(mergeDelivery(undefined)).toEqual({
      immediate: true,
      rateCap: null,
      grouping: false,
      groupReminderMinutes: null,
    });
    expect(mergeDelivery({ rateCap: { max: 0, perMinutes: 60 } }).rateCap).toBeNull();
    expect(mergeDelivery({ rateCap: { max: 5, perMinutes: 99999 } }).rateCap).toBeNull();
    expect(mergeDelivery({ rateCap: { max: 5, perMinutes: 30 } }).rateCap).toEqual({ max: 5, perMinutes: 30 });
    expect(mergeDelivery({ groupReminderMinutes: 1 }).groupReminderMinutes).toBeNull();
    expect(mergeDelivery({ grouping: "yes", immediate: false })).toEqual({ ...DEFAULT_DELIVERY, immediate: false });
  });
});

describe("staged pipeline (needs-review -> needs-user, send back)", () => {
  function stages() {
    const db = new Database(":memory:");
    let t = Date.parse("2026-11-25T00:00:00.000Z");
    const store = new ItemStageStore(localClient(db), () => new Date((t += 1000)));
    return { db, store };
  }

  test("every item enters at needs-review; escalation and send-back move it with one event each", async () => {
    const { db, store } = stages();
    const entered = await store.enterReview("needs-drew:ledger-1", "raised");
    expect(entered.transitioned).toBe(true);
    expect(entered.item!.stage).toBe("needs-review");
    expect((await store.enterReview("needs-drew:ledger-1")).transitioned).toBe(false);

    const up = await store.escalate("needs-drew:ledger-1", "only Drew can decide");
    expect(up.transitioned).toBe(true);
    expect(up.item!.stage).toBe("needs-user");
    expect((await store.escalate("needs-drew:ledger-1")).transitioned).toBe(false);

    const back = await store.sendBack("needs-drew:ledger-1", "AI can decide this");
    expect(back.transitioned).toBe(true);
    expect(back.item!.stage).toBe("needs-review");
    // Send back applies only to a needs-user item.
    expect((await store.sendBack("needs-drew:ledger-1")).transitioned).toBe(false);

    const events = await store.events("needs-drew:ledger-1");
    expect(events.map((e) => [e.from_stage, e.to_stage, e.reason])).toEqual([
      [null, "needs-review", "raised"],
      ["needs-review", "needs-user", "only Drew can decide"],
      ["needs-user", "needs-review", "AI can decide this"],
    ]);
    expect(events.every((e) => /Z$/.test(e.at))).toBe(true);
    db.close();
  });

  test("escalating an item not yet in the pipeline records its review stage first", async () => {
    const { db, store } = stages();
    await store.escalate("item:new");
    expect((await store.events("item:new")).map((e) => e.to_stage)).toEqual(["needs-review", "needs-user"]);
    expect((await store.resolve("item:new", "answered")).item!.stage).toBe("resolved");
    expect((await store.sendBack("item:new")).transitioned).toBe(false);
    expect((await store.reopen("item:new")).item!.stage).toBe("needs-review");
    db.close();
  });

  test("the needs-user list orders by priority, then issues unblocked, then age", async () => {
    const { db, store } = stages();
    for (const ref of ["a", "b", "c", "d"]) await store.escalate(ref);
    await store.setPriority("c", 1);
    await store.setBlockedByCount("b", 7);
    await store.setBlockedByCount("d", 2);
    await expect(store.setPriority("a", 9)).rejects.toThrow("priority");
    await expect(store.setBlockedByCount("a", -1)).rejects.toThrow("blocked-by");
    const list = await store.list("needs-user");
    expect(list.map((r) => [r.item_ref, r.priority, r.blocked_by_count])).toEqual([
      ["c", 1, 0],
      ["b", null, 7],
      ["d", null, 2],
      ["a", null, 0],
    ]);
    expect(await store.list("needs-review")).toEqual([]);
    db.close();
  });

  test("send back is the standard option every needs-user item carries", () => {
    expect(SEND_BACK_OPTION).toEqual({ id: "send-back", label: "Send back" });
  });

  test("sortForAnswering does not depend on store order", () => {
    const row = (item_ref: string, priority: number | null, blocked: number, at: string): ItemStageRow => ({
      item_ref,
      stage: "needs-user",
      priority,
      blocked_by_count: blocked,
      entered_stage_at: at,
      updated_at: at,
    });
    const rows = [row("z", null, 0, "2026-01-02"), row("y", null, 0, "2026-01-01"), row("x", 2, 0, "2026-01-03")];
    expect(sortForAnswering(rows).map((r) => r.item_ref)).toEqual(["x", "y", "z"]);
  });
});
