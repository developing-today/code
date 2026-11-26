// #15485 (#2046 §6): the per-binding request ceiling. Under the
// ceiling requests pass; at it, further requests are refused
// with a retryable 429 + Retry-After while export and the usage
// read stay available; after the window resets, requests pass
// again and the count restarts. Same harness style as
// usage.test.ts: the real UserDO against in-memory SQLite and
// an in-memory bucket.
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

function fakeBucket() {
  const objects = new Map<string, Uint8Array>();
  return {
    objects,
    async head(key: string) {
      const v = objects.get(key);
      return v ? { size: v.byteLength } : null;
    },
    async get(key: string) {
      const v = objects.get(key);
      if (!v) return null;
      return {
        size: v.byteLength,
        body: new ReadableStream({
          start(c) {
            c.enqueue(v);
            c.close();
          },
        }),
      };
    },
    async put(key: string, value: ArrayBuffer | Uint8Array) {
      objects.set(key, value instanceof Uint8Array ? value : new Uint8Array(value));
    },
    async delete(key: string) {
      objects.delete(key);
    },
  };
}

function makeDO(env: Record<string, unknown> = {}) {
  const bucket = fakeBucket();
  const state = {
    id: { name: A },
    storage: { sql: fakeSql() },
    blockConcurrencyWhile: async <T>(fn: () => Promise<T>): Promise<T> => fn(),
  };
  return { doA: new UserDO(state as never, { BLOBS: bucket, ...env } as never), bucket };
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
  at_request_cap: boolean;
  request_window_resets_at: string | null;
}

async function getUsage(d: UserDO): Promise<Usage> {
  const r = await d.fetch(routed("/v1/usage"));
  assert.equal(r.status, 200, "usage stays available at the request cap");
  return (await r.json()) as Usage;
}

test("under and at the ceiling requests pass; past it they are refused with 429 + Retry-After", async () => {
  const { doA } = makeDO({ USER_REQUEST_CEILING: "3", USER_REQUEST_WINDOW_MS: "60000" });
  // Requests 1 and 2: under the ceiling.
  assert.equal((await doA.fetch(routed("/v1/kv/k1", { method: "PUT", body: "a" }))).status, 200);
  assert.equal((await doA.fetch(routed("/v1/kv/k1"))).status, 200);
  // Request 3: at the ceiling — still admitted, the count reaches the cap.
  assert.equal((await doA.fetch(routed("/v1/kv/k1"))).status, 200);
  // Request 4 (cap+1): refused, retryable, and not counted.
  const refused = await doA.fetch(routed("/v1/kv/k1"));
  assert.equal(refused.status, 429);
  const body = (await refused.json()) as { error: string; retry_after_seconds: number };
  assert.equal(body.error, "request_ceiling_exceeded");
  assert.ok(body.retry_after_seconds >= 1 && body.retry_after_seconds <= 60);
  assert.equal(refused.headers.get("retry-after"), String(body.retry_after_seconds));
  const u = await getUsage(doA);
  assert.equal(u.requests, 3, "the refused request was not counted past the ceiling");
  assert.equal(u.request_ceiling, 3);
  assert.equal(u.at_request_cap, true);
  assert.ok(u.request_window_resets_at !== null);
});

test("export stays available past the request ceiling", async () => {
  const { doA } = makeDO({ USER_REQUEST_CEILING: "1", USER_REQUEST_WINDOW_MS: "60000" });
  assert.equal((await doA.fetch(routed("/v1/kv/k1", { method: "PUT", body: "a" }))).status, 200);
  assert.equal((await doA.fetch(routed("/v1/kv/k1"))).status, 429);
  const exp = await doA.fetch(routed("/v1/export"));
  assert.equal(exp.status, 200, "export is the recovery path and remains available at the cap");
  assert.equal(((await exp.json()) as { kv: Record<string, string> }).kv.k1, "a");
});

