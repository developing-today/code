// #3404 slice 3: the source hands out a snapshot only behind its
// fence; the target imports it into a fresh object that stays
// non-serving until activation after the switch, and both sides agree
// on one digest.
import { test } from "node:test";
import assert from "node:assert/strict";
import { DatabaseSync } from "node:sqlite";
import { UserDO, HDR_BINDING, HDR_JURISDICTION, HDR_ROUTED } from "../src/user-do.ts";
import { HDR_MIGRATION } from "../src/migration-fence.ts";
import { MIGRATED_TABLES, parseSnapshot, type MigrationSnapshot } from "../src/migration-snapshot.ts";
import { FailClosed } from "../src/routing.ts";

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

function makeDO(jurisdiction: string) {
  const bucket = fakeBucket();
  const state = {
    id: { name: A },
    storage: { sql: fakeSql() },
    blockConcurrencyWhile: async <T>(fn: () => Promise<T>): Promise<T> => fn(),
    acceptWebSocket: () => {},
    getWebSockets: () => [],
  };
  const env =
    jurisdiction === "us"
      ? { BLOBS_US: bucket }
      : jurisdiction === "eu"
        ? { BLOBS_EU: bucket }
        : { BLOBS_FEDRAMP: bucket };
  return { d: new UserDO(state as never, env as never), bucket, jurisdiction };
}

type Obj = ReturnType<typeof makeDO>;

function routed(o: Obj, path: string, init: RequestInit = {}): Request {
  const headers = new Headers(init.headers);
  headers.set(HDR_BINDING, A);
  headers.set(HDR_JURISDICTION, o.jurisdiction);
  headers.set(HDR_ROUTED, "restricted");
  return new Request("https://do" + path, { ...init, headers });
}

function control(o: Obj, path: string, body?: unknown): Request {
  const headers: Record<string, string> = { "content-type": "application/json", [HDR_MIGRATION]: "1" };
  return routed(o, path, body === undefined ? { headers } : { method: "POST", headers, body: JSON.stringify(body) });
}

async function call(
  o: Obj,
  req: Request,
): Promise<{ status: number; body: Record<string, unknown>; retryAfter: string | null }> {
  const r = await o.d.fetch(req);
  return {
    status: r.status,
    body: (await r.json()) as Record<string, unknown>,
    retryAfter: r.headers.get("retry-after"),
  };
}

const step = (o: Obj, path: string, id = "m-1", epoch = 1) =>
  call(o, control(o, `/v1/migration/${path}`, { migration_id: id, routing_epoch: epoch }));

async function seedSource(): Promise<Obj> {
  const src = makeDO("us");
  assert.equal((await call(src, routed(src, "/v1/kv/note", { method: "PUT", body: "hello" }))).status, 200);
  assert.equal((await call(src, routed(src, "/v1/kv/other", { method: "PUT", body: "world" }))).status, 200);
  const blob = await call(
    src,
    routed(src, "/v1/blobs", { method: "PUT", body: new TextEncoder().encode("blob bytes") }),
  );
  assert.equal(blob.status, 200);
  return src;
}

async function snapshotOf(src: Obj): Promise<MigrationSnapshot> {
  assert.equal((await step(src, "freeze")).status, 200);
  const snap = await step(src, "snapshot");
  assert.equal(snap.status, 200, JSON.stringify(snap.body));
  return snap.body as unknown as MigrationSnapshot;
}

test("the source refuses a snapshot until the freeze is acknowledged for that migration and epoch", async () => {
  const src = await seedSource();
  assert.equal((await step(src, "snapshot")).body.error, "freeze_not_acknowledged");
  await step(src, "freeze");
  assert.equal((await step(src, "snapshot", "m-2")).body.error, "freeze_not_acknowledged");
  assert.equal((await step(src, "snapshot", "m-1", 2)).body.error, "freeze_not_acknowledged");
  const snap = (await step(src, "snapshot")).body as unknown as MigrationSnapshot;
  assert.equal(snap.watermark, 3, "the watermark is the source's mutation count at the freeze");
  assert.deepEqual(
    snap.tables.kv.map((r) => r.key),
    ["note", "other"],
  );
  assert.equal(snap.tables.blobs.length, 1);
  assert.match(snap.digest, /^[0-9a-f]{64}$/);
  const again = (await step(src, "snapshot")).body as unknown as MigrationSnapshot;
  assert.equal(again.digest, snap.digest, "a frozen source yields the same snapshot every time");
});

test("a client cannot ask for a snapshot without the migration principal", async () => {
  const src = await seedSource();
  await step(src, "freeze");
  const r = await call(
    src,
    routed(src, "/v1/migration/snapshot", {
      method: "POST",
      body: JSON.stringify({ migration_id: "m-1", routing_epoch: 1 }),
    }),
  );
  assert.equal(r.status, 403);
});

