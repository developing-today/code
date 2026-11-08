import { describe, expect, test } from "bun:test";
import { mkdtemp, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import {
  buildNotificationCandidates,
  decideNotification,
  nextNotificationDigestAt,
  NOTIFICATION_CLASS_DEFAULTS,
  rollupNotificationSends,
  sendTimestamp,
  type NotificationCandidate,
  type NotificationSnapshot,
} from "../src/notifications.ts";
import { AlertsStore, type NotificationSendRow } from "../src/alerts.ts";
import { renderNotifications } from "../src/render.ts";
import { JsonlNotificationHistory } from "../src/notification-history.ts";
import type { Provenance } from "../src/model.ts";
import type { SupabaseClient } from "../src/supabase.ts";

const TASK_NOW = Date.parse("2026-11-03T18:00:00Z");

function snapshot(overrides: Partial<NotificationSnapshot> = {}): NotificationSnapshot {
  return {
    attention: { drew: [], drewExtra: [] } as unknown as NotificationSnapshot["attention"],
    quota: { pools: [] } as unknown as NotificationSnapshot["quota"],
    health: { services: [] },
    ...overrides,
  };
}

describe("dashboard notification candidate derivation", () => {
  test("includes non-session needs-drew items and emits the 48-hour breakthrough step", () => {
    const item = {
      id: "developing-today/code#3317",
      kind: "task",
      title: "Queue engine task",
      source: "github",
      reason: "Waiting for a decision",
      since: new Date(TASK_NOW - 49 * 60 * 60_000).toISOString(),
      lastSeenAt: new Date(TASK_NOW).toISOString(),
      ref: "developing-today/code#3317",
    };
    const candidates = buildNotificationCandidates(
      snapshot({ attention: { drew: [], drewExtra: [item] } as unknown as NotificationSnapshot["attention"] }),
      TASK_NOW,
    );

    expect(candidates.map((entry) => entry.class)).toEqual(["needs-drew", "blocked-age"]);
    expect(candidates[0].objectRef).toBe("item:developing-today/code#3317");
    expect(candidates[1].escalationStep).toBe(48);
    expect(candidates[1].breakthrough).toBe(true);
  });

  test("a later Drew action/comment on the matched dashboard object stops the needs-Drew ladder", () => {
    const since = new Date(TASK_NOW - 3 * 24 * 60 * 60_000).toISOString();
    const item = {
      id: "developing-today/code#3317",
      kind: "task",
      title: "Queue engine task",
      since,
      ref: "developing-today/code#3317",
    };
    const acted = buildNotificationCandidates(
      snapshot({
        attention: { drew: [], drewExtra: [item] } as unknown as NotificationSnapshot["attention"],
        drewActivity: [{ objectRef: "gh:developing-today/code#3317", at: new Date(TASK_NOW - 60_000).toISOString() }],
      }),
      TASK_NOW,
    );
    const preEntry = buildNotificationCandidates(
      snapshot({
        attention: { drew: [], drewExtra: [item] } as unknown as NotificationSnapshot["attention"],
        drewActivity: [{ objectRef: "gh:developing-today/code#3317", at: new Date(TASK_NOW - 4 * 24 * 60 * 60_000).toISOString() }],
      }),
      TASK_NOW,
    );

    expect(acted).toHaveLength(0);
    expect(preEntry.some((entry) => entry.class === "blocked-age")).toBe(true);
  });

  test("emits one digest-only needs-Drew reminder per week after the 48-hour breakthrough", () => {
    const since = TASK_NOW - (48 * 60 * 60_000 + 8 * 24 * 60 * 60_000);
    const item = {
      id: "developing-today/code#3317",
      kind: "task",
      title: "Queue engine task",
      since: new Date(since).toISOString(),
      lastSeenAt: new Date(TASK_NOW).toISOString(),
      ref: "developing-today/code#3317",
    };
    const make = (now: number) =>
      buildNotificationCandidates(
        snapshot({ attention: { drew: [], drewExtra: [item] } as unknown as NotificationSnapshot["attention"] }),
        now,
      ).filter((entry) => entry.class === "blocked-age" && entry.escalationStep === "weekly");

    const first = make(TASK_NOW);
    const nextWeek = make(TASK_NOW + 7 * 24 * 60 * 60_000);
    expect(first).toHaveLength(1);
    expect(first[0]).toMatchObject({ digestOnly: true, breakthrough: false, escalationStep: "weekly" });
    expect(decideNotification(first[0], [], TASK_NOW, false)).toBe("digest");
    expect(nextWeek[0].stateFingerprint).not.toBe(first[0].stateFingerprint);
  });

  test("only an explicitly sole-door pool gets near-wall warnings; wall candidates need time before reset", () => {
    const quota = {
      pools: [
        {
          id: "sole",
          provider: "Example",
          plan: "Pro",
          state: "near",
          windows: [{ name: "weekly", unit: "requests", limit: 100, used: 90, percent: 90, resetsAt: "2026-11-10T00:00:00Z" }],
        },
        {
          id: "multi",
          provider: "Other",
          plan: "Plus",
          state: "walled",
          windows: [{ name: "weekly", unit: "tokens", limit: 100, used: 100, percent: 100, resetsAt: "2026-11-05T00:00:00Z" }],
        },
      ],
    };
    const candidates = buildNotificationCandidates(
      snapshot({ quota: quota as unknown as NotificationSnapshot["quota"], soleDoorPoolIds: new Set(["sole"]) }),
      TASK_NOW,
    );

    expect(candidates.map((entry) => [entry.class, entry.objectRef])).toEqual([
      ["quota-near-wall", "quota:sole"],
      ["quota-wall", "quota:multi"],
    ]);
  });

  test("reports failing stack services, not unknown health", () => {
    const health = {
      services: [
        { def: { id: "aggregator", name: "Aggregator" }, state: "down", ok: false, detail: "probe failed" },
        { def: { id: "unknown", name: "Unknown" }, state: "unknown", ok: null, detail: "not measured" },
      ],
    };
    const candidates = buildNotificationCandidates(
      snapshot({ health: health as unknown as NotificationSnapshot["health"] }),
      TASK_NOW,
    );

    expect(candidates).toHaveLength(1);
    expect(candidates[0]).toMatchObject({ class: "stack-health", objectRef: "health:aggregator" });
  });
});

describe("notification transition policy", () => {
  const base: NotificationCandidate = {
    class: "needs-drew",
    objectRef: "item:task-1",
    stateFingerprint: "enter",
    title: "Task 1",
    body: "Waiting",
    url: "/attention#task-1",
    observedAt: new Date(TASK_NOW).toISOString(),
    breakthrough: false,
  };

  test("fires on entry once, suppresses a steady state, and permits a changed state after cooldown", () => {
    const history = [
      {
        at: new Date(TASK_NOW - 7 * 60 * 60_000).toISOString(),
        class: base.class,
        objectRef: base.objectRef,
        stateFingerprint: base.stateFingerprint,
        outcome: "sent" as const,
      },
    ];
    expect(decideNotification(base, [], TASK_NOW, false)).toBe("send");
    expect(decideNotification(base, history, TASK_NOW, false)).toBe("suppress-dedupe");
    expect(decideNotification({ ...base, stateFingerprint: "changed" }, history, TASK_NOW, false)).toBe("send");
  });

  test("applies cooldown and quiet-hours digest, but allows the breakthrough set", () => {
    const recent = [
      {
        at: new Date(TASK_NOW - 60 * 60_000).toISOString(),
        class: base.class,
        objectRef: base.objectRef,
        stateFingerprint: "older-state",
        outcome: "sent" as const,
      },
    ];
    expect(decideNotification({ ...base, stateFingerprint: "changed" }, recent, TASK_NOW, false)).toBe("suppress-cooldown");
    expect(decideNotification({ ...base, objectRef: "other" }, [], TASK_NOW, true)).toBe("digest");
    expect(decideNotification({ ...base, class: "quota-wall", breakthrough: true }, [], TASK_NOW, true)).toBe("send");
  });

  test("the 24-hour blocked-age reminder is digest-only; 48-hour escalation breaks through", () => {
    const age24: NotificationCandidate = { ...base, class: "blocked-age", stateFingerprint: '{"step":24}', digestOnly: true, escalationStep: 24 as const };
    const age48: NotificationCandidate = { ...age24, stateFingerprint: '{"step":48}', digestOnly: false, escalationStep: 48 as const, breakthrough: true };
    expect(decideNotification(age24, [], TASK_NOW, false)).toBe("digest");
    expect(decideNotification(age48, [], TASK_NOW, true)).toBe("send");
  });

  test("digest scheduling uses each subscription timezone and the next noon/window-end boundary", () => {
    const quiet = { start: "23:00", end: "07:00", tz: "America/Chicago" };
    expect(new Date(nextNotificationDigestAt(Date.parse("2026-11-04T12:00:00Z"), quiet)).toISOString()).toBe("2026-11-04T13:00:00.000Z");
    expect(new Date(nextNotificationDigestAt(Date.parse("2026-11-04T14:00:00Z"), quiet)).toISOString()).toBe("2026-11-04T18:00:00.000Z");
    expect(new Date(nextNotificationDigestAt(Date.parse("2026-11-04T20:00:00Z"), quiet)).toISOString()).toBe("2026-11-05T13:00:00.000Z");
  });

  test("sole-door near-wall class is enabled for new subscriptions by the recommended defaults", () => {
    expect(NOTIFICATION_CLASS_DEFAULTS["quota-near-wall"]).toBe(true);
  });
});

describe("JsonlNotificationHistory", () => {
  test("records a decision durably and reloads it after store recreation", async () => {
    const dir = await mkdtemp(join(tmpdir(), "notification-history-"));
    const file = join(dir, "notifications.jsonl");
    const entry = {
      at: "2026-11-04T12:00:00.000Z",
      class: "needs-drew" as const,
      objectRef: "item:task-1",
      stateFingerprint: "entered-needs-drew",
      outcome: "sent" as const,
      subscriptionId: "sub-1",
    };
    try {
      const history = new JsonlNotificationHistory(file);
      await history.append(entry);

      expect(await new JsonlNotificationHistory(file).read()).toEqual([entry]);
    } finally {
      await rm(dir, { recursive: true, force: true });
    }
  });
});

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
