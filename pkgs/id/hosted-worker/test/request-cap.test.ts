// #15485 (#2046 §6): the per-binding request cap. Requests past the
// cap in a window are refused with a retryable 429 and Retry-After;
// the usage read and the export reads stay available and uncounted;
// usage reports the window; the window resets; a malformed cap fails
// closed. Same harness as usage.test.ts.
import { test } from "node:test";
import assert from "node:assert/strict";
import { DatabaseSync } from "node:sqlite";
import { UserDO, HDR_BINDING, HDR_IROH, HDR_JURISDICTION, HDR_ROUTED } from "../src/user-do.ts";
import { authTranscript, bodyHashHex } from "../src/bridge-auth.ts";
import { bytesToHex, importIdKey } from "../src/ed25519.ts";
import { signEvent } from "../src/event.ts";
import { encodeBatch } from "../src/bridge-session.ts";

const A = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";

function fakeSql() {
  const db = new DatabaseSync(":memory:");
  return {
    exec(query: string, ...params: unknown[]) {
      const stmt = db.prepare(query);
      if (/^\s*select/i.test(query)) {
        const rows = stmt.all(...(params as never[])) as Record<string, unknown>[];
        return { toArray: () => rows };
      }
      stmt.run(...(params as never[]));
      return { toArray: () => [] as Record<string, unknown>[] };
    },
  };
}

function makeDO(env: Record<string, unknown> = {}) {
  const state = {
    id: { name: A },
    storage: { sql: fakeSql() },
    blockConcurrencyWhile: async <T>(fn: () => Promise<T>): Promise<T> => fn(),
  };
  return new UserDO(state as never, env as never);
}

function routed(path: string, init: RequestInit = {}): Request {
  const headers = new Headers(init.headers);
  headers.set(HDR_BINDING, A);
  headers.set(HDR_JURISDICTION, "us");
  headers.set(HDR_ROUTED, "restricted");
  return new Request("https://do" + path, { ...init, headers });
}

interface Usage {
  requests: number;
  request_ceiling: number | null;
  request_window_seconds: number | null;
  window_requests: number | null;
  window_resets_at_ms: number | null;
  at_request_cap: boolean;
}

async function usage(d: UserDO): Promise<Usage> {
  const r = await d.fetch(routed("/v1/usage"));
  assert.equal(r.status, 200);
  return (await r.json()) as Usage;
}

/** Run `fn` with Date.now pinned to the value `clock.now` holds. */
async function withClock<T>(clock: { now: number }, fn: () => Promise<T>): Promise<T> {
  const real = Date.now;
  Date.now = () => clock.now;
  try {
    return await fn();
  } finally {
    Date.now = real;
  }
}

test("no cap configured: nothing is refused and usage reports no cap", async () => {
  const d = makeDO();
  for (let i = 0; i < 5; i++) {
    assert.equal((await d.fetch(routed("/v1/state"))).status, 200);
  }
  const u = await usage(d);
  assert.equal(u.request_ceiling, null);
  assert.equal(u.window_requests, null);
  assert.equal(u.at_request_cap, false);
});

test("under, at and over the cap: the request past the cap is a retryable 429", async () => {
  const clock = { now: 1_000_000 };
  await withClock(clock, async () => {
    const d = makeDO({ USER_REQUEST_CEILING: "3", USER_REQUEST_WINDOW_SECONDS: "60" });
    for (let i = 0; i < 3; i++) {
      assert.equal((await d.fetch(routed("/v1/state"))).status, 200, `request ${i + 1}`);
    }
    const atCap = await usage(d);
    assert.equal(atCap.request_ceiling, 3);
    assert.equal(atCap.request_window_seconds, 60);
    assert.equal(atCap.window_requests, 3);
    assert.equal(atCap.at_request_cap, true);
    assert.equal(atCap.window_resets_at_ms, 1_000_000 + 60_000);

    clock.now += 20_000;
    const refused = await d.fetch(routed("/v1/kv/k", { method: "PUT", body: "v" }));
    assert.equal(refused.status, 429);
    assert.equal(refused.headers.get("retry-after"), "40");
    const body = (await refused.json()) as { error: string; retry_after_seconds: number };
    assert.equal(body.error, "request_cap_exceeded");
    assert.equal(body.retry_after_seconds, 40);
    // The refused write did not happen and did not use up the window.
    assert.equal((await usage(d)).window_requests, 3);
  });
});

test("the window resets and admits requests again", async () => {
  const clock = { now: 5_000_000 };
  await withClock(clock, async () => {
    const d = makeDO({ USER_REQUEST_CEILING: "1", USER_REQUEST_WINDOW_SECONDS: "10" });
    assert.equal((await d.fetch(routed("/v1/state"))).status, 200);
    assert.equal((await d.fetch(routed("/v1/state"))).status, 429);
    clock.now += 10_000;
    const fresh = await usage(d);
    assert.equal(fresh.window_requests, 0);
    assert.equal(fresh.at_request_cap, false);
    assert.equal((await d.fetch(routed("/v1/state"))).status, 200);
    assert.equal((await d.fetch(routed("/v1/state"))).status, 429);
  });
});

