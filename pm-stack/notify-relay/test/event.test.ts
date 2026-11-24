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
    },
    setAlarm: async () => {},
  };
  const relay = new PmNotify(durableState as never, {});
  const stub = { fetch: (request: Request) => relay.fetch(request) };
  const env = {
    RELAY_SHARED_SECRET: "test-secret",
    PM_NOTIFY: { idFromName: () => "singleton", get: () => stub },
  } as unknown as Env;
  return { env, values };
}

const event = {
  mirrorId: "mirror-1",
  objectRef: "incident:42",
  transition: "opened",
  raisedAt: "2026-11-24T12:00:00.000Z",
  payload: { class: "incident", title: "Fixture incident" },
};

describe("POST /mirror/event", () => {
  test("requires shared-secret auth before appending to the DO", async () => {
    const { env, values } = fixture();
    const response = await worker.fetch(
      new Request("https://pm-notify.example.workers.dev/mirror/event", {
        method: "POST",
        headers: { "content-type": "application/json" },
        body: JSON.stringify(event),
      }),
      env,
    );
    expect(response.status).toBe(401);
    expect(values.size).toBe(0);
  });

  test("acks duplicate retries in the DO log without delivering a push", async () => {
    const { env, values } = fixture();
    const request = () =>
      worker.fetch(
        new Request("https://pm-notify.example.workers.dev/mirror/event", {
          method: "POST",
          headers: { "content-type": "application/json", authorization: "Bearer test-secret" },
          body: JSON.stringify(event),
        }),
        env,
      );
    const first = await request();
    const retry = await request();
    const firstBody = await first.json();
    const retryBody = await retry.json();
    const state = values.get("relay-state") as { mirrorEvents: unknown[]; lastDelivery: unknown };

    expect(first.status).toBe(200);
    expect(firstBody).toMatchObject({ ok: true, outcome: "stored", mirrorId: event.mirrorId });
    expect(retryBody).toMatchObject({ ok: true, outcome: "duplicate", mirrorId: event.mirrorId });
    expect(state.mirrorEvents).toEqual([event]);
    expect(state.lastDelivery).toBeNull();
  });
});
