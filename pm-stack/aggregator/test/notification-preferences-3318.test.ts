// #3318 (slice (d) of the #1254 design): the notification
// preferences page — per-class toggles, quiet-hours editing,
// this-browser subscription state, and per-class test sends whose
// landings follow the design §3 deep-link contract.
import { describe, expect, test } from "bun:test";
import {
  buildClassTestPayload,
  classEnabled,
  inQuietHours,
  sendClassTestPush,
  validateNewSubscription,
  type PushSubscriptionRow,
} from "../src/alerts.ts";
import {
  classTestObjectRef,
  isNotificationTestClass,
  NOTIFICATION_CLASSES,
  NOTIFICATION_CLASS_DEFAULTS,
  notificationTestLanding,
} from "../src/notifications.ts";
import { renderNotificationPreferences } from "../src/render.ts";
import { manifestRouteFor } from "../src/pages.ts";
import type { Provenance } from "../src/model.ts";

const ORIGIN = "https://amd-6.example.ts.net";
const PROV: Provenance = {
  board: "test",
  roadmap: "test",
  items: "test",
  comments: "test",
  quota: "test",
  usage: "test",
  attention: "test",
  orchestration: "test",
} as Provenance;

const row = (overrides: Partial<PushSubscriptionRow> = {}): PushSubscriptionRow => ({
  id: "99999999-8888-7777-6666-555555555555",
  endpoint: "https://push.example.test/send/abc",
  p256dh: "p",
  auth: "a",
  classes: { incident: true, "quota-wall": true },
  quiet: { start: "22:30", end: "06:15", tz: "America/Chicago", enabled: true },
  user_agent: "test",
  created_at: "2026-11-01T00:00:00Z",
  last_seen_at: "2026-11-01T00:00:00Z",
  gone_count: 0,
  ...overrides,
});

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

describe("preference class metadata (#3318)", () => {
  test("the page's classes are exactly the stored-defaults keys", () => {
    expect(NOTIFICATION_CLASSES.map((c) => c.id).sort()).toEqual(Object.keys(NOTIFICATION_CLASS_DEFAULTS).sort());
  });

  test("test-class validation accepts every preference class plus digest, rejects anything else", () => {
    for (const c of NOTIFICATION_CLASSES) expect(isNotificationTestClass(c.id)).toBe(true);
    expect(isNotificationTestClass("digest")).toBe(true);
    expect(isNotificationTestClass("test")).toBe(false);
    expect(isNotificationTestClass("nope")).toBe(false);
    expect(isNotificationTestClass(42)).toBe(false);
    expect(isNotificationTestClass(undefined)).toBe(false);
  });
});

describe("deep-link landings for per-class tests (design §3)", () => {
  test("each class lands on its own triage-capable view, never the origin root", () => {
    expect(notificationTestLanding("incident", ORIGIN)).toBe(`${ORIGIN}/alerts#incident-test`);
    expect(notificationTestLanding("quota-wall", ORIGIN)).toBe(`${ORIGIN}/quota`);
    expect(notificationTestLanding("quota-near-wall", ORIGIN)).toBe(`${ORIGIN}/quota`);
    expect(notificationTestLanding("stack-health", ORIGIN)).toBe(`${ORIGIN}/health`);
    expect(notificationTestLanding("needs-drew", ORIGIN)).toBe(`${ORIGIN}/attention#test-needs-drew`);
    expect(notificationTestLanding("blocked-age", ORIGIN)).toBe(`${ORIGIN}/attention#test-blocked-age`);
    for (const c of NOTIFICATION_CLASSES) {
      expect(notificationTestLanding(c.id, ORIGIN)).not.toBe(ORIGIN);
      expect(notificationTestLanding(c.id, ORIGIN)).not.toBe(`${ORIGIN}/`);
    }
  });

  test("the digest landing is the only list-shaped one, and it is filtered to its refs", () => {
    const url = notificationTestLanding("digest", ORIGIN);
    expect(url.startsWith(`${ORIGIN}/attention?refs=`)).toBe(true);
    expect(decodeURIComponent(url.split("refs=")[1])).toBe("test:needs-drew,test:blocked-age");
  });

  test("a real candidate's own URL wins over the class fallback", () => {
    const candidate = { objectRef: "quota:pool-1", title: "Quota wall: pool-1", url: "/quota" };
    const payload = buildClassTestPayload("quota-wall", ORIGIN, candidate);
    expect(payload.url).toBe(`${ORIGIN}/quota`);
    expect(payload.objectRef).toBe("quota:pool-1");
    const abs = buildClassTestPayload("needs-drew", ORIGIN, {
      objectRef: "session:s1",
      title: "Session one",
      url: "/sessions?id=s1",
    });
    expect(abs.url).toBe(`${ORIGIN}/sessions?id=s1`);
    expect(abs.body).toContain("Session one");
  });

  test("the fallback payload is unmistakably a test and names its test item", () => {
    const p = buildClassTestPayload("stack-health", ORIGIN);
    expect(p.class).toBe("stack-health");
    expect(p.objectRef).toBe(classTestObjectRef("stack-health"));
    expect(p.objectRef).toBe("test:stack-health");
    expect(p.title.toLowerCase()).toContain("test");
    expect(p.body).toContain("test:stack-health");
  });
});

