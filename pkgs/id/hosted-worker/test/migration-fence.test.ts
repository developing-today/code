import { test } from "node:test";
import assert from "node:assert/strict";
import { DatabaseSync } from "node:sqlite";
import { HDR_BINDING, HDR_JURISDICTION, HDR_ROUTED, UserDO } from "../src/user-do.ts";

const BINDING = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";

function database(options: { failOpSequenceOnce?: boolean } = {}) {
  const db = new DatabaseSync(":memory:");
  let failOpSequenceOnce = options.failOpSequenceOnce ?? false;
  return {
    exec(query: string, ...params: unknown[]) {
      if (failOpSequenceOnce && /UPDATE meta SET value = CAST\(value AS INTEGER\) \+ 1/.test(query)) {
        failOpSequenceOnce = false;
        throw new Error("injected op sequence failure");
      }
      const stmt = db.prepare(query);
      if (/^\s*select/i.test(query)) {
        const rows = stmt.all(...(params as never[])) as Record<string, unknown>[];
        return { toArray: () => rows };
      }
      stmt.run(...(params as never[]));
      return { toArray: () => [] as Record<string, unknown>[] };
    },
    transactionSync<T>(fn: () => T): T {
      db.exec("BEGIN");
      try {
        const result = fn();
        db.exec("COMMIT");
        return result;
      } catch (error) {
        db.exec("ROLLBACK");
        throw error;
      }
    },
  };
}

function makeDO(sql = database(), bucket = {
  async head() { return null; }, async get() { return null; }, async put() {}, async delete() {},
}) {
  const state = { id: { name: BINDING }, storage: { sql, transactionSync: sql.transactionSync }, blockConcurrencyWhile: async <T>(fn: () => Promise<T>) => fn() };
  return { object: new UserDO(state as never, { BLOBS: bucket } as never), sql };
}

function routed(path: string, init: RequestInit = {}) {
  const headers = new Headers(init.headers);
  headers.set(HDR_BINDING, BINDING);
  headers.set(HDR_JURISDICTION, "us");
  headers.set(HDR_ROUTED, "restricted");
  return new Request(`https://do${path}`, { ...init, headers });
}

async function bind(object: UserDO) {
  assert.equal((await object.fetch(routed("/v1/state"))).status, 200);
}

test("freeze computes a durable watermark, is idempotent, survives restart, and is not HTTP-reachable", async () => {
  const { object, sql } = makeDO();
  await object.fetch(routed("/v1/kv/a", { method: "PUT", body: "one" }));
  await bind(object);
  // Public routing does not provide a freeze handler, even with forged claims.
  assert.equal((await object.fetch(routed("/v1/migration/freeze?migration_id=m&source_jurisdiction=us&epoch=2&watermark=999&drained=1", { method: "POST" }))).status, 404);
  const first = await object.freezeSource({ migrationId: "m", sourceJurisdiction: "us", epoch: 2 });
  assert.deepEqual(first, { migrationId: "m", sourceJurisdiction: "us", epoch: 2, watermark: 1, drained: true });
  assert.deepEqual(await object.freezeSource({ migrationId: "m", sourceJurisdiction: "us", epoch: 2 }), first);
  assert.equal((await object.fetch(routed("/v1/kv/a", { method: "PUT", body: "two" }))).status, 423);
  assert.equal((await object.fetch(routed("/v1/kv/a"))).status, 200);
  assert.equal((await object.fetch(routed("/v1/export"))).status, 200);
  assert.equal((await object.freezeAckFor("m"))?.watermark, 1);
  await assert.rejects(() => object.freezeSource({ migrationId: "m", sourceJurisdiction: "us", epoch: 3 }), /epoch/);
  await assert.rejects(() => object.freezeSource({ migrationId: "other", sourceJurisdiction: "us", epoch: 2 }), /another migration/);
  const restarted = makeDO(sql).object;
  assert.deepEqual(restarted.freezeAckFor("m"), await object.freezeAckFor("m"));
  assert.equal((await restarted.fetch(routed("/v1/kv/a", { method: "DELETE" }))).status, 423);
});

