// Alert throat (#3676) — pure-helper tests. The store itself is
// SQL-backed and verified live against muse-dev2; what can be pinned
// without a database is pinned here: SQL literal escaping, the
// class/quiet merge rules (design §1/§2), payload + deep-link shape
// (§3), the duplicate-suppression rule, and the probe's foreign-
// registration test (§6).
import { describe, expect, test } from "bun:test";
import {
  buildIncidentPayload,
  buildTestPayload,
  classEnabled,
  createAlertsStoreGetter,
  duplicateWouldRepeatDelivery,
  incidentObjectRef,
  incidentUrl,
  inQuietHours,
  mergeClasses,
  mergeQuiet,
  runAlertTick,
  sendTestPush,
  TEST_OBJECT_REF,
  toPublicSubscription,
  validateNewIncident,
  validateNewSubscription,
  AlertsStore,
  type AlertTickOptions,
  type AlertsStore as AlertsStoreType,
  type Incident,
  type PushSubscriptionRow,
} from "../src/alerts.ts";
import { isForeignSwScript, type PushOutcome, type PushTarget } from "../src/push.ts";
import { renderAlerts } from "../src/render.ts";
import type { Provenance } from "../src/model.ts";
import { sqlInt, sqlJson, sqlStr, type SupabaseClient } from "../src/supabase.ts";
import type { Config } from "../src/config.ts";
import type { NotificationHistoryEntry, NotificationSnapshot } from "../src/notifications.ts";

describe("getAlertsStore retry-after-null memoization", () => {
  test("retries a null client, shares the next in-flight attempt, and caches success", async () => {
    let clientCalls = 0;
    let resolveFirst!: (value: SupabaseClient | null) => void;
    const firstResult = new Promise<SupabaseClient | null>((resolve) => {
      resolveFirst = resolve;
    });
    let resolveRetry!: (value: SupabaseClient | null) => void;
    const retryResult = new Promise<SupabaseClient | null>((resolve) => {
      resolveRetry = resolve;
    });
    const fakeClient: SupabaseClient = {
      ref: "test-project",
      async query<T>() {
        return [] as T[];
      },
    };
    const getStore = createAlertsStoreGetter(async () => {
      clientCalls += 1;
      return clientCalls === 1 ? firstResult : retryResult;
    });

    const initial = getStore();
    const concurrentInitial = getStore();
    expect(clientCalls).toBe(1);
    resolveFirst(null);
    expect(await Promise.all([initial, concurrentInitial])).toEqual([null, null]);

    const first = getStore();
    const second = getStore();
    expect(clientCalls).toBe(2);
    resolveRetry(fakeClient);

    const [store, concurrentStore] = await Promise.all([first, second]);
    expect(store).not.toBeNull();
    expect(store).toBe(concurrentStore);
    expect(await getStore()).toBe(store);
    expect(clientCalls).toBe(2);
  });
});

describe("sql literals", () => {
  test("single quotes double", () => {
    expect(sqlStr("o'brien")).toBe("'o''brien'");
    expect(sqlStr("plain")).toBe("'plain'");
  });
  test("jsonb literal escapes through the string escaper", () => {
    expect(sqlJson({ a: "b'c" })).toBe(`'{"a":"b''c"}'::jsonb`);
  });
  test("integers only", () => {
    expect(sqlInt(15)).toBe("15");
    expect(() => sqlInt(1.5)).toThrow();
    expect(() => sqlInt(Number.NaN)).toThrow();
  });
});