test("a clock that moves backwards starts a fresh window rather than locking the caller out", async () => {
  const clock = { now: 9_000_000 };
  await withClock(clock, async () => {
    const d = makeDO({ USER_REQUEST_CEILING: "1" });
    assert.equal((await d.fetch(routed("/v1/state"))).status, 200);
    clock.now -= 1_000;
    assert.equal((await d.fetch(routed("/v1/state"))).status, 200);
  });
});

test("the default window is one day", async () => {
  const d = makeDO({ USER_REQUEST_CEILING: "10" });
  assert.equal((await usage(d)).request_window_seconds, 86_400);
});

test("a malformed cap fails every request closed", async () => {
  for (const env of [
    { USER_REQUEST_CEILING: "ten" },
    { USER_REQUEST_CEILING: "-1" },
    { USER_REQUEST_CEILING: "1.5" },
    { USER_REQUEST_CEILING: "10", USER_REQUEST_WINDOW_SECONDS: "0" },
    { USER_REQUEST_CEILING: "10", USER_REQUEST_WINDOW_SECONDS: "x" },
  ]) {
    const d = makeDO(env);
    for (const path of ["/v1/state", "/v1/usage"]) {
      const r = await d.fetch(routed(path));
      assert.equal(r.status, 500, JSON.stringify(env));
      assert.equal(((await r.json()) as { error: string }).error, "request_cap_misconfigured");
    }
  }
});

test("a zero cap refuses everything except the usage read", async () => {
  const d = makeDO({ USER_REQUEST_CEILING: "0" });
  assert.equal((await d.fetch(routed("/v1/state"))).status, 429);
  assert.equal((await d.fetch(routed("/v1/export"))).status, 429);
  assert.equal((await usage(d)).at_request_cap, true);
});

// #15591: the export paths stay reachable at the cap. The bridge
// challenge is exempt too, because bridge export needs a fresh
// single-use nonce that only the challenge mints.

function emptyBucket() {
  return {
    async head() {
      return null;
    },
    async get() {
      return null;
    },
    async put() {},
    async delete() {},
  };
}

test("plain export stays available at the request cap and is not counted", async () => {
  const clock = { now: 9_000_000 };
  await withClock(clock, async () => {
    const d = makeDO({ USER_REQUEST_CEILING: "2", USER_REQUEST_WINDOW_SECONDS: "60" });
    assert.equal((await d.fetch(routed("/v1/kv/k1", { method: "PUT", body: "a" }))).status, 200);
    assert.equal((await d.fetch(routed("/v1/kv/k2", { method: "PUT", body: "b" }))).status, 200);
    assert.equal((await d.fetch(routed("/v1/state"))).status, 429);
    const exp = await d.fetch(routed("/v1/export"));
    assert.equal(exp.status, 200, "plain export stays available at the request cap");
    const body = (await exp.json()) as { kv: Record<string, string> };
    assert.deepEqual(body.kv, { k1: "a", k2: "b" });
    assert.equal((await usage(d)).window_requests, 2);
  });
});

test("bridge export stays available at the request cap from a fresh challenge", async () => {
  const clock = { now: 12_000_000 };
  await withClock(clock, async () => {
    const d = makeDO({ USER_REQUEST_CEILING: "2", USER_REQUEST_WINDOW_SECONDS: "60", BLOBS: emptyBucket() });
    const key = await importIdKey(new Uint8Array(32).fill(61));
    const bridged = (path: string, init: RequestInit = {}): Request => {
      const headers = new Headers(init.headers);
      headers.set(HDR_IROH, key.publicHex);
      return routed(path, { ...init, headers });
    };
    const proof = async (nonce: string, context: "push" | "export", body: Uint8Array) => {
      const signature = await key.sign(authTranscript(A, nonce, context, bodyHashHex(body)));
      return { "x-id-nonce": nonce, "x-id-proof": bytesToHex(signature) };
    };
    const freshNonce = async (): Promise<string> => {
      const r = await d.fetch(bridged("/v1/bridge/challenge"));
      assert.equal(r.status, 200, "the bridge challenge stays available at the request cap");
      return ((await r.json()) as { nonce: string }).nonce;
    };

    // One pushed event (counted request 1) and one KV write
    // (counted request 2) fill the window; the challenge is not
    // counted.
    const event = await signEvent(key, { seq: 0, backlink: null, kind: "message", extra: [] });
    const batch = encodeBatch([event.bytes]);
    const pushNonce = await freshNonce();
    const push = await d.fetch(
      bridged("/v1/bridge/push", {
        method: "POST",
        headers: await proof(pushNonce, "push", batch),
        body: batch as BodyInit,
      }),
    );
    assert.equal(push.status, 200);
    assert.equal((await d.fetch(routed("/v1/kv/k1", { method: "PUT", body: "a" }))).status, 200);
    assert.equal((await d.fetch(routed("/v1/kv/k1"))).status, 429, "ordinary requests are refused at the cap");

    const nonce = await freshNonce();
    const exportOnce = async () =>
      d.fetch(bridged("/v1/bridge/export", { headers: await proof(nonce, "export", new Uint8Array(0)) }));
    const exp = await exportOnce();
    assert.equal(exp.status, 200, "bridge export with a proof minted at the cap succeeds");
    assert.equal(((await exp.json()) as { object_count: number }).object_count, 1);

    // The proof checks are unchanged: a reused nonce is still refused.
    const replay = await exportOnce();
    assert.notEqual(replay.status, 200);
    assert.equal((await usage(d)).window_requests, 2);
  });
});