test("freeze rejects wrong or unbound identity and fences every user-data HTTP mutation", async () => {
  const { object } = makeDO();
  await bind(object);
  await object.fetch(routed("/v1/kv/kept", { method: "PUT", body: "readable" }));
  const blob = new TextEncoder().encode("retained blob");
  const blobPut = await object.fetch(routed("/v1/blobs", { method: "PUT", body: blob }));
  const { hash } = (await blobPut.json()) as { hash: string };

  await assert.rejects(
    () => object.freezeSource({ migrationId: "wrong-jurisdiction", sourceJurisdiction: "eu", epoch: 1 }),
    /persisted object identity/,
  );
  const unbound = makeDO().object;
  await assert.rejects(
    () => unbound.freezeSource({ migrationId: "unbound", sourceJurisdiction: "us", epoch: 1 }),
    /persisted object identity/,
  );

  await object.freezeSource({ migrationId: "all-mutations", sourceJurisdiction: "us", epoch: 1 });
  for (const request of [
    routed("/v1/kv/kept", { method: "PUT", body: "changed" }),
    routed("/v1/kv/kept", { method: "DELETE" }),
    routed("/v1/blobs", { method: "PUT", body: "new blob" }),
    routed(`/v1/blobs/${hash}`, { method: "DELETE" }),
    routed("/v1/bridge/push", { method: "POST", body: new Uint8Array([0, 0, 0, 0]) }),
  ]) {
    assert.equal((await object.fetch(request)).status, 423);
  }
  assert.equal((await object.fetch(routed("/v1/kv/kept"))).status, 200);
  assert.equal((await object.fetch(routed("/v1/state"))).status, 200);
  assert.equal((await object.fetch(routed("/v1/export"))).status, 200);
});

test("freeze waits for a streamed KV mutation then excludes its refused body from watermark", async () => {
  const { object } = makeDO();
  await bind(object);
  let release!: () => void;
  const body = new ReadableStream<Uint8Array>({
    start(controller) {
      controller.enqueue(new TextEncoder().encode("partial"));
      void new Promise<void>((resolve) => { release = () => { controller.close(); resolve(); }; });
    },
  });
  const write = object.fetch(routed("/v1/kv/slow", { method: "PUT", body, duplex: "half" } as RequestInit));
  await new Promise<void>((resolve) => queueMicrotask(resolve));
  let freezeDone = false;
  const freezing = object.freezeSource({ migrationId: "stream", sourceJurisdiction: "us", epoch: 4 }).then((ack) => { freezeDone = true; return ack; });
  await new Promise<void>((resolve) => queueMicrotask(resolve));
  assert.equal(freezeDone, false, "freeze must not acknowledge while a streamed request is still active");
  release();
  const [response, ack] = await Promise.all([write, freezing]);
  assert.equal(response.status, 423);
  assert.equal(ack.watermark, 0);
  assert.equal(ack.drained, true);
});

test("freeze drains an R2 put before acknowledging and never commits its reference", async () => {
  const { promise, resolve } = Promise.withResolvers<void>();
  const bucket = {
    async head() { return null; }, async get() { return null; }, async delete() {},
    async put() { await promise; },
  };
  const { object, sql } = makeDO(database(), bucket);
  await bind(object);
  const upload = object.fetch(routed("/v1/blobs", { method: "PUT", body: new TextEncoder().encode("r2-pending") }));
  await new Promise<void>((resolveNext) => queueMicrotask(resolveNext));
  let freezeDone = false;
  const freezing = object.freezeSource({ migrationId: "r2", sourceJurisdiction: "us", epoch: 5 }).then((ack) => { freezeDone = true; return ack; });
  await new Promise<void>((resolveNext) => queueMicrotask(resolveNext));
  assert.equal(freezeDone, false, "R2 work must be serialized before freeze ack");
  resolve();
  const [response, ack] = await Promise.all([upload, freezing]);
  assert.equal(response.status, 423);
  assert.equal(ack.watermark, 0);
  assert.equal(Number(sql.exec("SELECT COUNT(*) AS n FROM blobs").toArray()[0].n), 0);
});

test("user data and op sequence roll back together if sequence advancement fails", async () => {
  const sql = database({ failOpSequenceOnce: true });
  const { object } = makeDO(sql);
  await bind(object);
  await assert.rejects(
    () => object.fetch(routed("/v1/kv/atomic", { method: "PUT", body: "value" })),
    /injected op sequence failure/,
  );
  assert.equal(Number(sql.exec("SELECT COUNT(*) AS n FROM kv WHERE key = 'atomic'").toArray()[0].n), 0);
  assert.equal(Number(sql.exec("SELECT value FROM meta WHERE key = 'op_seq'").toArray()[0].value), 0);
});
