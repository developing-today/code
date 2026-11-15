// Slice B of #3811 (#9955): metering accuracy, at-cap behavior
// (write rejected, read + export succeed), and recovery via
// delete. Same harness style as blobs.test.ts: the real UserDO
// against in-memory SQLite and an in-memory bucket.
import { test } from "node:test";
import assert from "node:assert/strict";
import { DatabaseSync } from "node:sqlite";
import { UserDO, HDR_BINDING, HDR_JURISDICTION, HDR_ROUTED } from "../src/user-do.ts";

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
  kv_count: number;
  kv_bytes: number;
  blob_count: number;
  blob_bytes: number;
  storage_bytes: number;
  ceiling_bytes: number | null;
  at_cap: boolean;
}

async function getUsage(d: UserDO): Promise<Usage> {
  const r = await d.fetch(routed("/v1/usage"));
  assert.equal(r.status, 200);
  return (await r.json()) as Usage;
}

test("metering is accurate across kv and blob writes", async () => {
  const { doA } = makeDO();
  await doA.fetch(routed("/v1/kv/k1", { method: "PUT", body: "12345" }));
  await doA.fetch(routed("/v1/kv/k2", { method: "PUT", body: "1234567" }));
  const blob = new TextEncoder().encode("blobbytes!");
  await doA.fetch(routed("/v1/blobs", { method: "PUT", body: blob }));
  const u = await getUsage(doA);
  assert.equal(u.kv_count, 2);
  assert.equal(u.kv_bytes, 12);
  assert.equal(u.blob_count, 1);
  assert.equal(u.blob_bytes, 10);
  assert.equal(u.storage_bytes, 22);
  assert.equal(u.ceiling_bytes, null);
  assert.equal(u.at_cap, false);
  // 3 writes + this usage read were all counted as requests.
  assert.equal(u.requests, 4);
  // Overwriting a key re-meters by delta, not by addition.
  await doA.fetch(routed("/v1/kv/k1", { method: "PUT", body: "xy" }));
  const u2 = await getUsage(doA);
  assert.equal(u2.kv_bytes, 9);
  assert.equal(u2.storage_bytes, 19);
});

test("at cap: writes fail closed, reads and export succeed, delete recovers headroom", async () => {
  const { doA } = makeDO({ USER_CEILING_BYTES: "10" });
  const put1 = await doA.fetch(routed("/v1/kv/k1", { method: "PUT", body: "123456" }));
  assert.equal(put1.status, 200);
  const put2 = await doA.fetch(routed("/v1/kv/k2", { method: "PUT", body: "12345" }));
  assert.equal(put2.status, 413);
  assert.equal(((await put2.json()) as { error: string }).error, "ceiling_exceeded");

  const blobPut = await doA.fetch(routed("/v1/blobs", { method: "PUT", body: new TextEncoder().encode("12345") }));
  assert.equal(blobPut.status, 413, "blob writes are ceilinged too");

  const read = await doA.fetch(routed("/v1/kv/k1"));
  assert.equal(read.status, 200, "reads continue at cap");
  assert.equal(((await read.json()) as { value: string }).value, "123456");

  const exp = await doA.fetch(routed("/v1/export"));
  assert.equal(exp.status, 200, "export remains available at cap");
  const expBody = (await exp.json()) as { kv: Record<string, string>; usage: Usage };
  assert.equal(expBody.kv.k1, "123456");
  assert.equal(expBody.usage.storage_bytes, 6);

  const u = await getUsage(doA);
  assert.equal(u.at_cap, false, "6 of 10 bytes used is not at cap");
  const del = await doA.fetch(routed("/v1/kv/k1", { method: "DELETE" }));
  assert.equal(del.status, 200, "deletes stay available at cap");
  const put3 = await doA.fetch(routed("/v1/kv/k2", { method: "PUT", body: "12345" }));
  assert.equal(put3.status, 200, "headroom recovered after delete");
});

test("a deduped re-PUT of the same blob does not double-charge", async () => {
  const { doA } = makeDO({ USER_CEILING_BYTES: "10" });
  const blob = new TextEncoder().encode("0123456789");
  assert.equal((await doA.fetch(routed("/v1/blobs", { method: "PUT", body: blob }))).status, 200);
  assert.equal((await doA.fetch(routed("/v1/blobs", { method: "PUT", body: blob }))).status, 200);
  const u = await getUsage(doA);
  assert.equal(u.blob_count, 1);
  assert.equal(u.blob_bytes, 10);
});

test("export carries kv, blob references, and usage", async () => {
  const { doA } = makeDO();
  await doA.fetch(routed("/v1/kv/note", { method: "PUT", body: "hi" }));
  await doA.fetch(routed("/v1/blobs", { method: "PUT", body: new TextEncoder().encode("data") }));
  const exp = await doA.fetch(routed("/v1/export"));
  const body = (await exp.json()) as {
    binding_id: string;
    kv: Record<string, string>;
    blobs: { hash: string; size: number }[];
    usage: Usage;
  };
  assert.equal(body.binding_id, A);
  assert.deepEqual(body.kv, { note: "hi" });
  assert.equal(body.blobs.length, 1);
  assert.equal(body.blobs[0].size, 4);
  assert.match(body.blobs[0].hash, /^[0-9a-f]{64}$/);
  assert.equal(body.usage.storage_bytes, 6);
});
