// Alert throat (#3676) — pure-helper tests. The store itself is
// SQL-backed and verified live against muse-dev2; what can be pinned
// without a database is pinned here: SQL literal escaping, the
// class/quiet merge rules (design §1/§2), payload + deep-link shape
// (§3), the duplicate-suppression rule, and the probe's foreign-
// registration test (§6).
import { describe, expect, test } from "bun:test";
import {
  buildIncidentPayload,
  classEnabled,
  duplicateWouldRepeatDelivery,
  incidentObjectRef,
  incidentUrl,
  inQuietHours,
  mergeClasses,
  mergeQuiet,
  runAlertTick,
  validateNewIncident,
  validateNewSubscription,
  type AlertsStore,
  type Incident,
} from "../src/alerts.ts";
import { isForeignSwScript, type PushOutcome, type PushTarget } from "../src/push.ts";
import { sqlInt, sqlJson, sqlStr } from "../src/supabase.ts";
import type { Config } from "../src/config.ts";

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
    expect(mergeClasses(undefined)).toEqual({ incident: true });
    expect(mergeClasses({ incident: false })).toEqual({ incident: false });
    expect(mergeClasses({ incident: true, "quota-wall": false })).toEqual({ incident: true, "quota-wall": false });
    expect(mergeClasses({ incident: "yes" })).toEqual({ incident: true }); // non-boolean ignored
  });
  test("classEnabled: only an explicit false disables", () => {
    expect(classEnabled({ classes: {} }, "incident")).toBe(true);
    expect(classEnabled({ classes: { incident: false } }, "incident")).toBe(false);
    expect(classEnabled({ classes: { incident: true } }, "incident")).toBe(true);
  });
  test("quiet defaults 23:00–07:00 America/Chicago; partial overrides merge", () => {
    expect(mergeQuiet(undefined)).toEqual({ start: "23:00", end: "07:00", tz: "America/Chicago" });
    expect(mergeQuiet({ start: "22:30" })).toEqual({ start: "22:30", end: "07:00", tz: "America/Chicago" });
    expect(mergeQuiet({ start: "9pm" })).toEqual({ start: "23:00", end: "07:00", tz: "America/Chicago" });
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
      expect(ok.value.classes).toEqual({ incident: true });
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

function alertTickStore(options: { deliveries?: Incident[]; escalations?: Incident[]; noSubscriptions?: boolean } = {}) {
  const calls = {
    delivered: [] as Array<{ id: string; detail: Record<string, unknown> }>,
    escalated: [] as Array<{ id: string; detail: Record<string, unknown> }>,
    sends: [] as Array<{ outcome: string; incidentId?: string }>,
  };
  const store = {
    listSubscriptions: async () => options.noSubscriptions ? [] : [
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
  return { store: store as unknown as AlertsStore, calls };
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

describe("alert tick delivery state", () => {
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
