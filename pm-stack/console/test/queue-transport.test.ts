// The console's own view of the queue transport's health (#13503 slice 1).
//
// Oracles: the alert Worker's publicStatus shape
// (pkgs/queue-transport-alert/worker/src/status.js) answered by a
// stand-in fetcher; the notification decision is compared with what
// that shape says. The store is the real Edge over a map: what the
// stateless part hands it is what its health-facing read gives back,
// and a body that is not a verdict is refused.
import { describe, expect, test } from "bun:test";
import { Edge } from "../edge/src/edge.ts";
import type { KV } from "../edge/src/edge-shared.ts";
import {
  fetchQueueTransport,
  parseQueueTransport,
  QUEUE_TRANSPORT_KEY,
  transportMessage,
  transportNeedsAttention,
  transportNotifyDecision,
  transportToldMarker,
  type QueueTransport,
} from "../edge/src/queue-transport.ts";

const AT = 1_800_000_000_000;

function answering(body: unknown, status = 200): (request: Request) => Promise<Response> {
  return (async () =>
    new Response(JSON.stringify(body), {
      status,
      headers: { "content-type": "application/json" },
    })) as unknown as (request: Request) => Promise<Response>;
}

/** An alert-Worker /status body, as publicStatus writes it. */
function statusBody(overrides: Record<string, unknown> = {}): Record<string, unknown> {
  return {
    service: "queue-broker",
    status: "up",
    up: true,
    last_check: "2026-11-27T05:00:00Z",
    last_ok: "2026-11-27T05:00:00Z",
    failing_since: null,
    consecutive_failures: 0,
    last_failure: null,
    incident: null,
    last_outage: null,
    last_push: null,
    pending_deliveries: [],
    last_errors: [],
    probe: "https://broker.example/healthz via Cloudflare Access",
    cadence_minutes: 10,
    alert_after_failures: 2,
    issue_title_prefix: "Queue transport incident:",
    source: "pkgs/queue-transport-alert/worker (code#6369)",
    ...overrides,
  };
}

const downBody = () =>
  statusBody({
    status: "down",
    up: false,
    failing_since: "2026-11-27T04:40:00Z",
    consecutive_failures: 3,
    last_failure: "HTTP 530: the tunnel could not reach the broker",
    last_ok: "2026-11-27T04:30:00Z",
    incident: { number: 7001, url: "https://github.com/developing-today/code/issues/7001" },
  });

describe("fetchQueueTransport", () => {
  test("an up verdict normalizes to the console's shape", async () => {
    const verdict = await fetchQueueTransport(answering(statusBody()), AT);
    expect(verdict).toEqual({
      status: "up",
      consecutiveFailures: 0,
      failingSince: null,
      lastFailure: null,
      lastCheck: "2026-11-27T05:00:00Z",
      lastOk: "2026-11-27T05:00:00Z",
      incident: null,
      checkedAt: AT,
    });
  });

  test("a down verdict keeps the incident and the failure line", async () => {
    const verdict = await fetchQueueTransport(answering(downBody()), AT);
    expect(verdict).toMatchObject({
      status: "down",
      consecutiveFailures: 3,
      failingSince: "2026-11-27T04:40:00Z",
      incident: { number: 7001, url: "https://github.com/developing-today/code/issues/7001" },
    });
  });

  test("a binding that reaches nothing is null, not a verdict", async () => {
    const fetcher = (async () => {
      throw new Error("no such worker");
    }) as unknown as (request: Request) => Promise<Response>;
    expect(await fetchQueueTransport(fetcher, AT)).toBeNull();
  });

  test("a non-JSON answer is null, honestly", async () => {
    const fetcher = (async () => new Response("hello")) as unknown as (request: Request) => Promise<Response>;
    expect(await fetchQueueTransport(fetcher, AT)).toBeNull();
  });
});