describe("class + quiet merges (design §1/§2)", () => {
  test("incident defaults on; explicit off survives", () => {
    expect(mergeClasses(undefined)).toEqual({
      incident: true,
      "needs-drew": true,
      "quota-wall": true,
      "quota-near-wall": true,
      "stack-health": true,
      "blocked-age": true,
    });
    expect(mergeClasses({ incident: false }).incident).toBe(false);
    expect(mergeClasses({ incident: true, "quota-wall": false })["quota-wall"]).toBe(false);
    expect(mergeClasses({ incident: "yes" }).incident).toBe(true); // non-boolean ignored
  });
  test("classEnabled preserves old rows while new rows get defaults", () => {
    expect(classEnabled({ classes: {} }, "incident")).toBe(true);
    expect(classEnabled({ classes: { incident: false } }, "incident")).toBe(false);
    expect(classEnabled({ classes: { incident: true } }, "incident")).toBe(true);
    expect(classEnabled({ classes: {} }, "needs-drew")).toBe(false);
    expect(classEnabled({ classes: mergeClasses(undefined) }, "needs-drew")).toBe(true);
  });
  test("quiet defaults 23:00–07:00 America/Chicago; partial overrides merge", () => {
    expect(mergeQuiet(undefined)).toEqual({ start: "23:00", end: "07:00", tz: "America/Chicago", enabled: true });
    expect(mergeQuiet({ start: "22:30" })).toEqual({
      start: "22:30",
      end: "07:00",
      tz: "America/Chicago",
      enabled: true,
    });
    expect(mergeQuiet({ start: "9pm" })).toEqual({
      start: "23:00",
      end: "07:00",
      tz: "America/Chicago",
      enabled: true,
    });
  });
  test("out-of-range quiet times fall back per field (#3686)", () => {
    // 99:99 and 24:60 keep the shape but are not real clock times —
    // each malformed field independently retains its DEFAULT_QUIET value.
    expect(mergeQuiet({ start: "99:99" }).start).toBe("23:00");
    expect(mergeQuiet({ end: "24:60" }).end).toBe("07:00");
    expect(mergeQuiet({ start: "24:00" }).start).toBe("23:00");
    expect(mergeQuiet({ end: "12:60" }).end).toBe("07:00");
    // Mixed: one bad field falls back while the good field is kept.
    expect(mergeQuiet({ start: "99:99", end: "23:59" })).toEqual({
      start: "23:00",
      end: "23:59",
      tz: "America/Chicago",
      enabled: true,
    });
  });
  test("boundary clock times 00:00 and 23:59 are accepted (#3686)", () => {
    expect(mergeQuiet({ start: "00:00" }).start).toBe("00:00");
    expect(mergeQuiet({ end: "23:59" }).end).toBe("23:59");
    expect(mergeQuiet({ start: "00:00", end: "23:59" })).toEqual({
      start: "00:00",
      end: "23:59",
      tz: "America/Chicago",
      enabled: true,
    });
  });
  test("invalid quiet timezone falls back to the default", () => {
    expect(mergeQuiet({ tz: "Not/AZone" }).tz).toBe("America/Chicago");
  });
  test("valid non-default timezone is preserved and used", () => {
    const quiet = mergeQuiet({ tz: "UTC" });
    expect(quiet.tz).toBe("UTC");
    // 2026-01-15T04:00Z is 04:00 UTC (inside), but 22:00 CST (outside).
    expect(inQuietHours(quiet, new Date("2026-01-15T04:00:00Z"))).toBe(true);
  });
  test("inQuietHours wraps midnight and honors the window edges", () => {
    const q = { start: "23:00", end: "07:00", tz: "America/Chicago" };
    // 2026-01-15T05:30Z = Jan 14 23:30 CST — inside.
    expect(inQuietHours(q, new Date("2026-01-15T05:30:00Z"))).toBe(true);
    // 2026-01-15T14:00Z = Jan 15 08:00 CST — outside.
    expect(inQuietHours(q, new Date("2026-01-15T14:00:00Z"))).toBe(false);
    // 2026-01-15T12:59Z = 06:59 CST — inside; 13:00Z = 07:00 — outside (end exclusive).
    expect(inQuietHours(q, new Date("2026-01-15T12:59:00Z"))).toBe(true);
    expect(inQuietHours(q, new Date("2026-01-15T13:00:00Z"))).toBe(false);
  });
  test("quiet hours OFF survives the merge (Drew 2026-11-01: no time-of-day limit)", () => {
    expect(mergeQuiet({ enabled: false })).toEqual({
      start: "23:00",
      end: "07:00",
      tz: "America/Chicago",
      enabled: false,
    });
    expect(mergeQuiet({ enabled: true }).enabled).toBe(true);
    expect(mergeQuiet({ enabled: "no" }).enabled).toBe(true); // non-boolean ignored — stays on
  });
  test("inQuietHours is never true when quiet hours are disabled", () => {
    const off = { start: "23:00", end: "07:00", tz: "America/Chicago", enabled: false };
    // 2026-01-15T05:30Z = Jan 14 23:30 CST — dead center of the window.
    expect(inQuietHours(off, new Date("2026-01-15T05:30:00Z"))).toBe(false);
    expect(inQuietHours(off, new Date("2026-01-15T12:59:00Z"))).toBe(false);
  });
});

describe("stored subscription preference reads (#4978)", () => {
  test("invalid stored quiet fields fall back per field without rewriting rows", async () => {
    const invalidQuiet = { start: "99:99", end: "24:60", tz: "Not/AZone", enabled: true };
    const validQuiet = { start: "22:30", end: "06:15", tz: "UTC", enabled: true };
    const rawRows: PushSubscriptionRow[] = [
      {
        id: "sub-invalid",
        endpoint: "https://push.example/invalid",
        p256dh: "p256dh",
        auth: "auth",
        classes: { incident: true },
        quiet: invalidQuiet,
        user_agent: null,
        created_at: "2026-11-01T00:00:00Z",
        last_seen_at: "2026-11-01T00:00:00Z",
        gone_count: 0,
      },
      {
        id: "sub-valid",
        endpoint: "https://push.example/valid",
        p256dh: "p256dh",
        auth: "auth",
        classes: { incident: true },
        quiet: validQuiet,
        user_agent: null,
        created_at: "2026-11-02T00:00:00Z",
        last_seen_at: "2026-11-02T00:00:00Z",
        gone_count: 0,
      },
    ];
    const queries: string[] = [];
    const db: SupabaseClient = {
      ref: "test",
      async query<T>(sql: string): Promise<T[]> {
        queries.push(sql);
        return rawRows as unknown as T[];
      },
    };

    const loaded = await new AlertsStore(db).listSubscriptions();

    expect(loaded.map((row) => row.quiet)).toEqual([
      { start: "23:00", end: "07:00", tz: "America/Chicago", enabled: true },
      validQuiet,
    ]);
    expect(inQuietHours(loaded[0].quiet, new Date("2026-01-15T12:00:00Z"))).toBe(true);
    expect(rawRows.map((row) => row.quiet)).toEqual([invalidQuiet, validQuiet]);
    expect(queries).toEqual(["select * from alerts.push_subscriptions order by created_at"]);
  });
});

