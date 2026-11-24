// #8569 Phase 2 step 1 — oracles for the notify-relay client
// (design record doc/2026-11-17_design_pm8569_notifications_phase2.md
// §1):
//  - the mirror is the COMPLETE set under a monotonic seq, and
//    only delivery fields cross (store bookkeeping stays home);
//  - an unchanged set is not re-posted; a changed set takes the
//    next seq; a failed POST retries with the SAME seq (the edge
//    acks an equal seq as a duplicate);
//  - a 409 (our counter fell behind the edge's) re-adopts the
//    edge's seq from /status and the next sync lands above it;
//  - everything is fail-soft: no secret, no store, or a dead
//    edge never throws into the aggregator.
import { afterEach, describe, expect, test } from "bun:test";
import { mkdtemp, readFile, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import type { IncidentMirrorRow, PushSubscriptionRow } from "../src/alerts.ts";
import type { NotificationCandidate } from "../src/notifications.ts";
import { buildHeartbeatBody, buildSnapshotBody, incidentMirrorEvent, NotifyRelayClient } from "../src/notify-relay.ts";

const row = (over: Partial<PushSubscriptionRow> = {}): PushSubscriptionRow => ({
  id: "sub-1",
  endpoint: "https://fcm.googleapis.com/fcm/send/fixture",
  p256dh: "p256dh-fixture",
  auth: "auth-fixture",
  classes: { incident: true, "needs-drew": true },
  quiet: { start: "23:00", end: "07:00", tz: "America/Chicago", enabled: true },
  user_agent: "fixture-agent",
  created_at: "2026-11-20T00:00:00Z",
  last_seen_at: "2026-11-20T00:00:00Z",
  gone_count: 0,
  ...over,
});

interface RecordedCall {
  url: string;
  method: string;
  authorization: string | null;
  body: Record<string, unknown> | null;
}

const clients: NotifyRelayClient[] = [];
const dirs: string[] = [];
afterEach(async () => {
  for (const client of clients.splice(0)) client.stop();
  for (const dir of dirs.splice(0)) await rm(dir, { recursive: true, force: true });
});

async function makeClient(
  handler: (call: RecordedCall) => { status: number; body?: unknown },
  opts: {
    rows?: PushSubscriptionRow[];
    storeNull?: boolean;
    secret?: string | null;
    blockStateFile?: boolean;
  } = {},
) {
  const dir = await mkdtemp(join(tmpdir(), "notify-relay-test-"));
  dirs.push(dir);
  const blocker = join(dir, "state-parent-blocker");
  if (opts.blockStateFile) await writeFile(blocker, "file where a parent directory is required");
  const stateFile = opts.blockStateFile ? join(blocker, "notify-relay.json") : join(dir, "notify-relay.json");
  const calls: RecordedCall[] = [];
  const rows = opts.rows ?? [row()];
  const client = new NotifyRelayClient({
    url: "https://pm-notify.example.workers.dev",
    stateFile,
    getStore: async () => (opts.storeNull ? null : { listSubscriptions: async () => rows }),
    resolveSecret: async () => (opts.secret === undefined ? "test-secret" : opts.secret),
    fetchImpl: (async (url: string, init: RequestInit) => {
      const call: RecordedCall = {
        url,
        method: init.method ?? "GET",
        authorization: new Headers(init.headers).get("authorization"),
        body: typeof init.body === "string" ? JSON.parse(init.body) : null,
      };
      calls.push(call);
      const out = handler(call);
      return {
        status: out.status,
        ok: out.status >= 200 && out.status < 300,
        json: async () => out.body ?? {},
      } as Response;
    }) as typeof fetch,
    mintEpoch: () => "epoch-test",
    now: () => Date.parse("2026-11-23T12:00:00Z"),
  });
  clients.push(client);
  return { client, calls, rows, stateFile };
}

const okHandler = (call: RecordedCall) =>
  call.url.endsWith("/heartbeat")
    ? { status: 200, body: { ok: true, pendingAnswers: 0 } }
    : { status: 200, body: { ok: true } };

describe("snapshot + heartbeat bodies (design §1 shapes)", () => {
  test("the snapshot carries only delivery fields, under the given seq", () => {
    const body = buildSnapshotBody([row()], 4);
    expect(body.snapshotSeq).toBe(4);
    expect(body.subscriptions).toEqual([
      {
        endpoint: "https://fcm.googleapis.com/fcm/send/fixture",
        p256dh: "p256dh-fixture",
        auth: "auth-fixture",
        classes: { incident: true, "needs-drew": true },
        quiet: { start: "23:00", end: "07:00", tz: "America/Chicago", enabled: true },
      },
    ]);
  });

  test("the heartbeat body is {stackEpoch, seq, at}", () => {
    expect(buildHeartbeatBody("epoch-x", 3, "2026-11-23T12:00:00Z")).toEqual({
      stackEpoch: "epoch-x",
      seq: 3,
      at: "2026-11-23T12:00:00Z",
    });
  });
});

describe("incident ledger replay (authoritative transition payload)", () => {
  test("an opened critical event is mirrored by its event id even if the incident has since closed", () => {
    const row: IncidentMirrorRow = {
      id: "incident-42",
      source: "sentinel",
      incident_key: "db-down",
      severity: "critical",
      title: "Database unavailable",
      body: "Connection refused",
      status: "closed",
      first_seen_at: "2026-11-23T11:00:00.000Z",
      last_seen_at: "2026-11-23T11:10:00.000Z",
      delivered_at: null,
      escalated_at: null,
      acked_at: null,
      closed_at: "2026-11-23T11:10:00.000Z",
      occurrences: 1,
      mirror_event_id: 91,
      mirror_event_at: "2026-11-23T11:00:00.000Z",
      mirror_event_kind: "opened",
    };

    expect(incidentMirrorEvent(row, "https://pm.example.test")).toMatchObject({
      mirrorId: "alerts-incident-event:91",
      objectRef: "incident:incident-42",
      transition: "opened",
      raisedAt: "2026-11-23T11:00:00.000Z",
      payload: {
        class: "incident",
        url: "https://pm.example.test/alerts#incident-incident-42",
        title: "Database unavailable",
      },
    });
  });
});

describe("the client's contact discipline", () => {
  test("first beat: snapshot seq 1 lands before the heartbeat, both Bearer-authed; the seq persists", async () => {
    const { client, calls, stateFile } = await makeClient(okHandler);
    await client.start();
    expect(calls.map((c) => new URL(c.url).pathname)).toEqual(["/mirror/subscriptions", "/heartbeat"]);
    expect(calls[0].body?.snapshotSeq).toBe(1);
    expect(calls[0].authorization).toBe("Bearer test-secret");
    expect(calls[1].body).toEqual({ stackEpoch: "epoch-test", seq: 1, at: "2026-11-23T12:00:00.000Z" });
    expect(JSON.parse(await readFile(stateFile, "utf8")).snapshotSeq).toBe(1);
    expect(client.lastPendingAnswers).toBe(0);
  });

  test("an unchanged set is not re-posted on the next beat", async () => {
    const { client, calls } = await makeClient(okHandler);
    await client.start();
    await client.beat();
    const snapshots = calls.filter((c) => c.url.endsWith("/mirror/subscriptions"));
    const beats = calls.filter((c) => c.url.endsWith("/heartbeat"));
    expect(snapshots).toHaveLength(1);
    expect(beats).toHaveLength(2);
    expect(beats[1].body?.seq).toBe(2);
  });

  test("a changed set takes the next seq after requestSubscriptionSync", async () => {
    const { client, calls, rows } = await makeClient(okHandler);
    await client.start();
    rows.push(row({ id: "sub-2", endpoint: "https://fcm.googleapis.com/fcm/send/other" }));
    client.requestSubscriptionSync();
    // The kick is async; wait for the second snapshot POST.
    for (let i = 0; i < 50 && calls.filter((c) => c.url.endsWith("/mirror/subscriptions")).length < 2; i++) {
      await new Promise((r) => setTimeout(r, 10));
    }
    const snapshots = calls.filter((c) => c.url.endsWith("/mirror/subscriptions"));
    expect(snapshots).toHaveLength(2);
    expect(snapshots[1].body?.snapshotSeq).toBe(2);
    expect((snapshots[1].body?.subscriptions as unknown[]).length).toBe(2);
  });

  test("a failed snapshot POST retries with the SAME seq on the next beat", async () => {
    let snapshotAttempts = 0;
    const { client, calls } = await makeClient((call) => {
      if (call.url.endsWith("/mirror/subscriptions")) {
        snapshotAttempts += 1;
        return snapshotAttempts === 1 ? { status: 500 } : { status: 200, body: { ok: true } };
      }
      return { status: 200, body: { ok: true, pendingAnswers: 0 } };
    });
    await client.start();
    await client.beat();
    const snapshots = calls.filter((c) => c.url.endsWith("/mirror/subscriptions"));
    expect(snapshots.map((c) => c.body?.snapshotSeq)).toEqual([1, 1]);
    // The heartbeat still went out on the failed beat — the mirror
    // owing a retry must not mute the dead-man's switch input.
    expect(calls.filter((c) => c.url.endsWith("/heartbeat"))).toHaveLength(2);
  });

  test("a 409 re-adopts the edge's seq from /status; the next sync lands above it", async () => {
    let snapshotAttempts = 0;
    const { client, calls, rows, stateFile } = await makeClient((call) => {
      if (call.url.endsWith("/mirror/subscriptions")) {
        snapshotAttempts += 1;
        return snapshotAttempts === 1 ? { status: 409 } : { status: 200, body: { ok: true } };
      }
      if (call.url.endsWith("/status")) return { status: 200, body: { snapshotSeq: 7 } };
      return { status: 200, body: { ok: true, pendingAnswers: 0 } };
    });
    await client.start();
    expect(JSON.parse(await readFile(stateFile, "utf8")).snapshotSeq).toBe(7);
    rows.push(row({ id: "sub-2", endpoint: "https://fcm.googleapis.com/fcm/send/other" }));
    client.requestSubscriptionSync();
    for (let i = 0; i < 50 && snapshotAttempts < 2; i++) await new Promise((r) => setTimeout(r, 10));
    const snapshots = calls.filter((c) => c.url.endsWith("/mirror/subscriptions"));
    expect(snapshots.map((c) => c.body?.snapshotSeq)).toEqual([1, 8]);
  });

  test("a missed event acknowledgment retries the same id until the mirror is complete", async () => {
    let attempts = 0;
    const serverMirror = new Map<string, Record<string, unknown>>();
    const { client, calls, stateFile } = await makeClient((call) => {
      if (call.url.endsWith("/mirror/event")) {
        attempts += 1;
        const event = call.body!;
        if (!serverMirror.has(String(event.mirrorId))) serverMirror.set(String(event.mirrorId), event);
        return attempts === 1 ? { status: 500 } : { status: 200, body: { ok: true, outcome: "duplicate" } };
      }
      return okHandler(call);
    });
    await client.start();
    const sourceEvents = [
      {
        mirrorId: "incident-event-id",
        objectRef: "incident:42",
        transition: "opened",
        raisedAt: "2026-11-23T12:00:00.000Z",
        payload: { class: "incident", title: "Fixture incident" },
      },
      {
        mirrorId: "needs-drew-event-id",
        objectRef: "item:task-7",
        transition: "raised",
        raisedAt: "2026-11-23T12:01:00.000Z",
        payload: { class: "needs-drew", title: "Fixture question" },
      },
      {
        mirrorId: "quota-wall-event-id",
        objectRef: "quota:pool-a",
        transition: "raised",
        raisedAt: "2026-11-23T12:02:00.000Z",
        payload: { class: "quota-wall", stateFingerprint: "walled" },
      },
    ];

    await client.mirrorEvent(sourceEvents[0]!);
    expect(JSON.parse(await readFile(stateFile, "utf8")).pendingEvents).toEqual([sourceEvents[0]]);
    await client.beat();
    await client.beat(); // the next beat retries if the first overlapped the missed acknowledgment
    await client.mirrorEvent(sourceEvents[1]!);
    await client.mirrorEvent(sourceEvents[2]!);
    await client.beat();
    await client.beat();

    const eventCalls = calls.filter((call) => call.url.endsWith("/mirror/event"));
    expect(eventCalls.map((call) => call.body?.mirrorId)).toContain(sourceEvents[0]!.mirrorId);
    expect(eventCalls.filter((call) => call.body?.mirrorId === sourceEvents[0]!.mirrorId)).toHaveLength(2);
    expect([...serverMirror.values()]).toEqual(sourceEvents);
    expect(JSON.parse(await readFile(stateFile, "utf8")).pendingEvents).toEqual([]);
    for (const event of sourceEvents) await client.mirrorEvent(event);
    expect(calls.filter((call) => call.url.endsWith("/mirror/event"))).toHaveLength(eventCalls.length);
  });

  test("a quota wall recurrence after recovery gets a new stable episode id", async () => {
    const mirrored = new Map<string, Record<string, unknown>>();
    const { client, calls } = await makeClient((call) => {
      if (call.url.endsWith("/mirror/event")) {
        const event = call.body!;
        mirrored.set(String(event.mirrorId), event);
      }
      return okHandler(call);
    });
    await client.start();
    const candidate: NotificationCandidate = {
      class: "quota-wall",
      objectRef: "quota:pool-a",
      stateFingerprint: JSON.stringify({ state: "walled", resetAt: "2026-11-25T00:00:00.000Z" }),
      title: "Quota wall",
      body: "Pool is walled",
      url: "/quota",
      observedAt: new Date().toISOString(),
      breakthrough: true,
    };

    await client.mirrorCandidates([candidate]);
    await client.beat();
    await client.mirrorCandidates([candidate]); // retries in the same episode keep one id
    await client.mirrorCandidates([]); // recovery closes the active episode
    await client.mirrorCandidates([candidate]);
    await client.beat();
    await client.beat();

    const ids = calls.filter((call) => call.url.endsWith("/mirror/event")).map((call) => call.body?.mirrorId);
    expect(new Set(ids).size).toBe(2);
    expect(mirrored.size).toBe(2);
  });

  test("a failed durable enqueue rejects before any mirror request can be sent", async () => {
    const { client, calls } = await makeClient(okHandler, { blockStateFile: true });
    await expect(
      client.mirrorEvent({
        mirrorId: "must-persist-first",
        objectRef: "incident:9",
        transition: "opened",
        raisedAt: new Date().toISOString(),
        payload: { class: "incident" },
      }),
    ).rejects.toThrow();
    expect(calls.filter((call) => call.url.endsWith("/mirror/event"))).toHaveLength(0);
  });
});

describe("fail-soft (the relay never breaks the throat)", () => {
  test("a dead edge: beats resolve, the error is recorded, nothing throws", async () => {
    const { client } = await makeClient(() => ({ status: 500 }));
    await client.start();
    await client.beat();
    expect(client.lastError).toContain("500");
  });

  test("no secret resolvable: nothing is posted, nothing throws", async () => {
    const { client, calls } = await makeClient(okHandler, { secret: null });
    await client.start();
    expect(calls).toHaveLength(0);
    expect(client.lastError).toContain("unavailable");
  });

  test("no alerts store: the sync is skipped, the heartbeat still posts", async () => {
    const { client, calls } = await makeClient(okHandler, { storeNull: true });
    await client.start();
    expect(calls.map((c) => c.url)).toEqual(["https://pm-notify.example.workers.dev/heartbeat"]);
  });
});