test("import, stage, then activate after the switch: the target serves the source's data", async () => {
  const src = await seedSource();
  const snap = await snapshotOf(src);
  const dst = makeDO("eu");

  const imported = await call(dst, control(dst, "/v1/migration/import", snap));
  assert.equal(imported.status, 200, JSON.stringify(imported.body));
  assert.equal((imported.body.staged as { digest: string }).digest, snap.digest);

  const blocked = await call(dst, routed(dst, "/v1/kv/note"));
  assert.equal(blocked.status, 503);
  assert.equal(blocked.body.error, "target_not_serving");
  assert.equal(blocked.retryAfter, "30");
  assert.equal(
    (await call(dst, routed(dst, "/v1/kv/new", { method: "PUT", body: "x" }))).body.error,
    "target_not_serving",
  );

  const replay = await call(dst, control(dst, "/v1/migration/import", snap));
  assert.equal(replay.body.idempotent, true, "a retried import is idempotent");

  assert.equal((await step(dst, "activate", "m-1", 1)).body.error, "stale_epoch", "activation needs the switch epoch");
  const early = await step(dst, "activate", "m-1", 2);
  assert.equal(early.body.error, "target_blobs_missing", "blob bytes must be copied first");

  for (const [key, value] of src.bucket.objects) dst.bucket.objects.set(key, value);
  const active = await step(dst, "activate", "m-1", 2);
  assert.equal(active.status, 200, JSON.stringify(active.body));
  assert.equal((await step(dst, "activate", "m-1", 2)).body.idempotent, true);

  const read = await call(dst, routed(dst, "/v1/kv/note"));
  assert.equal(read.status, 200);
  assert.equal(read.body.value, "hello");
  const exported = await call(dst, routed(dst, "/v1/export"));
  assert.deepEqual(exported.body.kv, { note: "hello", other: "world" });
  assert.equal((exported.body.blobs as unknown[]).length, 1);
  assert.equal(exported.body.jurisdiction, "eu");
  assert.equal((await call(dst, routed(dst, "/v1/kv/after", { method: "PUT", body: "moved" }))).status, 200);
});

test("the target refuses a tampered, foreign or non-fresh import and leaves itself empty", async () => {
  const src = await seedSource();
  const snap = await snapshotOf(src);

  const tampered = makeDO("eu");
  const bad = structuredClone(snap);
  bad.tables.kv[0].value = "changed";
  assert.equal(
    (await call(tampered, control(tampered, "/v1/migration/import", bad))).body.error,
    "snapshot_digest_mismatch",
  );
  assert.equal((await call(tampered, routed(tampered, "/v1/kv/note"))).status, 404, "nothing was written");

  const used = makeDO("eu");
  await call(used, routed(used, "/v1/kv/mine", { method: "PUT", body: "x" }));
  assert.equal((await call(used, control(used, "/v1/migration/import", snap))).body.error, "target_not_empty");

  const other = makeDO("eu");
  await call(other, control(other, "/v1/migration/import", snap));
  const second = { ...snap, migration_id: "m-2" };
  assert.equal((await call(other, control(other, "/v1/migration/import", second))).body.error, "target_staged_other");

  const foreign = makeDO("eu");
  const wrong = { ...snap, client_side_encryption: true };
  assert.equal((await call(foreign, control(foreign, "/v1/migration/import", wrong))).body.error, "identity_mismatch");
});

test("discarding a staged import empties the target so the migration can be retried", async () => {
  const src = await seedSource();
  const snap = await snapshotOf(src);
  const dst = makeDO("eu");
  await call(dst, control(dst, "/v1/migration/import", snap));
  assert.equal((await step(dst, "discard", "m-9")).body.error, "not_staged");
  assert.equal((await step(dst, "discard")).status, 200);
  const status = await call(dst, control(dst, "/v1/migration/target"));
  assert.equal(status.body.staged, null);
  assert.equal((await call(dst, control(dst, "/v1/migration/import", snap))).status, 200, "a fresh import works again");
});

test("bridge chain rows move with the snapshot", async () => {
  const src = makeDO("us");
  await call(src, routed(src, "/v1/kv/k", { method: "PUT", body: "v" }));
  // Seed the chain store directly: the snapshot carries rows, not semantics.
  const sql = (src.d as unknown as { state: { storage: { sql: ReturnType<typeof fakeSql> } } }).state.storage.sql;
  sql.exec(
    "INSERT INTO bridge_events (author, seq, id, bytes) VALUES (?, ?, ?, ?)",
    "author-1",
    0,
    "id-0",
    new Uint8Array([1, 2, 3, 255]),
  );
  sql.exec("INSERT INTO bridge_chains (author, next_seq, head, frozen) VALUES (?, ?, ?, ?)", "author-1", 1, "id-0", 0);
  const snap = await snapshotOf(src);
  assert.deepEqual(snap.tables.bridge_events[0].bytes, { b64: "AQID/w==" });
  const dst = makeDO("eu");
  assert.equal((await call(dst, control(dst, "/v1/migration/import", snap))).status, 200);
  const dsql = (dst.d as unknown as { state: { storage: { sql: ReturnType<typeof fakeSql> } } }).state.storage.sql;
  const rows = dsql.exec("SELECT bytes FROM bridge_events").toArray();
  assert.deepEqual([...(rows[0].bytes as Uint8Array)], [1, 2, 3, 255]);
});

test("parseSnapshot rejects unknown tables, bad columns and bad cells", () => {
  const base = {
    v: 1,
    migration_id: "m-1",
    routing_epoch: 1,
    binding_id: A,
    client_side_encryption: false,
    schema_version: 3,
    watermark: 0,
    digest: "0".repeat(64),
    tables: Object.fromEntries(MIGRATED_TABLES.map((t) => [t, []])),
  };
  const code = (body: unknown) => {
    try {
      parseSnapshot(body);
      return "ok";
    } catch (e) {
      return (e as FailClosed).code;
    }
  };
  assert.equal(code(base), "ok");
  assert.equal(code({ ...base, tables: { ...base.tables, meta: [] } }), "invalid_snapshot");
  assert.equal(code({ ...base, tables: { ...base.tables, kv: [{ "key; DROP": "x" }] } }), "invalid_snapshot");
  assert.equal(code({ ...base, tables: { ...base.tables, kv: [{ key: 1.5 }] } }), "invalid_snapshot");
  assert.equal(code({ ...base, tables: { ...base.tables, kv: [{ key: { b64: 1 } }] } }), "invalid_snapshot");
  assert.equal(code({ ...base, digest: "nope" }), "invalid_snapshot");
  assert.equal(code({ ...base, routing_epoch: 0 }), "invalid_snapshot");
});
