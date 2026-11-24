// Oracles for Phase-2 step-2: the authenticated event endpoint appends
// an idempotent 30-day mirror row and never sends a mirrored event.
import { describe, expect, test } from "bun:test";
import worker, { PmNotify, type Env } from "../src/index.ts";

function fixture() {
  const values = new Map<string, unknown>();
  const durableState = {
    storage: {
      get: async <T>(key: string) => values.get(key) as T | undefined,
      put: async (key: string, value: unknown) => void values.set(key, value),
      setAlarm: async () => {},
    },
  };
  const relay = new PmNotify(durableState as never, {});
  const stub = { fetch: (request: Request) => relay.fetch(request) };
  const env = {
    RELAY_SHARED_SECRET: "test-secret",
    PM_NOTIFY: { idFromName: () => "singleton", get: () => stub },
  } as unknown as Env;
  return { env, values, relay };
}

const event = (over: Partial<ReturnType<typeof makeEvent>> = {}) => ({ ...makeEvent(), ...over });
function makeEvent() {
  return {
    mirrorId: "mirror-1",
    objectRef: "incident:42",
    transition: "opened",
    raisedAt: new Date().toISOString(),
    payload: { class: "incident", title: "Fixture incident" },
  };
}

describe("POST /mirror/event", () => {
  test("requires shared-secret auth before appending to the DO", async () => {
    const { env, values } = fixture();
    const payload = event();
    const response = await worker.fetch(
      new Request("https://pm-notify.example.workers.dev/mirror/event", {
        method: "POST",
        headers: { "content-type": "application/json" },
        body: JSON.stringify(payload),
      }),
      env,
    );
    expect(response.status).toBe(401);
    expect(values.size).toBe(0);
  });

  test("acks duplicate retries in the DO log without delivering a push", async () => {
    const { env, values } = fixture();
    const payload = event();
    const request = () =>
      worker.fetch(
        new Request("https://pm-notify.example.workers.dev/mirror/event", {
          method: "POST",
          headers: { "content-type": "application/json", authorization: "Bearer test-secret" },
          body: JSON.stringify(payload),
        }),
        env,
      );
    const first = await request();
    const retry = await request();
    const firstBody = await first.json();
    const retryBody = await retry.json();
    const state = values.get("relay-state") as { mirrorEvents: unknown[]; lastDelivery: unknown };

    expect(first.status).toBe(200);
    expect(firstBody).toMatchObject({ ok: true, outcome: "stored", mirrorId: payload.mirrorId });
    expect(retryBody).toMatchObject({ ok: true, outcome: "duplicate", mirrorId: payload.mirrorId });
    expect(state.mirrorEvents).toEqual([payload]);
    expect(state.lastDelivery).toBeNull();
  });

  test("concurrent append and the source ledger reconcile to the DO mirror log", async () => {
    const { env, values } = fixture();
    const sourceLedger = [
      event({ mirrorId: "incident-1", objectRef: "incident:42", payload: { class: "incident" } }),
      event({ mirrorId: "needs-drew-1", objectRef: "item:task-7", transition: "raised", payload: { class: "needs-drew" } }),
      event({ mirrorId: "quota-wall-1", objectRef: "quota:pool-a", transition: "raised", payload: { class: "quota-wall" } }),
    ];
    const append = (payload: ReturnType<typeof event>) =>
      worker.fetch(
        new Request("https://pm-notify.example.workers.dev/mirror/event", {
          method: "POST",
          headers: { "content-type": "application/json", authorization: "Bearer test-secret" },
          body: JSON.stringify(payload),
        }),
        env,
      );

    await Promise.all([...sourceLedger.map(append), append(sourceLedger[0]!)]);
    const state = values.get("relay-state") as { mirrorEvents: Array<{ mirrorId: string }>; lastDelivery: unknown };

    expect(state.mirrorEvents.map((row) => row.mirrorId)).toEqual(sourceLedger.map((row) => row.mirrorId));
    expect(new Set(state.mirrorEvents.map((row) => row.mirrorId)).size).toBe(sourceLedger.length);
    expect(state.lastDelivery).toBeNull();
  });

  test("event, heartbeat, and alarm updates serialize without dropping mirror rows", async () => {
    const { env, values, relay } = fixture();
    const payload = event({ mirrorId: "during-beat", objectRef: "incident:77" });
    const append = worker.fetch(
      new Request("https://pm-notify.example.workers.dev/mirror/event", {
        method: "POST",
        headers: { "content-type": "application/json", authorization: "Bearer test-secret" },
        body: JSON.stringify(payload),
      }),
      env,
    );
    const heartbeat = worker.fetch(
      new Request("https://pm-notify.example.workers.dev/heartbeat", {
        method: "POST",
        headers: { "content-type": "application/json", authorization: "Bearer test-secret" },
        body: JSON.stringify({ stackEpoch: "epoch", seq: 1 }),
      }),
      env,
    );
    await Promise.all([append, heartbeat, relay.alarm()]);
    const state = values.get("relay-state") as {
      mirrorEvents: Array<{ mirrorId: string }>;
      heartbeat: { lastBeatAt: number | null };
    };

    expect(state.mirrorEvents.map((row) => row.mirrorId)).toContain(payload.mirrorId);
    expect(state.heartbeat.lastBeatAt).not.toBeNull();
  });
});