const incident: Incident = {
  id: "11111111-2222-3333-4444-555555555555",
  source: "globalping-watch",
  incident_key: "zettel.drewrypope.com",
  severity: "critical",
  title: "zettel.drewrypope.com down",
  body: "probes saw 503",
  status: "open",
  first_seen_at: "2026-11-01T12:00:00Z",
  last_seen_at: "2026-11-01T12:00:00Z",
  delivered_at: null,
  escalated_at: null,
  acked_at: null,
  closed_at: null,
  occurrences: 1,
};

describe("payload + deep link (design §3)", () => {
  test("objectRef and url anchor the incident on /alerts", () => {
    expect(incidentObjectRef(incident.id)).toBe(`incident:${incident.id}`);
    expect(incidentUrl("https://amd-6.example.ts.net", incident.id)).toBe(
      `https://amd-6.example.ts.net/alerts#incident-${incident.id}`,
    );
  });
  test("payload names the item in plain text and deep-links to it", () => {
    const p = buildIncidentPayload(incident, "https://amd-6.example.ts.net", false);
    expect(p.class).toBe("incident");
    expect(p.url).toContain("#incident-");
    expect(p.body).toContain("zettel.drewrypope.com down");
    expect(p.body).toContain("globalping-watch");
    const esc = buildIncidentPayload(incident, "https://amd-6.example.ts.net", true);
    expect(esc.escalation).toBe(true);
    expect(esc.title).toStartWith("Escalation:");
  });
});

describe("duplicate suppression rule", () => {
  test("a repeat post only 'suppresses a delivery' once one was delivered", () => {
    expect(duplicateWouldRepeatDelivery(incident)).toBe(false);
    expect(duplicateWouldRepeatDelivery({ ...incident, delivered_at: "2026-11-01T12:01:00Z" })).toBe(true);
    expect(
      duplicateWouldRepeatDelivery({
        ...incident,
        delivered_at: "2026-11-01T12:01:00Z",
        status: "closed",
        closed_at: "2026-11-01T12:02:00Z",
      }),
    ).toBe(false);
  });
});

describe("validation", () => {
  test("incident intake requires source/key/title and a known severity", () => {
    expect(validateNewIncident({ source: "s", key: "k", title: "t", severity: "critical" }).ok).toBe(true);
    expect(validateNewIncident({ source: "s", key: "k", title: "t", severity: "loud" }).ok).toBe(false);
    expect(validateNewIncident({ key: "k", title: "t", severity: "critical" }).ok).toBe(false);
    expect(validateNewIncident(null).ok).toBe(false);
  });
  test("subscription intake takes the PushSubscription JSON shape", () => {
    const ok = validateNewSubscription({
      endpoint: "https://fcm.googleapis.com/fcm/send/abc",
      keys: { p256dh: "p", auth: "a" },
    });
    expect(ok.ok).toBe(true);
    if (ok.ok) {
      expect(ok.value.classes).toEqual({
        incident: true,
        "needs-drew": true,
        "quota-wall": true,
        "quota-near-wall": true,
        "stack-health": true,
        "blocked-age": true,
      });
      expect(ok.value.quiet.tz).toBe("America/Chicago");
    }
    expect(
      validateNewSubscription({ endpoint: "http://insecure.example/x", keys: { p256dh: "p", auth: "a" } }).ok,
    ).toBe(false);
    expect(validateNewSubscription({ endpoint: "https://x.example/", keys: {} }).ok).toBe(false);
  });
});

describe("sw-probe foreign test (design §6)", () => {
  test("only /sw.js is ours", () => {
    expect(isForeignSwScript("https://amd-6.example.ts.net/sw.js")).toBe(false);
    expect(isForeignSwScript("https://amd-6.example.ts.net/service-worker.js")).toBe(true);
    expect(isForeignSwScript("not a url")).toBe(true);
  });
});

const tickConfig = {
  publicOrigin: "https://alerts.example.ts.net",
  alerts: { ackWindowMin: 30, tickMs: 60_000 },
} as Config;

function alertTickStore(
  options: { deliveries?: Incident[]; escalations?: Incident[]; noSubscriptions?: boolean } = {},
) {
  const calls = {
    delivered: [] as Array<{ id: string; detail: Record<string, unknown> }>,
    escalated: [] as Array<{ id: string; detail: Record<string, unknown> }>,
    sends: [] as Array<{ outcome: string; incidentId?: string }>,
  };
  const store = {
    listSubscriptions: async () =>
      options.noSubscriptions
        ? []
        : [
            {
              id: "sub-1",
              endpoint: "https://push.example/sub-1",
              p256dh: "p256dh",
              auth: "auth",
              classes: { incident: true },
              quiet: { start: "23:00", end: "07:00", tz: "UTC" },
              user_agent: null,
              created_at: "2026-11-01T00:00:00Z",
              last_seen_at: "2026-11-01T00:00:00Z",
              gone_count: 0,
            },
          ],
    pendingDeliveries: async () => (calls.delivered.length ? [] : (options.deliveries ?? [incident])),
    pendingEscalations: async () => (calls.escalated.length ? [] : (options.escalations ?? [])),
    markDelivered: async (id: string, detail: Record<string, unknown>) => {
      calls.delivered.push({ id, detail });
      return true;
    },
    markEscalated: async (id: string, detail: Record<string, unknown>) => {
      calls.escalated.push({ id, detail });
      return true;
    },
    logSend: async (entry: { outcome: string; incidentId?: string }) => {
      calls.sends.push(entry);
    },
    noteSubscriptionAlive: async () => {},
    noteSubscriptionGone: async () => "marked" as const,
  };
  return { store: store as unknown as AlertsStoreType, calls };
}

