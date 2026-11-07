// Notification measurement (#3319): the sent-vs-acted-on roll-up over
// the notification_sends ledger. The roll-up is pure, so the counting
// rules are pinned here with a hand-checked fixture; the store read
// is pinned against a fake Supabase client.
import { describe, expect, test } from "bun:test";
import { AlertsStore, type NotificationSendRow } from "../src/alerts.ts";
import { rollupNotificationSends, sendTimestamp } from "../src/notifications.ts";
import { renderNotifications } from "../src/render.ts";
import type { Provenance } from "../src/model.ts";
import type { SupabaseClient } from "../src/supabase.ts";

const NOW = new Date("2026-11-03T12:00:00Z");
// Rolling window: [2026-10-06T12:00Z, 2026-11-03T12:00Z], buckets of
// 7 days starting Oct 6, Oct 13, Oct 20, Oct 27.

const PROV: Provenance = {
  board: "x",
  roadmap: "x",
  items: "x",
  comments: "x",
  quota: "x",
  usage: "x",
  attention: "x",
  orchestration: "x",
  dns: "x",
  registry: "x",
  aa: "x",
};

function send(partial: Partial<NotificationSendRow>): NotificationSendRow {
  return {
    class: "incident",
    object_ref: "incident:x",
    incident_id: "inc-1",
    outcome: "sent",
    created_at: "2026-11-01T00:00:00Z",
    acted_on_at: null,
    ...partial,
  };
}