describe("parseQueueTransport", () => {
  test("a kept verdict reads back whole", () => {
    const verdict: QueueTransport = {
      status: "down",
      consecutiveFailures: 2,
      failingSince: "2026-11-27T04:40:00Z",
      lastFailure: "HTTP 530",
      lastCheck: "2026-11-27T05:00:00Z",
      lastOk: "2026-11-27T04:30:00Z",
      incident: { number: 7001, url: "https://github.com/developing-today/code/issues/7001" },
      checkedAt: AT,
    };
    expect(parseQueueTransport(JSON.parse(JSON.stringify(verdict)))).toEqual(verdict);
  });

  test("anything else is not a verdict", () => {
    expect(parseQueueTransport(null)).toBeNull();
    expect(parseQueueTransport({})).toBeNull();
    expect(parseQueueTransport({ status: "down", consecutiveFailures: 2 })).toBeNull();
    expect(parseQueueTransport({ status: "down", consecutiveFailures: 2, checkedAt: "now" })).toBeNull();
  });
});

describe("transportNotifyDecision", () => {
  const down: QueueTransport = {
    status: "down",
    consecutiveFailures: 3,
    failingSince: "2026-11-27T04:40:00Z",
    lastFailure: "HTTP 530",
    lastCheck: "2026-11-27T05:00:00Z",
    lastOk: "2026-11-27T04:30:00Z",
    incident: { number: 7001, url: "https://github.com/developing-today/code/issues/7001" },
    checkedAt: AT,
  };
  const up: QueueTransport = { ...down, status: "up", consecutiveFailures: 0, failingSince: null, incident: null };

  test("down and untold notifies; the same outage once told stays quiet", () => {
    expect(transportNeedsAttention(down)).toBe(true);
    expect(transportNotifyDecision(down, null)).toBe("notify");
    expect(transportNotifyDecision(down, transportToldMarker(down))).toBe("quiet");
  });

  test("degraded and stale are attention-worthy; up is not", () => {
    expect(transportNeedsAttention({ ...down, status: "degraded" })).toBe(true);
    expect(transportNeedsAttention({ ...down, status: "stale" })).toBe(true);
    expect(transportNeedsAttention(up)).toBe(false);
    expect(transportNeedsAttention(null)).toBe(false);
  });

  test("well again after told is a recovery; never told and well is quiet", () => {
    expect(transportNotifyDecision(up, transportToldMarker(down))).toBe("recovered");
    expect(transportNotifyDecision(up, null)).toBe("quiet");
  });

  test("the down message names the outage and the incident", () => {
    const message = transportMessage(down);
    expect(message.title).toBe("Queue broker is down");
    expect(message.tag).toBe("queue-transport");
    expect(message.body).toContain("3 failed checks");
    expect(message.body).toContain("#7001");
  });
});

describe("the Edge keeps a transport verdict", () => {
  function makeEdge() {
    const store = new Map<string, unknown>();
    const kv: KV = {
      get: async (key: string) => store.get(key),
      put: async (key: string, value: unknown) => {
        store.set(key, value);
      },
      delete: async (key: string) => {
        store.delete(key);
      },
    } as KV;
    return { edge: new Edge({ kv }), store };
  }

  const verdict: QueueTransport = {
    status: "down",
    consecutiveFailures: 2,
    failingSince: "2026-11-27T04:40:00Z",
    lastFailure: "HTTP 530",
    lastCheck: "2026-11-27T05:00:00Z",
    lastOk: "2026-11-27T04:30:00Z",
    incident: null,
    checkedAt: AT,
  };

  test("what the stateless part hands over is what is kept", async () => {
    const { edge, store } = makeEdge();
    const res = await edge.fetch(
      new Request("https://pm.internal/internal/queue-transport", {
        method: "POST",
        headers: { "content-type": "application/json" },
        body: JSON.stringify(verdict),
      }),
    );
    expect(res.status).toBe(200);
    expect(await edge.lastQueueTransport()).toEqual(verdict);
    expect(store.get(QUEUE_TRANSPORT_KEY)).toEqual(verdict);
  });

  test("a body that is not a verdict is refused and nothing is kept", async () => {
    const { edge } = makeEdge();
    const res = await edge.fetch(
      new Request("https://pm.internal/internal/queue-transport", {
        method: "POST",
        headers: { "content-type": "application/json" },
        body: JSON.stringify({ status: "down" }),
      }),
    );
    expect(res.status).toBe(400);
    expect(await edge.lastQueueTransport()).toBeNull();
  });
});
