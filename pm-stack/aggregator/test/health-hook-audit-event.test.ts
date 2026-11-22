import { describe, expect, test } from "bun:test";
import { mkdtemp, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { EventsStore } from "../src/events.ts";
import { healthHookAuditEvent } from "../src/health-hook-audit-event.ts";

describe("health-hook audit event (#9571)", () => {
  const stackHealth = {
    serviceId: "opencode-working",
    name: "OpenCode working",
    state: "down" as const,
    since: "2026-11-21T12:00:00.000Z",
    detail: "connect failed",
  };

  test("maps an incident to the event-stream audit contract", () => {
    expect(
      healthHookAuditEvent({ source: "health-hook", title: "Health hook: OpenCode working is down", stackHealth }),
    ).toEqual({
      id: "health-hook/opencode-working/down/2026-11-21T12:00:00.000Z",
      at: "2026-11-21T12:00:00.000Z",
      actor: "health-hook",
      kind: "health-hook-incident",
      ref: "health-hook:opencode-working",
      summary: "Health hook: OpenCode working is down",
      attribution: { confidence: "unknown", asserted: false },
      source: { identity: "health-hook/opencode-working" },
    });
  });

  test("also records recoveries and ignores unrelated stack-health producers", () => {
    expect(
      healthHookAuditEvent({
        source: "health-hook",
        title: "Health hook: OpenCode working is up",
        stackHealth: { ...stackHealth, state: "up", since: "2026-11-21T12:05:00.000Z" },
      }),
    ).toMatchObject({ kind: "health-hook-incident", ref: "health-hook:opencode-working", actor: "health-hook" });
    expect(healthHookAuditEvent({ source: "other", title: "other producer", stackHealth })).toBeNull();
  });

  test("appends both down and recovery records to the versioned event stream", async () => {
    const dir = await mkdtemp(join(tmpdir(), "health-hook-events-"));
    try {
      const store = new EventsStore(join(dir, "events.json"));
      const down = healthHookAuditEvent({ source: "health-hook", title: "OpenCode down", stackHealth });
      const up = healthHookAuditEvent({
        source: "health-hook",
        title: "OpenCode recovered",
        stackHealth: { ...stackHealth, state: "up", since: "2026-11-21T12:05:00.000Z" },
      });
      if (!down || !up) throw new Error("health-hook events should be generated");
      await store.record(down);
      await store.record(up);
      const records = await store.list({ ref: "health-hook:opencode-working" });
      expect(records).toHaveLength(2);
      expect(records.map((event) => event.summary).sort()).toEqual(["OpenCode down", "OpenCode recovered"]);
      expect(records.every((event) => event.kind === "health-hook-incident")).toBe(true);
      expect(records.every((event) => event.attribution.confidence === "unknown" && !event.attribution.asserted)).toBe(
        true,
      );
    } finally {
      await rm(dir, { recursive: true, force: true });
    }
  });
});
