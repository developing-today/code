// #14084 S2 — the unified sends ledger, the census, and the dead-push
// sentinel.
//
// Oracle: SQLite executes every statement through the same dialect
// bridge the Turso client uses (storeStatement + decodeStoreRow), so
// these tests run the store's real SQL, not a string snapshot. The
// sentinel evaluator is pure; the tick is exercised against a fake
// store with the same structural surface the engine tick uses.
import { describe, expect, test } from "bun:test";
import { Database } from "bun:sqlite";
import { decodeStoreRow, storeStatement } from "../src/turso-stores.ts";
import {
  canonicalClass,
  ensurePushRegistry,
  ItemStageStore,
  normalizeClassMap,
  readPushCensus,
  UnifiedSendStore,
} from "../src/push-registry.ts";
import { evaluatePushSentinel, runPushSentinelTick } from "../src/push-sentinel.ts";

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

const NOW = new Date("2026-11-27T12:00:00.000Z");

describe("the unified sends ledger", () => {
  test("the writer stamps sent_at; the schema refuses an undated row", async () => {
    const db = new Database(":memory:");
    const c = localClient(db);
    const sends = new UnifiedSendStore(c as never, () => NOW);
    const row = await sends.log({ class: "incident", objectRef: "o/r#1", outcome: "sent" });
    expect(row.sent_at).toBe("2026-11-27T12:00:00.000Z");
    expect(row.acted_on_at).toBeNull();
    // An explicit stamp is honored (the #14061 lesson: the writer owns the time).
    const stamped = await sends.log({
      class: "incident",
      objectRef: "o/r#2",
      outcome: "error",
      sentAt: "2026-11-26T10:00:00.000Z",
    });
    expect(stamped.sent_at).toBe("2026-11-26T10:00:00.000Z");
    // The schema requires the stamp: no default, no undated row.
    await expect(
      c.query(`insert into push_sends (class, object_ref, outcome) values ('incident', 'o/r#3', 'sent')`),
    ).rejects.toThrow();
  });

  test("classes are canonicalized on write; the ledger lists oldest first", async () => {
    const db = new Database(":memory:");
    const c = localClient(db);
    const sends = new UnifiedSendStore(c as never, () => NOW);
    const row = await sends.log({
      endpointId: "ep-1",
      class: "needs-drew",
      objectRef: "o/r#9",
      outcome: "sent",
      groupRef: "digest:2026-11-27",
    });
    expect(row.class).toBe("needs-user");
    expect(row.endpoint_id).toBe("ep-1");
    expect(row.group_ref).toBe("digest:2026-11-27");
    await sends.log({ class: "incident", objectRef: "o/r#10", outcome: "error" });
    const rows = await sends.list();
    expect(rows.map((r) => r.object_ref)).toEqual(["o/r#9", "o/r#10"]);
    expect(await sends.lastSuccessfulSendAt()).toBe("2026-11-27T12:00:00.000Z");
  });

  test("the exactly-once Ack transition stamps acted_on_at once", async () => {
    const db = new Database(":memory:");
    const c = localClient(db);
    const sends = new UnifiedSendStore(c as never, () => NOW);
    await sends.log({ class: "incident", objectRef: "a", incidentId: "inc-1", outcome: "sent" });
    await sends.log({ class: "incident", objectRef: "b", incidentId: "inc-1", outcome: "sent" });
    expect(await sends.markActedByIncident("inc-1")).toBe(2);
    expect(await sends.markActedByIncident("inc-1")).toBe(0);
    expect(await sends.markActedByIncident("inc-unknown")).toBe(0);
  });
});

describe("the class authority", () => {
  test("needs-drew means needs-user; the new key's choice wins", () => {
    expect(canonicalClass("needs-drew")).toBe("needs-user");
    expect(canonicalClass("incident")).toBe("incident");
    expect(normalizeClassMap({ incident: true, "needs-drew": false })).toEqual({
      incident: true,
      "needs-user": false,
    });
    expect(normalizeClassMap({ "needs-drew": false, "needs-user": true })).toEqual({ "needs-user": true });
  });
});