test("the count resets and requests pass again after the window elapses", async () => {
  const { doA } = makeDO({ USER_REQUEST_CEILING: "1", USER_REQUEST_WINDOW_MS: "25" });
  assert.equal((await doA.fetch(routed("/v1/kv/k1", { method: "PUT", body: "a" }))).status, 200);
  assert.equal((await doA.fetch(routed("/v1/kv/k1"))).status, 429);
  await new Promise((r) => setTimeout(r, 40));
  assert.equal((await doA.fetch(routed("/v1/kv/k1"))).status, 200, "a fresh window admits requests again");
  const u = await getUsage(doA);
  assert.equal(u.at_request_cap, true, "the fresh window's first request reaches the ceiling of 1 again");
  assert.equal(u.requests, 1, "the window count restarted instead of accumulating across windows");
});

test("no ceiling configured: usage reports a null request ceiling and nothing is refused", async () => {
  const { doA } = makeDO();
  for (let i = 0; i < 5; i++) {
    assert.equal((await doA.fetch(routed("/v1/kv/k1"))).status, 404, "missing key, but never a cap refusal");
  }
  const u = await getUsage(doA);
  assert.equal(u.request_ceiling, null);
  assert.equal(u.at_request_cap, false);
  assert.equal(u.request_window_resets_at, null);
  assert.equal(u.requests, 6, "the monotonic metering count is unchanged with no ceiling configured");
});

test("a malformed request ceiling fails closed", async () => {
  const { doA } = makeDO({ USER_REQUEST_CEILING: "many" });
  const r = await doA.fetch(routed("/v1/kv/k1"));
  assert.equal(r.status, 500);
  assert.equal(((await r.json()) as { error: string }).error, "request_ceiling_misconfigured");
});
test("bridge export stays available past the request ceiling: the challenge still mints a nonce at the cap", async () => {
  const { doA } = makeDO({ USER_REQUEST_CEILING: "2", USER_REQUEST_WINDOW_MS: "60000" });
  const key = await importIdKey(new Uint8Array(32).fill(61));
  const bridgeRouted = (path: string, init: RequestInit = {}): Request => {
    const headers = new Headers(init.headers);
    headers.set(HDR_IROH, key.publicHex);
    return routed(path, { ...init, headers });
  };
  const proofFor = async (nonce: string, context: "push" | "export", body: Uint8Array) => {
    const signature = await key.sign(authTranscript(A, nonce, context, bodyHashHex(body)));
    return { "x-id-nonce": nonce, "x-id-proof": bytesToHex(signature) };
  };
  // Seed one bridge event (counted request 1) and one KV write
  // (counted request 2): the window count reaches the ceiling of 2.
  const event = await signEvent(key, {
    seq: 0,
    backlink: null,
    kind: "message",
    body: new TextEncoder().encode("body 0"),
  });
  const batch = encodeBatch([event.bytes]);
  const firstChallenge = await doA.fetch(bridgeRouted("/v1/bridge/challenge"));
  assert.equal(firstChallenge.status, 200);
  const firstNonce = ((await firstChallenge.json()) as { nonce: string }).nonce;
  const push = await doA.fetch(
    bridgeRouted("/v1/bridge/push", {
      method: "POST",
      headers: await proofFor(firstNonce, "push", batch),
      body: batch as BodyInit,
    }),
  );
  assert.equal(push.status, 200);
  assert.equal((await doA.fetch(routed("/v1/kv/k1", { method: "PUT", body: "a" }))).status, 200);
  // At the cap, ordinary requests are refused...
  assert.equal((await doA.fetch(routed("/v1/kv/k1"))).status, 429);
  // ...but the challenge route is cap-exempt, so a caller holding
  // no live nonce can still mint one. Nonces are single-use with a
  // 5-minute TTL (bridge-auth.ts): without this exemption the
  // cap-exempt bridge export would be unreachable at the cap.
  const challenge = await doA.fetch(bridgeRouted("/v1/bridge/challenge"));
  assert.equal(challenge.status, 200, "the bridge challenge stays available at the request cap");
  const { nonce } = (await challenge.json()) as { nonce: string };
  assert.ok(nonce.length > 0);
  // Bridge export with the proof minted at the cap succeeds, with
  // the export proof checks fully intact.
  const exp = await doA.fetch(
    bridgeRouted("/v1/bridge/export", { headers: await proofFor(nonce, "export", new Uint8Array(0)) }),
  );
  assert.equal(exp.status, 200, "bridge export with a proof minted at the cap succeeds");
  assert.equal(((await exp.json()) as { object_count: number }).object_count, 1);
});