describe("rollupNotificationSends", () => {
  test("hand-checked fixture week: per-class, per-week sent/acted/ratio", () => {
    const rows: NotificationSendRow[] = [];
    // Week of Oct 27 (newest bucket): 10 incident sends, 3 acked in 24h.
    for (let i = 0; i < 10; i++) {
      rows.push(
        send({
          created_at: "2026-10-28T08:00:00Z",
          acted_on_at: i < 3 ? "2026-10-28T20:00:00Z" : null,
        }),
      );
    }
    // Week of Oct 6 (oldest bucket): 2 incident sends, 1 acked.
    rows.push(send({ created_at: "2026-10-07T08:00:00Z", acted_on_at: "2026-10-07T09:00:00Z" }));
    rows.push(send({ created_at: "2026-10-07T10:00:00Z" }));
    const rollup = rollupNotificationSends(rows, NOW);
    expect(rollup.windowStart).toBe("2026-10-06T12:00:00.000Z");
    expect(rollup.classes).toHaveLength(1);
    const incident = rollup.classes[0];
    expect(incident.class).toBe("incident");
    expect(incident.weeks.map((w) => w.sent)).toEqual([2, 0, 0, 10]);
    expect(incident.weeks.map((w) => w.acted)).toEqual([1, 0, 0, 3]);
    expect(incident.weeks[0].start).toBe("2026-10-06T12:00:00.000Z");
    expect(incident.weeks[3].start).toBe("2026-10-27T12:00:00.000Z");
    expect(incident.sent).toBe(12);
    expect(incident.acted).toBe(4);
    expect(incident.ratio).toBeCloseTo(4 / 12);
    expect(incident.proposal).toBe(false);
  });

  test("only outcome=sent counts; suppressed/gone/error never enter the denominator", () => {
    const rows = [
      send({ outcome: "suppressed-quiet", created_at: "2026-11-01T00:00:00Z" }),
      send({ outcome: "suppressed-dedupe", created_at: "2026-11-01T00:00:00Z" }),
      send({ outcome: "gone", created_at: "2026-11-01T00:00:00Z" }),
      send({ outcome: "error", created_at: "2026-11-01T00:00:00Z" }),
      send({ outcome: "sent", created_at: "2026-11-01T00:00:00Z", acted_on_at: "2026-11-01T01:00:00Z" }),
    ];
    const rollup = rollupNotificationSends(rows, NOW);
    expect(rollup.classes).toHaveLength(1);
    expect(rollup.classes[0].sent).toBe(1);
    expect(rollup.classes[0].acted).toBe(1);
    expect(rollup.classes[0].ratio).toBe(1);
  });

  test("the 24-hour acted window is inclusive at exactly 24h and closed after it", () => {
    const at = "2026-11-01T00:00:00Z";
    const rows = [
      send({ created_at: at, acted_on_at: "2026-11-02T00:00:00Z" }), // exactly 24h — counts
      send({ created_at: at, acted_on_at: "2026-11-02T00:00:01Z" }), // 24h + 1s — does not
      send({ created_at: at, acted_on_at: "2026-10-31T23:59:59Z" }), // before the send — does not
      send({ created_at: at, acted_on_at: null }), // never acked — does not
    ];
    const rollup = rollupNotificationSends(rows, NOW);
    expect(rollup.classes[0].sent).toBe(4);
    expect(rollup.classes[0].acted).toBe(1);
  });

  test("the rolling four-week window excludes older sends and future sends", () => {
    const rows = [
      send({ created_at: "2026-10-06T11:59:59Z", acted_on_at: "2026-10-06T12:30:00Z" }), // just before the window
      send({ created_at: "2026-10-06T12:00:00Z" }), // window start — included, oldest bucket
      send({ created_at: "2026-11-03T12:00:01Z" }), // after `now` — excluded
    ];
    const rollup = rollupNotificationSends(rows, NOW);
    expect(rollup.classes[0].sent).toBe(1);
    expect(rollup.classes[0].weeks[0].sent).toBe(1);
  });

  test("classes group separately; an undated sent row is counted as excluded, not as a send", () => {
    const rows = [
      send({ class: "test", object_ref: "test:push", incident_id: null, created_at: "2026-11-02T00:00:00Z" }),
      send({ class: "incident", created_at: null, sent_at: null }),
    ];
    const rollup = rollupNotificationSends(rows, NOW);
    expect(rollup.classes.map((c) => c.class)).toEqual(["test"]);
    expect(rollup.undated).toBe(1);
  });

  test("proposal: under 10% over the window proposes; exactly 10% and zero sends do not", () => {
    const rows: NotificationSendRow[] = [];
    for (let i = 0; i < 20; i++) {
      rows.push(send({ class: "digest", created_at: "2026-11-01T00:00:00Z", acted_on_at: i === 0 ? "2026-11-01T01:00:00Z" : null }));
    }
    for (let i = 0; i < 10; i++) {
      rows.push(send({ class: "edge", created_at: "2026-11-01T00:00:00Z", acted_on_at: i === 0 ? "2026-11-01T01:00:00Z" : null }));
    }
    const rollup = rollupNotificationSends(rows, NOW);
    const digest = rollup.classes.find((c) => c.class === "digest")!;
    const edge = rollup.classes.find((c) => c.class === "edge")!;
    expect(digest.ratio).toBeCloseTo(0.05);
    expect(digest.proposal).toBe(true);
    expect(edge.ratio).toBeCloseTo(0.1);
    expect(edge.proposal).toBe(false);
  });

  test("sent_at wins over created_at when a row carries both", () => {
    const row = send({ sent_at: "2026-10-07T00:00:00Z", created_at: "2026-11-02T00:00:00Z" });
    expect(sendTimestamp(row)).toBe("2026-10-07T00:00:00Z");
    const rollup = rollupNotificationSends([row], NOW);
    expect(rollup.classes[0].weeks[0].sent).toBe(1);
  });
});

describe("AlertsStore.listSends", () => {
  test("reads the notification_sends ledger", async () => {
    const rows = [send({})];
    const db: SupabaseClient = {
      ref: "test",
      async query<T>(sql: string): Promise<T[]> {
        expect(sql).toContain("alerts.notification_sends");
        return rows as T[];
      },
    };
    const store = new AlertsStore(db);
    expect(await store.listSends()).toEqual(rows);
  });
});

describe("renderNotifications", () => {
  test("renders the roll-up, the attribution note, and a display-only proposal", () => {
    const rows: NotificationSendRow[] = [];
    for (let i = 0; i < 20; i++) {
      rows.push(send({ class: "digest", created_at: "2026-11-01T00:00:00Z", acted_on_at: i === 0 ? "2026-11-01T01:00:00Z" : null }));
    }
    const html = renderNotifications({ available: true, rollup: rollupNotificationSends(rows, NOW) }, PROV);
    expect(html).toContain("digest");
    expect(html).toContain("Proposal: default class");
    expect(html).toContain("never changes a preference or a default");
    expect(html).toContain("acted_on_at");
    expect(html).toContain("/api/notifications.json");
  });

  test("unavailable renders the paused message, not an empty roll-up", () => {
    const html = renderNotifications({ available: false, rollup: null }, PROV);
    expect(html).toContain("unreachable");
  });
});