describe("the census", () => {
  test("one read: endpoints by transport/client, last success, needs-user backlog", async () => {
    const db = new Database(":memory:");
    const c = localClient(db);
    await ensurePushRegistry(c as never);
    await c.query(
      `insert into push_endpoints (id, transport, client, endpoint, p256dh, auth, created_at, last_seen_at) values ('e1', 'webpush', 'browser', 'https://example.com/e1', 'p', 'a', '2026-11-27T00:00:00.000Z', '2026-11-27T00:00:00.000Z'), ('e2', 'webpush', 'android-app', 'https://example.com/e2', 'p', 'a', '2026-11-27T00:00:00.000Z', '2026-11-27T00:00:00.000Z')`,
    );
    const sends = new UnifiedSendStore(c as never, () => NOW);
    await sends.log({ endpointId: "e1", class: "incident", objectRef: "o/r#1", outcome: "sent" });
    const stages = new ItemStageStore(c as never, () => NOW);
    await stages.escalate("o/r#7");
    await stages.setBlockedByCount("o/r#7", 3);
    const census = await readPushCensus(c as never, stages, NOW);
    expect(census).toEqual({
      available: true,
      asOf: "2026-11-27T12:00:00.000Z",
      endpoints: {
        total: 2,
        byTransportClient: [
          { transport: "webpush", client: "android-app", count: 1 },
          { transport: "webpush", client: "browser", count: 1 },
        ],
      },
      lastSuccessfulSendAt: "2026-11-27T12:00:00.000Z",
      needsUser: { open: 1, totalBlockedBy: 3 },
    });
  });
});

describe("the dead-push sentinel", () => {
  const snap = {
    liveEndpoints: 1,
    openUndelivered: 0,
    incidentsOpen: 1,
    lastSuccessAt: "2026-11-27T11:00:00.000Z",
    undated: 0,
    nowMs: NOW.getTime(),
  };

  test("quiet when delivery is live; each trip fires on its own condition", () => {
    expect(evaluatePushSentinel(snap)).toEqual([]);
    expect(evaluatePushSentinel({ ...snap, liveEndpoints: 0, openUndelivered: 2 }).map((f) => f.trip)).toEqual([
      "zero-endpoints",
    ]);
    // Zero endpoints with nothing undelivered is not an alarm.
    expect(evaluatePushSentinel({ ...snap, liveEndpoints: 0, openUndelivered: 0 })).toEqual([]);
    expect(evaluatePushSentinel({ ...snap, lastSuccessAt: "2026-11-20T12:00:00.000Z" }).map((f) => f.trip)).toEqual([
      "no-successful-send",
    ]);
    // No success ever, with incidents open, trips the stale key.
    expect(evaluatePushSentinel({ ...snap, lastSuccessAt: null }).map((f) => f.trip)).toEqual(["no-successful-send"]);
    // A fresh success with nothing else wrong stays quiet.
    expect(evaluatePushSentinel({ ...snap, incidentsOpen: 0, lastSuccessAt: null })).toEqual([]);
    expect(evaluatePushSentinel({ ...snap, undated: 2 }).map((f) => f.trip)).toEqual(["undated-rows"]);
  });

  test("the tick raises warnings only, and closes what cleared", async () => {
    const posted: { severity: string; key: string }[] = [];
    const closed: string[] = [];
    const store = {
      listSubscriptions: async () => [] as { id: string }[],
      listUnifiedSends: async () => [{ outcome: "sent", sent_at: "not-a-time" }],
      listOpenIncidents: async () => [{ status: "open", delivered_at: null }],
      postIncident: async (i: { severity: string; key: string }) => (posted.push(i), { incident: i, created: true }),
      closeOpenIncidentByKey: async (_s: string, k: string) => (closed.push(k), { incident: null, transitioned: true }),
    };
    const findings = await runPushSentinelTick(store as never, NOW);
    expect(findings.map((f) => f.trip).sort()).toEqual(["no-successful-send", "undated-rows", "zero-endpoints"]);
    // A push about no-push is a dead letter: warnings never ride push.
    expect(posted.length).toBe(3);
    expect(posted.every((p) => p.severity === "warning")).toBe(true);
    // Nothing cleared, so nothing closed.
    expect(closed).toEqual([]);
  });

  test("the tick closes cleared trips and never throws", async () => {
    const posted: unknown[] = [];
    const closed: string[] = [];
    const store = {
      listSubscriptions: async () => [{ id: "e1" }],
      listUnifiedSends: async () => [{ outcome: "sent", sent_at: NOW.toISOString() }],
      listOpenIncidents: async () => [],
      postIncident: async (i: unknown) => (posted.push(i), { incident: i, created: true }),
      closeOpenIncidentByKey: async (_s: string, k: string) => (closed.push(k), { incident: null, transitioned: true }),
    };
    await expect(runPushSentinelTick(store as never, NOW)).resolves.toEqual([]);
    expect(posted).toEqual([]);
    expect(closed.sort()).toEqual(["no-successful-send", "undated-rows", "zero-endpoints"]);
    // A store failure fails soft: delivery and the dashboard tick go on.
    const broken = {
      ...store,
      listSubscriptions: async () => {
        throw new Error("registry unreachable");
      },
    };
    await expect(runPushSentinelTick(broken as never, NOW)).resolves.toEqual([]);
  });
});
