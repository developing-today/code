// #11337: the front door (worker.ts default export) must bound body
// memory before the DO's ceiling check can run. These tests go
// through the default export, not UserDO alone, with a namespace fake
// that routes to the real UserDO over in-memory SQLite and R2.
import { test } from "node:test";
import assert from "node:assert/strict";
import { DatabaseSync } from "node:sqlite";
import worker from "../src/worker.ts";
import { UserDO } from "../src/user-do.ts";

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
    async head(key: string) {
      const v = objects.get(key);
      return v ? { size: v.byteLength } : null;
    },
    async get(key: string) {
      const v = objects.get(key);
      return v ? { size: v.byteLength, body: new Blob([v]).stream() } : null;
    },
    async put(key: string, value: Uint8Array) {
      objects.set(key, value);
    },
    async delete(key: string) {
      objects.delete(key);
    },
  };
}

function makeEnv(vars: Record<string, string> = {}) {
  const calls = { stub: 0 };
  const userDO = new UserDO(
    {
      id: { name: A },
      storage: { sql: fakeSql() },
      blockConcurrencyWhile: async <T>(fn: () => Promise<T>): Promise<T> => fn(),
    } as never,
    { BLOBS: fakeBucket(), ...vars } as never,
  );
  const ns = {
    jurisdiction() {
      return ns;
    },
    idFromName(name: string) {
      return { name };
    },
    get() {
      return {
        fetch: (r: Request) => {
          calls.stub++;
          return userDO.fetch(r);
        },
      };
    },
  };
  return { env: { USERS: ns, ...vars } as never, calls };
}

function front(path: string, init: RequestInit = {}): Request {
  const headers = new Headers(init.headers);
  headers.set("x-binding-id", A);
  headers.set("x-jurisdiction", "us");
  return new Request("https://hosted.example" + path, { ...init, headers, duplex: "half" } as RequestInit);
}

/** A body stream that records how many bytes were pulled from it. */
function countingBody(chunks: Uint8Array[]) {
  const seen = { bytes: 0 };
  let i = 0;
  const body = new ReadableStream<Uint8Array>({
    pull(c) {
      if (i >= chunks.length) return c.close();
      const chunk = chunks[i++];
      seen.bytes += chunk.byteLength;
      c.enqueue(chunk);
    },
  }, { highWaterMark: 0 });
  return { body, seen };
}

test("a declared oversized body is refused at the front door without being read", async () => {
  const { env, calls } = makeEnv({ USER_CEILING_BYTES: "10" });
  const { body, seen } = countingBody([new Uint8Array(64)]);
  const r = await worker.fetch(
    front("/v1/blobs", { method: "PUT", body, headers: { "content-length": "64" } }),
    env,
  );
  assert.equal(r.status, 413);
  assert.equal(((await r.json()) as { error: string }).error, "body_too_large");
  assert.equal(seen.bytes, 0, "the body was never read");
  assert.equal(calls.stub, 0, "the DO was never reached");
});

test("a chunked oversized body stops being read at the limit plus one chunk", async () => {
  const { env } = makeEnv({ USER_CEILING_BYTES: "10" });
  const chunk = new Uint8Array(4);
  const { body, seen } = countingBody(Array.from({ length: 1000 }, () => chunk));
  const r = await worker.fetch(front("/v1/blobs", { method: "PUT", body }), env);
  assert.equal(r.status, 413);
  assert.ok(seen.bytes <= 10 + 2 * chunk.byteLength, `read ${seen.bytes} bytes of 4000`);

  const kv = countingBody(Array.from({ length: 1000 }, () => chunk));
  const r2 = await worker.fetch(front("/v1/kv/big", { method: "PUT", body: kv.body }), env);
  assert.equal(r2.status, 413);
  assert.ok(kv.seen.bytes <= 10 + 2 * chunk.byteLength, `read ${kv.seen.bytes} bytes of 4000`);
});

test("normal kv and blob PUTs still succeed end to end through the front door", async () => {
  const { env } = makeEnv({ USER_CEILING_BYTES: "100" });
  const kv = await worker.fetch(front("/v1/kv/note", { method: "PUT", body: "hello" }), env);
  assert.equal(kv.status, 200);
  const read = await worker.fetch(front("/v1/kv/note"), env);
  assert.equal(((await read.json()) as { value: string }).value, "hello");
  const blob = await worker.fetch(
    front("/v1/blobs", { method: "PUT", body: new TextEncoder().encode("blobbytes!") }),
    env,
  );
  assert.equal(blob.status, 200);
  const usage = (await (await worker.fetch(front("/v1/usage"), env)).json()) as { storage_bytes: number };
  assert.equal(usage.storage_bytes, 15);
});

test("with no ceiling, normal writes pass and the hard body bound still applies to declared length", async () => {
  const { env, calls } = makeEnv();
  assert.equal((await worker.fetch(front("/v1/kv/k", { method: "PUT", body: "x" }), env)).status, 200);
  const r = await worker.fetch(
    front("/v1/kv/k", { method: "PUT", body: "x", headers: { "content-length": String(200 * 1024 * 1024) } }),
    env,
  );
  assert.equal(r.status, 413);
  assert.equal(calls.stub, 1, "only the normal write reached the DO");
});
