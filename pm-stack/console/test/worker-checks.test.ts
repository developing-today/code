// The edge's checks of the platform's Workers (#15793).
//
// Oracles: a stand-in fetcher answers as a Worker would — a version
// route naming its commit, a health answer naming its version, a 404
// where no version route exists, a throw where the binding reaches
// nothing — and the check is compared with what that answer says.
// The store is the real Edge over a map: what the stateless part
// hands it is what its health-facing read gives back, and a body
// that is not a set is refused.
import { describe, expect, test } from "bun:test";
import { Edge, type KV } from "../edge/src/edge.ts";
import {
  parseWorkerCheckSet,
  probeWorker,
  probeWorkers,
  selfCheck,
  WORKER_CHECKS_KEY,
  type WorkerCheckSet,
} from "../edge/src/worker-checks.ts";

const AT = 1_800_000_000_000;

function answering(body: unknown, status = 200): typeof fetch {
  return (async () =>
    new Response(JSON.stringify(body), {
      status,
      headers: { "content-type": "application/json" },
    })) as typeof fetch;
}

function byPath(routes: Record<string, { body?: unknown; status?: number; text?: string }>): typeof fetch {
  return (async (input: RequestInfo | URL) => {
    const path = new URL(typeof input === "string" ? input : input instanceof URL ? input : input.url).pathname;
    const route = routes[path];
    if (!route) return new Response("not found", { status: 404 });
    if (route.text !== undefined) return new Response(route.text, { status: route.status ?? 200 });
    return new Response(JSON.stringify(route.body ?? {}), {
      status: route.status ?? 200,
      headers: { "content-type": "application/json" },
    });
  }) as typeof fetch;
}

describe("probeWorker", () => {
  test("a /version answer naming its commit is the check", async () => {
    const check = await probeWorker(
      { name: "pm-service", fetcher: answering({ commit: "abc123" }) },
      AT,
    );
    expect(check).toEqual({
      name: "pm-service",
      answering: true,
      commit: "abc123",
      askedAt: AT,
      via: "/version",
      status: 200,
    });
  });

  test("a version field is the commit a VERSION-stamped Worker names", async () => {
    const check = await probeWorker(
      { name: "decision-events", fetcher: answering({ ok: true, version: "def456" }) },
      AT,
    );
    expect(check.commit).toBe("def456");
    expect(check.via).toBe("/version");
  });

  test("no /version route: the health answer is asked instead", async () => {
    const fetcher = byPath({ "/health": { body: { ok: true, version: "feed42" } } });
    const check = await probeWorker({ name: "message-bridge", fetcher }, AT);
    expect(check).toMatchObject({ answering: true, commit: "feed42", via: "/health", status: 200 });
  });

  test("a binding that reaches nothing is not answering", async () => {
    const fetcher = (async () => {
      throw new Error("no such worker");
    }) as typeof fetch;
    const check = await probeWorker({ name: "gone", fetcher }, AT);
    expect(check).toEqual({
      name: "gone",
      answering: false,
      commit: null,
      askedAt: AT,
      via: null,
      status: null,
    });
  });

  test("an answer that names no commit records none, honestly", async () => {
    const fetcher = byPath({ "/version": { text: "hello" } });
    const check = await probeWorker({ name: "plain", fetcher }, AT);
    expect(check).toMatchObject({ answering: true, commit: null, via: "/version", status: 200 });
  });
});

describe("probeWorkers and selfCheck", () => {
  test("the set is sorted by name and stamped once", async () => {
    const set = await probeWorkers(
      [
        { name: "zeta", fetcher: answering({ commit: "z1" }) },
        { name: "alpha", fetcher: answering({ commit: "a1" }) },
      ],
      AT,
    );
    expect(set.askedAt).toBe(AT);
    expect(set.checks.map((c) => c.name)).toEqual(["alpha", "zeta"]);
    expect(set.checks.every((c) => c.askedAt === AT)).toBe(true);
  });

  test("the edge's own record comes from its VERSION var", () => {
    expect(selfCheck("pm-console-edge", "cafe01", AT)).toMatchObject({
      answering: true,
      commit: "cafe01",
      via: "self",
    });
    expect(selfCheck("pm-console-edge", undefined, AT).commit).toBeNull();
  });
});

describe("parseWorkerCheckSet", () => {
  const set: WorkerCheckSet = {
    askedAt: AT,
    checks: [
      { name: "pm-service", answering: true, commit: "abc123", askedAt: AT, via: "/version", status: 200 },
      { name: "gone", answering: false, commit: null, askedAt: AT, via: null, status: null },
    ],
  };

  test("a written set reads back whole", () => {
    expect(parseWorkerCheckSet(JSON.parse(JSON.stringify(set)))).toEqual(set);
  });

  test("anything else is not a set", () => {
    expect(parseWorkerCheckSet(null)).toBeNull();
    expect(parseWorkerCheckSet({})).toBeNull();
    expect(parseWorkerCheckSet({ askedAt: AT, checks: [{ name: 1 }] })).toBeNull();
    expect(parseWorkerCheckSet({ askedAt: "now", checks: [] })).toBeNull();
  });
});

describe("the Edge keeps a round", () => {
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

  const set: WorkerCheckSet = {
    askedAt: AT,
    checks: [
      { name: "pm-service", answering: true, commit: "abc123", askedAt: AT, via: "/health", status: 200 },
    ],
  };

  test("what the stateless part hands over is what is kept", async () => {
    const { edge, store } = makeEdge();
    const res = await edge.fetch(
      new Request("https://pm.internal/internal/worker-checks", {
        method: "POST",
        headers: { "content-type": "application/json" },
        body: JSON.stringify(set),
      }),
    );
    expect(res.status).toBe(200);
    expect(await edge.lastWorkerChecks()).toEqual(set);
    expect(store.get(WORKER_CHECKS_KEY)).toEqual(set);
  });

  test("a body that is not a set is refused and nothing is kept", async () => {
    const { edge } = makeEdge();
    const res = await edge.fetch(
      new Request("https://pm.internal/internal/worker-checks", {
        method: "POST",
        headers: { "content-type": "application/json" },
        body: JSON.stringify({ askedAt: AT, checks: [{ name: 7 }] }),
      }),
    );
    expect(res.status).toBe(400);
    expect(await edge.lastWorkerChecks()).toBeNull();
  });
});