function pendingIncidentStore(rows: Incident[]): AlertsStore {
  const db: SupabaseClient = {
    ref: "test",
    async query<T>(sql: string): Promise<T[]> {
      const orderColumn = /order by (first_seen_at|delivered_at)/i.exec(sql)?.[1] as
        | "first_seen_at"
        | "delivered_at"
        | undefined;
      if (!orderColumn) throw new Error(`unexpected query: ${sql}`);
      const isDeliveryQuery = /delivered_at is null/i.test(sql);
      const limit = Number(/limit (\d+)/i.exec(sql)?.[1]);
      const cursorMatch = /and \((first_seen_at|delivered_at), id\) (>|<=) \('([^']+)', '([^']+)'\)/i.exec(sql);
      const candidates = rows
        .filter((row) => (isDeliveryQuery ? row.delivered_at === null : row.delivered_at !== null))
        .filter((row) => {
          if (!cursorMatch) return true;
          const [, column, operator, timestamp, id] = cursorMatch;
          const value = column === "first_seen_at" ? row.first_seen_at : row.delivered_at!;
          const comparison = value.localeCompare(timestamp) || row.id.localeCompare(id);
          return operator === ">" ? comparison > 0 : comparison <= 0;
        })
        .sort((a, b) => {
          const aValue = orderColumn === "first_seen_at" ? a.first_seen_at : a.delivered_at!;
          const bValue = orderColumn === "first_seen_at" ? b.first_seen_at : b.delivered_at!;
          return aValue.localeCompare(bValue) || a.id.localeCompare(b.id);
        })
        .slice(0, limit);
      return candidates as T[];
    },
  };
  const store = new AlertsStore(db);
  Object.assign(store, {
    listSubscriptions: async () => [
      {
        id: "sub-1",
        endpoint: "https://push.example/sub-1",
        p256dh: "p256dh",
        auth: "auth",
        classes: { incident: true },
        quiet: { start: "23:00", end: "07:00", tz: "UTC" },
        user_agent: null,
        created_at: "2026-11-01T00:00:00Z",
        last_seen_at: "2026-11-01T00:00:00Z",
        gone_count: 0,
      },
    ],
    logSend: async () => {},
    noteSubscriptionAlive: async () => {},
    noteSubscriptionGone: async () => "marked" as const,
  });
  return store;
}

const failedSend = async (_target: PushTarget, _payload: unknown): Promise<PushOutcome> => ({
  ok: false,
  gone: false,
  error: "push service unavailable",
});
const successfulSend = async (_target: PushTarget, _payload: unknown): Promise<PushOutcome> => ({
  ok: true,
  status: 201,
});

describe("pending alert batch fairness", () => {
  const incidents = (delivered: boolean) =>
    Array.from({ length: 21 }, (_, index) => {
      const timestamp = new Date(Date.UTC(2026, 0, 1, 0, 0, index)).toISOString();
      return {
        ...incident,
        id: `00000000-0000-0000-0000-${String(index + 1).padStart(12, "0")}`,
        first_seen_at: timestamp,
        delivered_at: delivered ? timestamp : null,
      };
    });

  test("a newer incident gets a delivery attempt after the oldest batch keeps failing", async () => {
    const rows = incidents(false);
    const store = pendingIncidentStore(rows);
    const attempted: string[] = [];

    const failingPush = async (_target: PushTarget, payload: unknown): Promise<PushOutcome> => {
      attempted.push((payload as { objectRef: string }).objectRef);
      return failedSend(_target, payload);
    };
    await runAlertTick(tickConfig, store, failingPush);
    await runAlertTick(tickConfig, store, failingPush);

    expect(attempted).toContain(`incident:${rows[20].id}`);
  });

  test("a newer incident gets an escalation attempt after the oldest batch keeps failing", async () => {
    const rows = incidents(true);
    const store = pendingIncidentStore(rows);
    const attempted: string[] = [];

    const failingPush = async (_target: PushTarget, payload: unknown): Promise<PushOutcome> => {
      attempted.push((payload as { objectRef: string }).objectRef);
      return failedSend(_target, payload);
    };
    await runAlertTick(tickConfig, store, failingPush);
    await runAlertTick(tickConfig, store, failingPush);

    expect(attempted).toContain(`incident:${rows[20].id}`);
  });
});

