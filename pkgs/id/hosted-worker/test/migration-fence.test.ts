// #3404 slice 1: the durable source fence. The fence lives in the user
// object's own storage and is checked at the commit point of every
// mutation; only the migration principal can freeze, seal or unfreeze;
// reads stay open; a replayed or stale transition is refused by epoch.
import { test } from "node:test";
import assert from "node:assert/strict";
import { DatabaseSync } from "node:sqlite";
import { UserDO, HDR_BINDING, HDR_JURISDICTION, HDR_ROUTED, HDR_IROH } from "../src/user-do.ts";
import {
  HDR_MIGRATION,
  parseFenceRequest,
  planFreeze,
  planSeal,
  planUnfreeze,
  readFence,
  type SourceFence,
} from "../src/migration-fence.ts";
import { FailClosed } from "../src/routing.ts";

const A = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const IROH = "11".repeat(32);

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

/** An R2 fake whose put can be held open, to land a freeze mid-write. */
function fakeBucket() {
  const objects = new Map<string, Uint8Array>();
  let hold: Promise<void> | null = null;
  return {
    objects,
    holdPuts(until: Promise<void>) {
      hold = until;
    },
    async head(key: string) {
      const v = objects.get(key);
      return v ? { size: v.byteLength } : null;
    },
    async get(key: string) {
      const v = objects.get(key);
      return v ? { size: v.byteLength, body: new Blob([v]).stream() } : null;
    },
    async put(key: string, value: Uint8Array) {
      if (hold) await hold;
      objects.set(key, value);
    },
    async delete(key: string) {
      objects.delete(key);
    },
  };
}

class FakeSocket {
  sent: Uint8Array[] = [];
  closed: { code?: number; reason?: string } | null = null;
  private attachment: unknown = { v: 1, sid: "sid-1", bindingId: A, irohHex: IROH };
  send(data: Uint8Array) {
    this.sent.push(data);
  }
  close(code?: number, reason?: string) {
    this.closed = { code, reason };
  }
  serializeAttachment(value: unknown) {
    this.attachment = value;
  }
  deserializeAttachment() {
    return this.attachment;
  }
}

function makeDO(sockets: FakeSocket[] = []) {
  const bucket = fakeBucket();
  const state = {
    id: { name: A },
    storage: { sql: fakeSql() },
    blockConcurrencyWhile: async <T>(fn: () => Promise<T>): Promise<T> => fn(),
    acceptWebSocket: () => {},
    getWebSockets: () => sockets,
  };
  return { d: new UserDO(state as never, { BLOBS: bucket } as never), bucket };
}

function routed(path: string, init: RequestInit = {}): Request {
  const headers = new Headers(init.headers);
  headers.set(HDR_BINDING, A);
  headers.set(HDR_JURISDICTION, "us");
  headers.set(HDR_ROUTED, "restricted");
  return new Request("https://do" + path, { ...init, headers });
}

function control(path: string, body?: unknown, principal = true): Request {
  const headers: Record<string, string> = { "content-type": "application/json" };
  if (principal) headers[HDR_MIGRATION] = "1";
  return routed(path, body === undefined ? { headers } : { method: "POST", headers, body: JSON.stringify(body) });
}

interface Status {
  fence: SourceFence | null;
  epoch_floor: number;
  mutation_seq: number;
  acknowledged?: boolean;
}

async function call(d: UserDO, req: Request): Promise<{ status: number; body: Record<string, unknown> }> {
  const r = await d.fetch(req);
  return { status: r.status, body: (await r.json()) as Record<string, unknown> };
}

async function freeze(d: UserDO, id = "m-1", epoch = 1) {
  return call(d, control("/v1/migration/freeze", { migration_id: id, routing_epoch: epoch }));
}

const put = (key: string, value: string) => routed(`/v1/kv/${key}`, { method: "PUT", body: value });

// ----- the pure transitions -----