describe("per-class test sends honour the stored toggles", () => {
  test("a subscription with the class off receives nothing and logs nothing", async () => {
    const off = row({ classes: { incident: true, "quota-wall": false } });
    const { store, sends } = stubStore([off]);
    const summary = await sendClassTestPush(store, ORIGIN, "quota-wall", async () => ({ ok: true, status: 201 }));
    expect(summary.subscriptions).toBe(1);
    expect(summary.eligible).toBe(0);
    expect(summary.sent).toBe(0);
    expect(sends).toEqual([]);
  });

  test("a subscription with the class on gets the test, logged under the class with the test ref", async () => {
    const { store, sends, alive } = stubStore([row()]);
    const summary = await sendClassTestPush(store, ORIGIN, "quota-wall", async () => ({ ok: true, status: 201 }));
    expect(summary.eligible).toBe(1);
    expect(summary.sent).toBe(1);
    expect(summary.url).toBe(`${ORIGIN}/quota`);
    expect(sends).toEqual([{ class: "quota-wall", objectRef: "test:quota-wall", outcome: "sent" }]);
    expect(alive).toEqual(["https://push.example.test/send/abc"]);
  });

  test("the digest test reaches every subscription (digest is a delivery shape, not a class)", async () => {
    const off = row({ classes: { incident: false } });
    const { store, sends } = stubStore([off]);
    const summary = await sendClassTestPush(store, ORIGIN, "digest", async () => ({ ok: true, status: 201 }));
    expect(summary.eligible).toBe(1);
    expect(summary.sent).toBe(1);
    expect(sends).toEqual([{ class: "digest", objectRef: "test:digest", outcome: "sent" }]);
  });

  test("a gone endpoint is logged gone and marked", async () => {
    const { store, sends, gone } = stubStore([row()]);
    const summary = await sendClassTestPush(store, ORIGIN, "incident", async () => ({
      ok: false,
      gone: true,
      status: 410,
      error: "gone",
    }));
    expect(summary.gone).toBe(1);
    expect(sends).toEqual([{ class: "incident", objectRef: "test:incident", outcome: "gone" }]);
    expect(gone).toEqual(["https://push.example.test/send/abc"]);
  });
});

describe("a saved toggle is the next engine decision (acceptance 1)", () => {
  test("the subscription-save validation carries class + quiet choices into the engine's checks", () => {
    const parsed = validateNewSubscription({
      endpoint: "https://push.example.test/send/abc",
      keys: { p256dh: "p", auth: "a" },
      classes: { "quota-wall": false },
      quiet: { enabled: false, start: "23:00", end: "07:00", tz: "America/Chicago" },
    });
    expect(parsed.ok).toBe(true);
    if (!parsed.ok) return;
    // classEnabled is the exact check the engine's per-subscription
    // candidate filter applies.
    expect(classEnabled(parsed.value, "quota-wall")).toBe(false);
    expect(classEnabled(parsed.value, "incident")).toBe(true);
    // inQuietHours is the exact check the engine applies per tick.
    expect(inQuietHours(parsed.value.quiet, new Date("2026-11-03T05:00:00Z"))).toBe(false);
  });
});

describe("the preferences page", () => {
  test("renders a toggle and a test button per class, the digest test, and the saved quiet window", () => {
    const html = renderNotificationPreferences({ available: true, subscriptions: [row()] }, PROV, "VAPID-TEST");
    for (const c of NOTIFICATION_CLASSES) {
      expect(html).toContain(`id="pref-class-${c.id}"`);
      expect(html).toContain(`data-test-class="${c.id}"`);
    }
    expect(html).toContain('data-test-class="digest"');
    expect(html).toContain('id="pref-quiet-enabled"');
    expect(html).toContain('value="22:30"');
    expect(html).toContain('value="06:15"');
    expect(html).toContain('id="pref-save"');
    expect(html).toContain('id="pref-enable"');
    expect(html).toContain('id="pref-status"');
    // The stored class-off state renders unticked.
    const htmlOff = renderNotificationPreferences(
      { available: true, subscriptions: [row({ classes: { incident: true, "quota-wall": false } })] },
      PROV,
      "VAPID-TEST",
    );
    expect(htmlOff).toContain('id="pref-class-quota-wall">');
    expect(htmlOff).not.toContain('id="pref-class-quota-wall" checked');
  });

  test("saving posts every class plus quiet through pmPush.updatePrefs", () => {
    const html = renderNotificationPreferences({ available: true, subscriptions: [] }, PROV, "VAPID-TEST");
    expect(html).toContain("window.pmPush.updatePrefs(p)");
    expect(html).toContain("CLASS_IDS");
    expect(html).toContain("/api/push/test-class");
  });

  test("unavailable state says the ledger is unreachable, with no form", () => {
    const html = renderNotificationPreferences({ available: false, subscriptions: [] }, PROV, "VAPID-TEST");
    expect(html).toContain("unreachable");
    expect(html).not.toContain('id="pref-save"');
  });

  test("the page is reachable in the manifest as a sub-route of /alerts", () => {
    expect(manifestRouteFor("/alerts/preferences")).toBe("/alerts");
  });
});