describe("alert tick delivery state", () => {
  test("dashboard candidates are delivered once and their state is persisted for dedupe", async () => {
    const now = Date.parse("2026-11-03T18:00:00Z");
    const { store } = alertTickStore({ deliveries: [], escalations: [] });
    const sub: PushSubscriptionRow = {
      id: "sub-dashboard",
      endpoint: "https://push.example/sub-dashboard",
      p256dh: "p256dh",
      auth: "auth",
      classes: { incident: true, "needs-drew": true },
      quiet: { start: "23:00", end: "07:00", tz: "UTC" },
      user_agent: null,
      created_at: "2026-11-01T00:00:00Z",
      last_seen_at: "2026-11-01T00:00:00Z",
      gone_count: 0,
    };
    store.listSubscriptions = async () => [sub];
    const historyRows: NotificationHistoryEntry[] = [];
    const notificationHistory: NonNullable<AlertTickOptions["notificationHistory"]> = {
      read: async () => [...historyRows],
      append: async (entry) => {
        historyRows.push(entry);
      },
    };
    let dashboardPushes = 0;
    const push: typeof successfulSend = async (_target, payload) => {
      if ((payload as { class: string }).class === "needs-drew") dashboardPushes += 1;
      return successfulSend(_target, payload);
    };
    const item = {
      id: "task-1",
      kind: "task",
      title: "Needs-Drew task",
      since: new Date(now - 60_000).toISOString(),
      state: "open",
    };
    const dashboardSnapshot = async (): Promise<NotificationSnapshot> => ({
      attention: { drew: [item], drewExtra: [] } as unknown as NotificationSnapshot["attention"],
      quota: { pools: [] } as unknown as NotificationSnapshot["quota"],
      health: { services: [] },
    });

    await runAlertTick(tickConfig, store, push, { dashboardSnapshot, notificationHistory, now });
    await runAlertTick(tickConfig, store, push, { dashboardSnapshot, notificationHistory, now: now + 60_000 });

    expect(dashboardPushes).toBe(1);
    expect(historyRows.map((row) => row.outcome)).toEqual(["sent", "suppressed-dedupe"]);
    expect(historyRows.every((row) => row.subscriptionId === sub.id)).toBe(true);
  });

  test("quiet-hour candidates persist across ticks and drain as one digest at window end", async () => {
    const now = Date.parse("2026-11-04T01:00:00Z");
    const { store } = alertTickStore({ deliveries: [], escalations: [] });
    const sub: PushSubscriptionRow = {
      id: "sub-digest",
      endpoint: "https://push.example/sub-digest",
      p256dh: "p256dh",
      auth: "auth",
      classes: { incident: true, "needs-drew": true },
      quiet: { start: "23:00", end: "07:00", tz: "UTC" },
      user_agent: null,
      created_at: "2026-11-01T00:00:00Z",
      last_seen_at: "2026-11-01T00:00:00Z",
      gone_count: 0,
    };
    store.listSubscriptions = async () => [sub];
    const historyRows: NotificationHistoryEntry[] = [];
    const notificationHistory: NonNullable<AlertTickOptions["notificationHistory"]> = {
      read: async () => [...historyRows],
      append: async (entry) => {
        historyRows.push(entry);
      },
    };
    const pushed: unknown[] = [];
    const push: typeof successfulSend = async (_target, payload) => {
      pushed.push(payload);
      return successfulSend(_target, payload);
    };
    const item = {
      id: "quiet-task",
      kind: "task",
      title: "Quiet-hour task",
      since: new Date(now - 60_000).toISOString(),
      state: "open",
    };
    const dashboardSnapshot = async (): Promise<NotificationSnapshot> => ({
      attention: { drew: [item], drewExtra: [] } as unknown as NotificationSnapshot["attention"],
      quota: { pools: [] } as unknown as NotificationSnapshot["quota"],
      health: { services: [] },
    });

    await runAlertTick(tickConfig, store, push, { dashboardSnapshot, notificationHistory, now });
    expect(pushed).toHaveLength(0);
    expect(historyRows.map((row) => row.outcome)).toEqual(["suppressed-quiet", "queued"]);

    // Recreate the tick over the same durable history at quiet-window end.
    await runAlertTick(tickConfig, store, push, { dashboardSnapshot, notificationHistory, now: Date.parse("2026-11-04T07:00:00Z") });
    expect(pushed).toHaveLength(1);
    expect(pushed[0]).toMatchObject({ class: "digest", items: [{ title: "Quiet-hour task" }] });
    expect(historyRows.map((row) => row.outcome)).toEqual(["suppressed-quiet", "queued", "sent"]);
  });

  test("all-error delivery stays pending and a later successful tick marks it delivered once", async () => {
    const { store, calls } = alertTickStore();

    await runAlertTick(tickConfig, store, failedSend);
    expect(calls.delivered).toEqual([]);
    expect(calls.sends.map((send) => send.outcome)).toEqual(["error"]);

    await runAlertTick(tickConfig, store, successfulSend);
    expect(calls.delivered).toHaveLength(1);
    expect(calls.delivered[0]).toMatchObject({
      id: incident.id,
      detail: { subscriptions: 1, sent: 1, gone: 0, errors: 0 },
    });
    expect(calls.sends.map((send) => send.outcome)).toEqual(["error", "sent"]);
  });

  test("escalation also stays pending on all-error sends and marks only after success", async () => {
    const escalation = { ...incident, delivered_at: "2026-11-01T11:00:00Z" };
    const { store, calls } = alertTickStore({ deliveries: [], escalations: [escalation] });

    await runAlertTick(tickConfig, store, failedSend);
    expect(calls.escalated).toEqual([]);

    await runAlertTick(tickConfig, store, successfulSend);
    expect(calls.escalated).toHaveLength(1);
    expect(calls.escalated[0]).toMatchObject({
      id: incident.id,
      detail: { subscriptions: 1, sent: 1, gone: 0, errors: 0, ackWindowMin: 30 },
    });
    expect(calls.sends.map((send) => send.outcome)).toEqual(["error", "sent"]);
  });

  test("zero subscriptions leave delivery and escalation untouched", async () => {
    const { store, calls } = alertTickStore({ noSubscriptions: true });

    await runAlertTick(tickConfig, store, successfulSend);
    expect(calls.delivered).toEqual([]);
    expect(calls.escalated).toEqual([]);
    expect(calls.sends).toEqual([]);
  });
});