test("planFreeze: fresh, retried, held, sealed and stale", () => {
  const req = { migrationId: "m-1", routingEpoch: 5 };
  const { fence, changed } = planFreeze(null, 4, req, 7, 1000);
  assert.equal(changed, true);
  assert.deepEqual(fence, {
    v: 1,
    migration_id: "m-1",
    routing_epoch: 5,
    state: "frozen",
    frozen_at_ms: 1000,
    watermark: 7,
  });
  assert.deepEqual(planFreeze(fence, 5, req, 9, 2000), { fence, changed: false });
  const code = (fn: () => unknown) => {
    try {
      fn();
    } catch (e) {
      return (e as FailClosed).code;
    }
    return "ok";
  };
  assert.equal(code(() => planFreeze(fence, 5, { migrationId: "m-2", routingEpoch: 6 }, 0, 0)), "fence_held");
  assert.equal(code(() => planFreeze(fence, 5, { migrationId: "m-1", routingEpoch: 6 }, 0, 0)), "stale_epoch");
  assert.equal(code(() => planFreeze({ ...fence, state: "sealed" }, 5, req, 0, 0)), "fence_sealed");
  assert.equal(code(() => planFreeze(null, 5, req, 0, 0)), "stale_epoch");
  assert.equal(planUnfreeze(null, req), 5, "an unfenced store still raises its epoch floor");
  assert.equal(code(() => planUnfreeze(fence, req)), "stale_epoch");
  assert.equal(planUnfreeze(fence, { migrationId: "m-1", routingEpoch: 6 }), 6);
  assert.equal(code(() => planUnfreeze({ ...fence, state: "sealed" }, { migrationId: "m-1", routingEpoch: 9 })), "fence_sealed");
  assert.equal(code(() => planSeal(fence, { migrationId: "m-1", routingEpoch: 4 })), "stale_epoch");
  const sealed = planSeal(fence, { migrationId: "m-1", routingEpoch: 6 });
  assert.deepEqual(sealed, { fence: { ...fence, state: "sealed", routing_epoch: 6 }, changed: true });
  assert.deepEqual(planSeal(sealed.fence, { migrationId: "m-1", routingEpoch: 6 }), { fence: sealed.fence, changed: false });
});

test("control bodies and stored fences are validated strictly", () => {
  assert.deepEqual(parseFenceRequest({ migration_id: "m:1", routing_epoch: 2 }), { migrationId: "m:1", routingEpoch: 2 });
  for (const bad of [{}, { migration_id: "", routing_epoch: 1 }, { migration_id: "a b", routing_epoch: 1 }, { migration_id: "m", routing_epoch: 0 }, { migration_id: "m", routing_epoch: 1.5 }, { migration_id: "m", routing_epoch: "1" }]) {
    assert.throws(() => parseFenceRequest(bad), FailClosed);
  }
  assert.equal(readFence(null), null);
  assert.throws(() => readFence("{"), /unreadable/);
  assert.throws(() => readFence(JSON.stringify({ v: 1, migration_id: "m", routing_epoch: 1, state: "open", frozen_at_ms: 0, watermark: 0 })), /unreadable/);
});

// ----- the fence on the object -----

test("only the migration principal reaches the control routes", async () => {
  const { d } = makeDO();
  const refused = await call(d, control("/v1/migration/freeze", { migration_id: "m-1", routing_epoch: 1 }, false));
  assert.equal(refused.status, 403);
  assert.equal(refused.body.error, "migration_principal_required");
  assert.equal((await call(d, control("/v1/migration/fence", undefined, false))).status, 403);
  assert.equal((await call(d, put("k", "v"))).status, 200, "nothing was frozen");
});

test("a freeze acknowledges the watermark, refuses every mutation, and keeps reads open", async () => {
  const { d } = makeDO();
  assert.equal((await call(d, put("a", "1"))).status, 200);
  assert.equal((await call(d, put("b", "2"))).status, 200);
  const blob = await call(d, routed("/v1/blobs", { method: "PUT", body: "bytes" }));
  assert.equal(blob.status, 200);

  const ack = await freeze(d, "m-1", 3);
  assert.equal(ack.status, 200);
  const status = ack.body as unknown as Status;
  assert.equal(status.acknowledged, true);
  assert.equal(status.mutation_seq, 3);
  assert.equal(status.fence?.watermark, 3);
  assert.equal(status.fence?.state, "frozen");
  assert.equal(status.epoch_floor, 3);

  const refusals = [
    put("a", "changed"),
    routed("/v1/kv/a", { method: "DELETE" }),
    routed("/v1/blobs", { method: "PUT", body: "more" }),
    routed(`/v1/blobs/${blob.body.hash}`, { method: "DELETE" }),
  ];
  for (const req of refusals) {
    const r = await d.fetch(req);
    assert.equal(r.status, 503, `${req.method} ${new URL(req.url).pathname}`);
    assert.equal(r.headers.get("retry-after"), "30");
    assert.equal(((await r.json()) as { error: string }).error, "source_frozen");
  }
  const ws = await d.fetch(routed("/v1/bridge/ws", { headers: { upgrade: "websocket", [HDR_IROH]: IROH } }));
  assert.equal(ws.status, 503, "no new bridge session while frozen");

  assert.equal(((await call(d, routed("/v1/kv/a"))).body as { value: string }).value, "1");
  assert.equal((await d.fetch(routed(`/v1/blobs/${blob.body.hash}`))).status, 200);
  assert.equal((await d.fetch(routed("/v1/export"))).status, 200);
  const after = (await call(d, control("/v1/migration/fence"))).body as unknown as Status;
  assert.equal(after.mutation_seq, 3, "the watermark never moved");

  const retried = await freeze(d, "m-1", 3);
  assert.equal(retried.status, 200, "a retried freeze is answered");
  assert.equal((retried.body as unknown as Status).fence?.frozen_at_ms, status.fence?.frozen_at_ms);
  assert.equal((await freeze(d, "m-2", 4)).body.error, "fence_held");
});

