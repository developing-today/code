import { describe, expect, test } from "bun:test";
import {
  buildNotificationCandidates,
  decideNotification,
  type NotificationCandidate,
  type NotificationSnapshot,
} from "../src/notifications.ts";

const NOW = Date.parse("2026-11-03T18:00:00Z");

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
      since: new Date(NOW - 49 * 60 * 60_000).toISOString(),
      lastSeenAt: new Date(NOW).toISOString(),
      ref: "developing-today/code#3317",
    };
    const candidates = buildNotificationCandidates(
      snapshot({ attention: { drew: [], drewExtra: [item] } as unknown as NotificationSnapshot["attention"] }),
      NOW,
    );

    expect(candidates.map((entry) => entry.class)).toEqual(["needs-drew", "blocked-age"]);
    expect(candidates[0].objectRef).toBe("item:developing-today/code#3317");
    expect(candidates[1].escalationStep).toBe(48);
    expect(candidates[1].breakthrough).toBe(true);
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
      NOW,
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
      NOW,
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
    observedAt: new Date(NOW).toISOString(),
    breakthrough: false,
  };

  test("fires on entry once, suppresses a steady state, and permits a changed state after cooldown", () => {
    const history = [
      {
        at: new Date(NOW - 7 * 60 * 60_000).toISOString(),
        class: base.class,
        objectRef: base.objectRef,
        stateFingerprint: base.stateFingerprint,
        outcome: "sent" as const,
      },
    ];
    expect(decideNotification(base, [], NOW, false)).toBe("send");
    expect(decideNotification(base, history, NOW, false)).toBe("suppress-dedupe");
    expect(decideNotification({ ...base, stateFingerprint: "changed" }, history, NOW, false)).toBe("send");
  });

  test("applies cooldown and quiet-hours digest, but allows the breakthrough set", () => {
    const recent = [
      {
        at: new Date(NOW - 60 * 60_000).toISOString(),
        class: base.class,
        objectRef: base.objectRef,
        stateFingerprint: "older-state",
        outcome: "sent" as const,
      },
    ];
    expect(decideNotification({ ...base, stateFingerprint: "changed" }, recent, NOW, false)).toBe("suppress-cooldown");
    expect(decideNotification({ ...base, objectRef: "other" }, [], NOW, true)).toBe("digest");
    expect(decideNotification({ ...base, class: "quota-wall", breakthrough: true }, [], NOW, true)).toBe("send");
  });

  test("the 24-hour blocked-age reminder is digest-only; 48-hour escalation breaks through", () => {
    const age24 = { ...base, class: "blocked-age", stateFingerprint: '{"step":24}', digestOnly: true, escalationStep: 24 as const };
    const age48 = { ...age24, stateFingerprint: '{"step":48}', digestOnly: false, escalationStep: 48 as const, breakthrough: true };
    expect(decideNotification(age24, [], NOW, false)).toBe("digest");
    expect(decideNotification(age48, [], NOW, true)).toBe("send");
  });
});