describe("alert tick all-gone lifecycle (#6905)", () => {
  const goneSend = async (_target: PushTarget, _payload: unknown): Promise<PushOutcome> => ({
    ok: false,
    gone: true,
    status: 410,
    error: "gone",
  });

  // Stateful fake store mirroring AlertsStore.noteSubscriptionGone
  // semantics (src/alerts.ts): a gone response increments the
  // subscription strike count, the second consecutive gone prunes it,
  // and a successful send (noteSubscriptionAlive) resets the count.
  function goneLifecycleStore(options: { deliveries?: Incident[]; escalations?: Incident[] } = {}) {
    const sub: PushSubscriptionRow = {
      id: "sub-1",
      endpoint: "https://push.example/sub-1",
      p256dh: "p256dh",
      auth: "auth",
      classes: { incident: true },
      quiet: { start: "23:00", end: "07:00", tz: "UTC" },
      user_agent: null,
      created_at: "2026-11-01T00:00:00Z",
      last_seen_at: "2026-11-01T00:00:00Z",
      gone_count: 0,
    };
    let subs: PushSubscriptionRow[] = [sub];
    let strikes = 0;
    const deliveryList: Incident[] = [...(options.deliveries ?? [incident])];
    const deliveredIds = new Set<string>();
    const escalatedIds = new Set<string>();
    const calls = {
      delivered: [] as Array<{ id: string; detail: Record<string, unknown> }>,
      escalated: [] as Array<{ id: string; detail: Record<string, unknown> }>,
      sends: [] as Array<{ outcome: string; incidentId?: string }>,
      goneNotes: [] as Array<"marked" | "pruned">,
      aliveNotes: [] as string[],
      pushes: 0,
    };
    const store = {
      listSubscriptions: async () => subs,
      pendingDeliveries: async () => deliveryList.filter((i) => !deliveredIds.has(i.id)),
      pendingEscalations: async () => (options.escalations ?? []).filter((i) => !escalatedIds.has(i.id)),
      markDelivered: async (id: string, detail: Record<string, unknown>) => {
        deliveredIds.add(id);
        calls.delivered.push({ id, detail });
        return true;
      },
      markEscalated: async (id: string, detail: Record<string, unknown>) => {
        escalatedIds.add(id);
        calls.escalated.push({ id, detail });
        return true;
      },
      logSend: async (entry: { outcome: string; incidentId?: string }) => {
        calls.sends.push(entry);
      },
      noteSubscriptionAlive: async (endpoint: string) => {
        strikes = 0;
        calls.aliveNotes.push(endpoint);
      },
      noteSubscriptionGone: async (_endpoint: string) => {
        strikes += 1;
        if (strikes >= 2) {
          subs = [];
          calls.goneNotes.push("pruned");
          return "pruned" as const;
        }
        calls.goneNotes.push("marked");
        return "marked" as const;
      },
    };
    return {
      store: store as unknown as AlertsStoreType,
      calls,
      subCount: () => subs.length,
      resubscribe: () => {
        subs = [sub];
        strikes = 0;
      },
      addDelivery: (i: Incident) => {
        deliveryList.push(i);
      },
    };
  }

  test("all-gone delivery: never marked delivered, pruned on the second gone, then zero-subscription silence", async () => {
    const { store, calls, subCount, resubscribe } = goneLifecycleStore();
    const countingGone = async (target: PushTarget, payload: unknown): Promise<PushOutcome> => {
      calls.pushes += 1;
      return goneSend(target, payload);
    };

    // Tick 1: the only subscription is a dead endpoint. Gone is not a
    // send (#3690 contract), so delivery stays unmarked; one strike.
    await runAlertTick(tickConfig, store, countingGone);
    expect(calls.delivered).toEqual([]);
    expect(calls.sends.map((s) => s.outcome)).toEqual(["gone"]);
    expect(calls.goneNotes).toEqual(["marked"]);
    expect(subCount()).toBe(1);

    // Tick 2: second consecutive gone prunes the subscription; the
    // incident is STILL not marked delivered.
    await runAlertTick(tickConfig, store, countingGone);
    expect(calls.delivered).toEqual([]);
    expect(calls.sends.map((s) => s.outcome)).toEqual(["gone", "gone"]);
    expect(calls.goneNotes).toEqual(["marked", "pruned"]);
    expect(subCount()).toBe(0);

    // Tick 3: zero subscriptions — the engine does not even attempt a
    // send, and the incident stays pending (zero-subscription case).
    await runAlertTick(tickConfig, store, countingGone);
    expect(calls.pushes).toBe(2);
    expect(calls.sends).toHaveLength(2);
    expect(calls.delivered).toEqual([]);

    // A later subscription still gets the push, exactly once.
    resubscribe();
    await runAlertTick(tickConfig, store, successfulSend);
    expect(calls.delivered).toHaveLength(1);
    expect(calls.delivered[0]).toMatchObject({
      id: incident.id,
      detail: { subscriptions: 1, sent: 1, gone: 0, errors: 0 },
    });
  });

  test("all-gone escalation: never marked escalated on gone alone, pruned on the second gone", async () => {
    const escalation = { ...incident, delivered_at: "2026-11-01T11:00:00Z" };
    const { store, calls, subCount } = goneLifecycleStore({ deliveries: [], escalations: [escalation] });

    await runAlertTick(tickConfig, store, goneSend);
    expect(calls.escalated).toEqual([]);
    expect(calls.sends.map((s) => s.outcome)).toEqual(["gone"]);
    expect(calls.goneNotes).toEqual(["marked"]);
    expect(subCount()).toBe(1);

    await runAlertTick(tickConfig, store, goneSend);
    expect(calls.escalated).toEqual([]);
    expect(calls.sends.map((s) => s.outcome)).toEqual(["gone", "gone"]);
    expect(calls.goneNotes).toEqual(["marked", "pruned"]);
    expect(subCount()).toBe(0);

    // Zero subscriptions: escalation stays pending and untouched.
    await runAlertTick(tickConfig, store, goneSend);
    expect(calls.sends).toHaveLength(2);
    expect(calls.escalated).toEqual([]);
  });

  test("a successful send between gones resets the strike count (no prune)", async () => {
    const { store, calls, subCount, addDelivery } = goneLifecycleStore({ deliveries: [incident] });

    await runAlertTick(tickConfig, store, goneSend);
    expect(calls.goneNotes).toEqual(["marked"]);
    expect(calls.delivered).toEqual([]);

    // The same incident delivers on the next tick; the success notes
    // the subscription alive, resetting its gone strikes.
    await runAlertTick(tickConfig, store, successfulSend);
    expect(calls.delivered).toHaveLength(1);
    expect(calls.aliveNotes).toEqual(["https://push.example/sub-1"]);
    expect(subCount()).toBe(1);

    // A fresh incident going gone is a first strike again — marked,
    // not pruned — proving the reset landed.
    const third = { ...incident, id: "00000000-0000-0000-0000-000000000100" };
    addDelivery(third);
    await runAlertTick(tickConfig, store, goneSend);
    expect(calls.goneNotes).toEqual(["marked", "marked"]);
    expect(subCount()).toBe(1);
    expect(calls.delivered).toHaveLength(1);
  });
});