test("a freeze that lands during a blob write's R2 await refuses its commit", async () => {
  const { d, bucket } = makeDO();
  let release!: () => void;
  bucket.holdPuts(new Promise<void>((resolve) => (release = resolve)));
  const pending = d.fetch(routed("/v1/blobs", { method: "PUT", body: "racing" }));
  await new Promise((resolve) => setTimeout(resolve, 10));
  assert.equal((await freeze(d)).status, 200);
  release();
  const r = await pending;
  assert.equal(r.status, 503);
  assert.equal(((await r.json()) as { error: string }).error, "source_frozen");
  const status = (await call(d, control("/v1/migration/fence"))).body as unknown as Status;
  assert.equal(status.mutation_seq, 0, "the racing write never committed");
  const usage = (await call(d, routed("/v1/usage"))).body as { blob_count?: number; storage_bytes: number };
  assert.equal(usage.storage_bytes, 0, "no reference row was written");
});

test("a freeze closes live bridge sessions with a named error", async () => {
  const socket = new FakeSocket();
  const { d } = makeDO([socket]);
  assert.equal((await freeze(d)).status, 200);
  assert.deepEqual(socket.closed, { code: 1013, reason: "source_frozen" });
  const frame = socket.sent.at(-1)!;
  assert.match(new TextDecoder().decode(frame.subarray(1)), /source_frozen/);
});

test("an abort unfreezes at a newer epoch, and a replayed freeze cannot re-freeze", async () => {
  const { d } = makeDO();
  assert.equal((await freeze(d, "m-1", 1)).status, 200);
  const same = await call(d, control("/v1/migration/unfreeze", { migration_id: "m-1", routing_epoch: 1 }));
  assert.equal(same.body.error, "stale_epoch");
  const other = await call(d, control("/v1/migration/unfreeze", { migration_id: "m-9", routing_epoch: 2 }));
  assert.equal(other.body.error, "fence_held");
  const open = await call(d, control("/v1/migration/unfreeze", { migration_id: "m-1", routing_epoch: 2 }));
  assert.equal(open.status, 200);
  assert.equal((open.body as unknown as Status).fence, null);
  assert.equal((await call(d, put("k", "v"))).status, 200, "the source reopened");
  assert.equal((await freeze(d, "m-1", 1)).body.error, "stale_epoch", "the aborted migration cannot re-freeze");
  assert.equal((await freeze(d, "m-1", 2)).body.error, "stale_epoch");
  assert.equal((await freeze(d, "m-2", 3)).status, 200, "a new migration at a newer epoch can");
});

test("after the switch the source is sealed for good", async () => {
  const { d } = makeDO();
  assert.equal((await call(d, control("/v1/migration/seal", { migration_id: "m-1", routing_epoch: 2 }))).body.error, "not_fenced");
  await freeze(d, "m-1", 1);
  const sealed = await call(d, control("/v1/migration/seal", { migration_id: "m-1", routing_epoch: 2 }));
  assert.equal(sealed.status, 200);
  assert.equal((sealed.body as unknown as Status).fence?.state, "sealed");
  assert.equal((await call(d, control("/v1/migration/seal", { migration_id: "m-1", routing_epoch: 2 }))).status, 200);
  const write = await d.fetch(put("k", "v"));
  assert.equal(write.status, 503);
  assert.equal(((await write.json()) as { error: string }).error, "source_sealed");
  const reopen = await call(d, control("/v1/migration/unfreeze", { migration_id: "m-1", routing_epoch: 9 }));
  assert.equal(reopen.body.error, "fence_sealed");
  assert.equal((await freeze(d, "m-2", 10)).body.error, "fence_held");
});

test("control routes ignore the request cap and reject bad bodies", async () => {
  const bucket = fakeBucket();
  const state = {
    id: { name: A },
    storage: { sql: fakeSql() },
    blockConcurrencyWhile: async <T>(fn: () => Promise<T>): Promise<T> => fn(),
  };
  const d = new UserDO(state as never, { BLOBS: bucket, USER_REQUEST_CEILING: "1", USER_REQUEST_WINDOW_SECONDS: "60" } as never);
  assert.equal((await d.fetch(put("k", "v"))).status, 200);
  assert.equal((await d.fetch(put("k", "w"))).status, 429);
  assert.equal((await freeze(d)).status, 200, "a capped tenant can still be frozen");
  assert.equal((await call(d, control("/v1/migration/freeze", { migration_id: "", routing_epoch: 1 }))).body.error, "invalid_migration_id");
  const notJson = await call(d, routed("/v1/migration/freeze", { method: "POST", headers: { [HDR_MIGRATION]: "1" }, body: "{" }));
  assert.equal(notJson.body.error, "invalid_body");
  assert.equal((await call(d, control("/v1/migration/nope", { migration_id: "m", routing_epoch: 1 }))).status, 404);
});
