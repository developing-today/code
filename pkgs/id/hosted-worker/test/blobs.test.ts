// Slice A of #3811 (#9954): blob round-trip via reference
// indirection and user isolation, proven against the real UserDO
// class with an in-memory SQLite (node:sqlite) behind its SQL
// interface and an in-memory bucket behind the R2 interface. The
// workerd-level proof (real R2 emulation) lives in smoke/smoke.mjs.
import { test } from "node:test";
import assert from "node:assert/strict";
import { DatabaseSync } from "node:sqlite";
import { UserDO, HDR_BINDING, HDR_JURISDICTION, HDR_ROUTED } from "../src/user-do.ts";

const A = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const B = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";

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

function makeDO(name: string, bucket: ReturnType<typeof fakeBucket>) {
  const state = {
    id: { name },
    storage: { sql: fakeSql() },
    blockConcurrencyWhile: async <T>(fn: () => Promise<T>): Promise<T> => fn(),
  };
  return new UserDO(state as never, { BLOBS: bucket } as never);
}

function routed(path: string, binding: string, init: RequestInit = {}): Request {
  const headers = new Headers(init.headers);
  headers.set(HDR_BINDING, binding);
  headers.set(HDR_JURISDICTION, "us");
  headers.set(HDR_ROUTED, "restricted");
  return new Request("https://do" + path, { ...init, headers });
}

async function sha256Hex(bytes: Uint8Array): Promise<string> {
  const d = await crypto.subtle.digest("SHA-256", bytes as BufferSource);
  return [...new Uint8Array(d)].map((b) => b.toString(16).padStart(2, "0")).join("");
}

test("blob round-trip via reference indirection", async () => {
  const bucket = fakeBucket();
  const doA = makeDO(A, bucket);
  const bytes = new TextEncoder().encode("hello hosted world");
  const put = await doA.fetch(routed("/v1/blobs", A, { method: "PUT", body: bytes }));
  assert.equal(put.status, 200);
  const putBody = (await put.json()) as { hash: string; size: number; deduped: boolean };
  assert.equal(putBody.hash, await sha256Hex(bytes));
  assert.equal(putBody.size, bytes.byteLength);
  assert.equal(putBody.deduped, false);
  assert.equal(bucket.objects.size, 1);

  const get = await doA.fetch(routed("/v1/blobs/" + putBody.hash, A));
  assert.equal(get.status, 200);
  assert.equal(get.headers.get("x-blob-size"), String(bytes.byteLength));
  assert.deepEqual(new Uint8Array(await get.arrayBuffer()), bytes);
});

test("user B cannot read user A's blob by hash; dedup grants B its own reference", async () => {
  const bucket = fakeBucket();
  const doA = makeDO(A, bucket);
  const doB = makeDO(B, bucket);
  const bytes = new TextEncoder().encode("shared content");
  const put = await doA.fetch(routed("/v1/blobs", A, { method: "PUT", body: bytes }));
  const { hash } = (await put.json()) as { hash: string };

  const stolen = await doB.fetch(routed("/v1/blobs/" + hash, B));
  assert.equal(stolen.status, 404, "B has no reference row, so A's blob is invisible by hash");

  const putB = await doB.fetch(routed("/v1/blobs", B, { method: "PUT", body: bytes }));
  const putBBody = (await putB.json()) as { hash: string; deduped: boolean };
  assert.equal(putBBody.hash, hash);
  assert.equal(putBBody.deduped, true, "identical bytes are stored once");
  assert.equal(bucket.objects.size, 1);
  const getB = await doB.fetch(routed("/v1/blobs/" + hash, B));
  assert.equal(getB.status, 200, "B's own reference makes the blob readable for B");
});

test("delete removes only the caller's reference; shared bytes survive", async () => {
  const bucket = fakeBucket();
  const doA = makeDO(A, bucket);
  const doB = makeDO(B, bucket);
  const bytes = new TextEncoder().encode("delete me not");
  const { hash } = (await (await doA.fetch(routed("/v1/blobs", A, { method: "PUT", body: bytes }))).json()) as {
    hash: string;
  };
  await doB.fetch(routed("/v1/blobs", B, { method: "PUT", body: bytes }));

  const del = await doA.fetch(routed("/v1/blobs/" + hash, A, { method: "DELETE" }));
  assert.equal(del.status, 200);
  assert.equal((await doA.fetch(routed("/v1/blobs/" + hash, A))).status, 404);
  assert.equal((await doB.fetch(routed("/v1/blobs/" + hash, B))).status, 200, "B's read is unaffected by A's delete");
  assert.equal(bucket.objects.size, 1, "byte reclamation is GC's job, not delete's");
  assert.equal((await doA.fetch(routed("/v1/blobs/" + hash, A, { method: "DELETE" }))).status, 404);
});

test("blob routes stay behind the identity guard; schema is v3", async () => {
  const bucket = fakeBucket();
  const doA = makeDO(A, bucket);
  const unrouted = new Request("https://do/v1/blobs", { method: "PUT", body: "x" });
  assert.equal((await doA.fetch(unrouted)).status, 403);
  const state = await doA.fetch(routed("/v1/state", A));
  const body = (await state.json()) as { schema_version: number };
  assert.equal(body.schema_version, 3);
  const badHash = await doA.fetch(routed("/v1/blobs/not-a-hash", A));
  assert.equal(badHash.status, 404);
});