describe("test push (Drew 2026-11-01: the /alerts test button)", () => {
  test("the payload is unmistakably a test and deep-links to /alerts", () => {
    const p = buildTestPayload("https://amd-6.example.ts.net");
    expect(p.class).toBe("test");
    expect(p.objectRef).toBe(TEST_OBJECT_REF);
    expect(p.url).toBe("https://amd-6.example.ts.net/alerts");
    expect(p.title.toLowerCase()).toContain("test");
    expect(p.body.toLowerCase()).toContain("test");
  });

  const row: PushSubscriptionRow = {
    id: "99999999-8888-7777-6666-555555555555",
    endpoint: "https://push.example.test/send/abc",
    p256dh: "p",
    auth: "a",
    classes: { incident: true },
    quiet: { start: "23:00", end: "07:00", tz: "America/Chicago", enabled: true },
    user_agent: "test",
    created_at: "2026-11-01T00:00:00Z",
    last_seen_at: "2026-11-01T00:00:00Z",
    gone_count: 0,
  };

  function stubStore(subs: PushSubscriptionRow[]) {
    const sends: { class: string; objectRef: string; outcome: string }[] = [];
    const gone: string[] = [];
    const alive: string[] = [];
    const store = {
      listSubscriptions: async () => subs,
      logSend: async (e: { class: string; objectRef: string; outcome: "sent" | "gone" | "error" }) => {
        sends.push(e);
      },
      noteSubscriptionGone: async (endpoint: string) => {
        gone.push(endpoint);
        return "marked" as const;
      },
      noteSubscriptionAlive: async (endpoint: string) => {
        alive.push(endpoint);
      },
    };
    return { store, sends, gone, alive };
  }

  test("a sent test records one class 'test' sends row with outcome sent", async () => {
    const { store, sends, alive } = stubStore([row]);
    const summary = await sendTestPush(store, "https://amd-6.example.ts.net", async () => ({ ok: true, status: 201 }));
    expect(summary).toEqual({ subscriptions: 1, sent: 1, gone: 0, errors: 0 });
    expect(sends).toEqual([{ class: "test", objectRef: TEST_OBJECT_REF, outcome: "sent" }]);
    expect(alive).toEqual([row.endpoint]);
  });

  test("no subscriptions: summary is all zero and nothing is logged (the endpoint's plain-words case)", async () => {
    const { store, sends } = stubStore([]);
    const summary = await sendTestPush(store, "https://amd-6.example.ts.net", async () => ({ ok: true, status: 201 }));
    expect(summary).toEqual({ subscriptions: 0, sent: 0, gone: 0, errors: 0 });
    expect(sends).toEqual([]);
  });

  test("a gone endpoint records outcome gone and marks the subscription", async () => {
    const { store, sends, gone } = stubStore([row]);
    const summary = await sendTestPush(store, "https://amd-6.example.ts.net", async () => ({
      ok: false,
      gone: true,
      status: 410,
      error: "gone",
    }));
    expect(summary).toEqual({ subscriptions: 1, sent: 0, gone: 1, errors: 0 });
    expect(sends).toEqual([{ class: "test", objectRef: TEST_OBJECT_REF, outcome: "gone" }]);
    expect(gone).toEqual([row.endpoint]);
  });
});

describe("alerts page push panel (Drew 2026-11-01: load-order race + saved state)", () => {
  const PROV: Provenance = {
    board: "test",
    roadmap: "test",
    items: "test",
    comments: "test",
    quota: "test",
    usage: "test",
    attention: "test",
    orchestration: "test",
    dns: "test",
    registry: "test",
    aa: "test",
  };
  const subRow = (
    quiet: PushSubscriptionRow["quiet"],
    classes: Record<string, boolean> = { incident: true },
  ): PushSubscriptionRow => ({
    id: "11111111-2222-3333-4444-555555555555",
    endpoint: "https://push.example.test/send/xyz",
    p256dh: "p",
    auth: "a",
    classes,
    quiet,
    user_agent: "test",
    created_at: "2026-11-01T00:00:00Z",
    last_seen_at: "2026-11-01T12:00:00Z",
    gone_count: 0,
  });
  const page = (subs: PushSubscriptionRow[]) =>
    renderAlerts(
      { available: true, incidents: [], events: [], subscriptions: subs, ackWindowMin: 15 },
      PROV,
      "VAPIDKEY",
    );

  test("stored quiet-hours OFF renders unchecked with the time inputs disabled (no client JS needed)", () => {
    const html = page([subRow({ start: "22:30", end: "06:15", tz: "America/Chicago", enabled: false })]);
    expect(html).toContain('id="push-quiet-enabled">');
    expect(html).not.toContain('id="push-quiet-enabled" checked');
    expect(html).toContain('id="push-quiet-start" value="22:30" disabled');
    expect(html).toContain('id="push-quiet-end" value="06:15" disabled');
  });

  test("stored incident-class OFF renders its checkbox unchecked", () => {
    const html = page([
      subRow({ start: "23:00", end: "07:00", tz: "America/Chicago", enabled: true }, { incident: false }),
    ]);
    expect(html).toContain('id="push-class-incident">');
    expect(html).not.toContain('id="push-class-incident" checked');
  });

  test("stored quiet-hours ON renders checked with the stored window", () => {
    const html = page([subRow({ start: "22:30", end: "06:15", tz: "America/Chicago", enabled: true })]);
    expect(html).toContain('id="push-quiet-enabled" checked>');
    expect(html).toContain('id="push-quiet-start" value="22:30">');
  });

  test("no subscriptions: defaults render (quiet on, 23:00-07:00)", () => {
    const html = page([]);
    expect(html).toContain('id="push-quiet-enabled" checked>');
    expect(html).toContain('id="push-quiet-start" value="23:00"');
  });

  test("support is judged from the window, not from the deferred push client", () => {
    const html = page([]);
    expect(html).toContain("function pushSupportedHere()");
    expect(html).toContain('"PushManager" in window');
    expect(html).toContain("waitForPmPush");
    expect(html).not.toContain("!window.pmPush || !window.pmPush.supported()");
  });

  test("prefs application falls back to the single stored subscription (endpoint rotation)", () => {
    const html = page([subRow({ start: "23:00", end: "07:00", tz: "America/Chicago", enabled: false })]);
    expect(html).toContain("SUBS.length === 1 ? SUBS[0] : null");
  });

  test("Save preferences confirms in place instead of reloading", () => {
    const html = page([]);
    expect(html).toContain("Saved \u2014 quiet hours off, no time-of-day limit.");
    expect(html).not.toContain("updatePrefs(prefs()); location.reload()");
  });
});

describe("push subscription redaction (#3693)", () => {
  test("GET/POST serialization omits p256dh/auth values and keys, keeps non-secret fields", () => {
    const sentinelP256dh = "SENTINEL-p256dh-4f8e2c9a";
    const sentinelAuth = "SENTINEL-auth-7b1d5e3f";
    const row: PushSubscriptionRow = {
      id: "aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee",
      endpoint: "https://push.example.test/send/sentinel-3693",
      p256dh: sentinelP256dh,
      auth: sentinelAuth,
      classes: { incident: true },
      quiet: { start: "23:00", end: "07:00", tz: "America/Chicago", enabled: true },
      user_agent: "sentinel-ua",
      created_at: "2026-11-01T00:00:00Z",
      last_seen_at: "2026-11-01T12:00:00Z",
      gone_count: 0,
    };
    // Serialize exactly as the /api/push/subscriptions handlers do.
    const getBody = JSON.stringify({ subscriptions: [row].map(toPublicSubscription) });
    const postBody = JSON.stringify({ subscription: toPublicSubscription(row) });
    const expectedFields = [
      "id",
      "endpoint",
      "classes",
      "quiet",
      "user_agent",
      "created_at",
      "last_seen_at",
      "gone_count",
    ];
    for (const body of [getBody, postBody]) {
      expect(body).not.toContain(sentinelP256dh);
      expect(body).not.toContain(sentinelAuth);
      expect(body).not.toContain("p256dh");
      // "auth" as a JSON key; the word also appears inside "user_agent"
      // only as a substring, so match the quoted key.
      expect(body).not.toContain('"auth"');
      expect(body).toContain(row.endpoint);
      expect(body).toContain("sentinel-ua");
      expect(body).toContain(row.id);
      const parsed = JSON.parse(body);
      const subscription = parsed.subscriptions?.[0] ?? parsed.subscription;
      expect(Object.keys(subscription).sort()).toEqual(expectedFields.sort());
    }
    // The verbatim rows the handlers used to return still carry the
    // secrets — the old shape fails this redaction by construction.
    const verbatim = JSON.stringify({ subscriptions: [row] });
    expect(verbatim).toContain(sentinelP256dh);
    expect(verbatim).toContain(sentinelAuth);
  });
});
