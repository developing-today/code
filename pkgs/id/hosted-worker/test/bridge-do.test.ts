// The bridge at the Durable Object boundary (src/user-do.ts):
// the HTTPS push/pull legs with per-call ID-key proofs, the
// WebSocket leg end to end through the real route, and the
// fail-closed edges. Same harness style as usage.test.ts, plus a
// fake WebSocketPair for the session leg.
import { test } from "node:test";
import assert from "node:assert/strict";
import { DatabaseSync } from "node:sqlite";
import { HDR_BINDING, HDR_IROH, HDR_JURISDICTION, HDR_ROUTED, UserDO } from "../src/user-do.ts";
import { authTranscript, bodyHashHex } from "../src/bridge-auth.ts";
import { BridgeStore, BRIDGE_SCHEMA } from "../src/bridge-store.ts";
import { bytesToHex, importIdKey, type IdKey } from "../src/ed25519.ts";
import { signEvent, type SignedEvent } from "../src/event.ts";
import { BridgeClient, decodeBatch, encodeBatch, type FrameTransport } from "../src/bridge-session.ts";

const BINDING = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";

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
  const sql = fakeSql();
  const state = {
    id: { name: BINDING },
    storage: { sql },
    blockConcurrencyWhile: async <T>(fn: () => Promise<T>): Promise<T> => fn(),
  };
  return { doA: new UserDO(state as never, { BLOBS: bucket, ...env } as never), bucket, sql };
}

function routed(path: string, irohHex: string | null, init: RequestInit = {}): Request {
  const headers = new Headers(init.headers);
  headers.set(HDR_BINDING, BINDING);
  headers.set(HDR_JURISDICTION, "us");
  headers.set(HDR_ROUTED, "restricted");
  if (irohHex !== null) headers.set(HDR_IROH, irohHex);
  return new Request("https://do" + path, { ...init, headers });
}

async function challenge(doA: UserDO, irohHex: string): Promise<string> {
  const res = await doA.fetch(routed("/v1/bridge/challenge", irohHex));
  assert.equal(res.status, 200);
  const body = (await res.json()) as { nonce: string };
  return body.nonce;
}

async function proofHeaders(
  key: IdKey,
  nonce: string,
  context: "push" | "pull" | "heads",
  body: Uint8Array,
): Promise<Record<string, string>> {
  const transcript = authTranscript(BINDING, nonce, context, bodyHashHex(body));
  const signature = await key.sign(transcript);
  return { "x-id-nonce": nonce, "x-id-proof": bytesToHex(signature) };
}

async function chainOf(key: IdKey, count: number): Promise<SignedEvent[]> {
  const events: SignedEvent[] = [];
  for (let seq = 0; seq < count; seq++) {
    events.push(
      await signEvent(key, {
        seq,
        backlink: seq === 0 ? null : events[seq - 1].idBytes,
        kind: "message",
        body: new TextEncoder().encode(`body ${seq}`),
      }),
    );
  }
  return events;
}

test("HTTPS push/pull round trip with per-call proofs", async () => {
  const key = await importIdKey(new Uint8Array(32).fill(61));
  const author = await importIdKey(new Uint8Array(32).fill(62));
  const { doA } = makeDO();
  const events = await chainOf(author, 2);
  const batch = encodeBatch(events.map((e) => e.bytes));

  const nonce = await challenge(doA, key.publicHex);
  const headers = await proofHeaders(key, nonce, "push", batch);
  const push = await doA.fetch(
    routed("/v1/bridge/push", key.publicHex, { method: "POST", headers, body: batch as BodyInit }),
  );
  assert.equal(push.status, 200);
  const pushBody = (await push.json()) as { outcomes: { kind: string }[]; heads: { next_seq: number }[] };
  assert.deepEqual(
    pushBody.outcomes.map((o) => o.kind),
    ["accepted", "accepted"],
  );
  assert.equal(pushBody.heads[0].next_seq, 2);

  // Pull from position zero replays the store; from the head, nothing.
  const pullBody = new TextEncoder().encode(JSON.stringify({ positions: [{ author: author.publicHex, next_seq: 0 }] }));
  const pullNonce = await challenge(doA, key.publicHex);
  const pullHeaders = await proofHeaders(key, pullNonce, "pull", pullBody);
  const pull = await doA.fetch(
    routed("/v1/bridge/pull", key.publicHex, { method: "POST", headers: pullHeaders, body: pullBody as BodyInit }),
  );
  assert.equal(pull.status, 200);
  assert.equal(pull.headers.get("x-bridge-more"), "0");
  const pulled = decodeBatch(new Uint8Array(await pull.arrayBuffer()), {
    maxEventBytes: 65536,
    maxBatchEvents: 64,
    maxBatchBytes: 1 << 20,
    maxWantIds: 512,
  });
  assert.deepEqual(
    pulled,
    events.map((e) => e.bytes),
  );

  // Usage counts the bridge bytes.
  const usage = (await (await doA.fetch(routed("/v1/usage", null))).json()) as { bridge_bytes: number };
  assert.equal(
    usage.bridge_bytes,
    events.reduce((sum, e) => sum + e.bytes.length, 0),
  );
});

test("bridge HTTPS legs fail closed: no key, no proof, wrong key, replayed nonce", async () => {
  const key = await importIdKey(new Uint8Array(32).fill(61));
  const wrong = await importIdKey(new Uint8Array(32).fill(63));
  const { doA } = makeDO();
  // No bound key in the routing context: even the challenge refuses.
  assert.equal((await doA.fetch(routed("/v1/bridge/challenge", null))).status, 503);
  // No proof headers: 401.
  const batch = encodeBatch([]);
  assert.equal(
    (await doA.fetch(routed("/v1/bridge/push", key.publicHex, { method: "POST", body: batch as BodyInit }))).status,
    401,
  );
  // A proof from the wrong key: 403.
  const nonce = await challenge(doA, key.publicHex);
  const badHeaders = await proofHeaders(wrong, nonce, "push", batch);
  assert.equal(
    (
      await doA.fetch(
        routed("/v1/bridge/push", key.publicHex, { method: "POST", headers: badHeaders, body: batch as BodyInit }),
      )
    ).status,
    403,
  );
  // A good proof works once; the same nonce replayed is refused.
  const nonce2 = await challenge(doA, key.publicHex);
  const good = await proofHeaders(key, nonce2, "push", batch);
  assert.equal(
    (
      await doA.fetch(
        routed("/v1/bridge/push", key.publicHex, { method: "POST", headers: good, body: batch as BodyInit }),
      )
    ).status,
    200,
  );
  assert.equal(
    (
      await doA.fetch(
        routed("/v1/bridge/push", key.publicHex, { method: "POST", headers: good, body: batch as BodyInit }),
      )
    ).status,
    403,
  );
});

test("the storage ceiling refuses a push before anything is written", async () => {
  const key = await importIdKey(new Uint8Array(32).fill(61));
  const author = await importIdKey(new Uint8Array(32).fill(64));
  const { doA, sql } = makeDO({ USER_CEILING_BYTES: "10" });
  const events = await chainOf(author, 1);
  const batch = encodeBatch(events.map((e) => e.bytes));
  const nonce = await challenge(doA, key.publicHex);
  const headers = await proofHeaders(key, nonce, "push", batch);
  const res = await doA.fetch(
    routed("/v1/bridge/push", key.publicHex, { method: "POST", headers, body: batch as BodyInit }),
  );
  assert.equal(res.status, 413);
  const store = new BridgeStore(sql as never);
  void store;
  const rows = sql.exec("SELECT COUNT(*) AS n FROM bridge_events").toArray();
  assert.equal(Number(rows[0].n), 0);
});

// ----- the WebSocket leg ---------------------------------------------

class FakeSocket {
  peer!: FakeSocket;
  private listeners = new Map<string, ((ev: { data?: unknown }) => void)[]>();
  private pendingMessages: { data?: unknown }[] = [];
  accepted = false;

  accept(): void {
    this.accepted = true;
  }

  send(data: Uint8Array): void {
    const copy = new Uint8Array(data);
    const peer = this.peer;
    queueMicrotask(() => peer.emit("message", { data: copy.buffer }));
  }

  close(): void {
    const peer = this.peer;
    queueMicrotask(() => {
      this.emit("close", {});
      peer.emit("close", {});
    });
  }

  addEventListener(type: string, cb: (ev: { data?: unknown }) => void): void {
    const list = this.listeners.get(type) ?? [];
    list.push(cb);
    this.listeners.set(type, list);
    // The DO's session sends its CHALLENGE as soon as the route
    // runs, before the client side has registered: messages with
    // no listener yet are buffered, not dropped.
    if (type === "message" && this.pendingMessages.length > 0) {
      const pending = this.pendingMessages.splice(0);
      queueMicrotask(() => pending.forEach((ev) => this.emit("message", ev)));
    }
  }

  emit(type: string, ev: { data?: unknown }): void {
    const list = this.listeners.get(type) ?? [];
    if (type === "message" && list.length === 0) {
      this.pendingMessages.push(ev);
      return;
    }
    for (const cb of list) cb(ev);
  }
}

class FakeWebSocketPair {
  static instances: FakeWebSocketPair[] = [];
  0 = new FakeSocket();
  1 = new FakeSocket();
  constructor() {
    this[0].peer = this[1];
    this[1].peer = this[0];
    FakeWebSocketPair.instances.push(this);
  }
}

test("the WebSocket leg runs a full session through the real DO route", async () => {
  (globalThis as Record<string, unknown>).WebSocketPair = FakeWebSocketPair;
  // Node's Response refuses status 101 (the range is 200-599);
  // workerd's accepts it with a webSocket. Stub the constructor so
  // the route's 101 is observable here.
  const RealResponse = globalThis.Response;
  class TestResponse extends RealResponse {
    constructor(body?: BodyInit | null, init?: ResponseInit & { webSocket?: unknown }) {
      super(body, init?.status === 101 ? { ...init, status: 200 } : init);
      if (init?.status === 101) Object.defineProperty(this, "status", { value: 101 });
    }
  }
  (globalThis as Record<string, unknown>).Response = TestResponse;
  try {
    await runWsSession();
  } finally {
    (globalThis as Record<string, unknown>).Response = RealResponse;
  }
});

async function runWsSession(): Promise<void> {
  const key = await importIdKey(new Uint8Array(32).fill(61));
  const author = await importIdKey(new Uint8Array(32).fill(65));
  const { doA, sql } = makeDO();
  const events = await chainOf(author, 3);

  // The client's own store starts with the events; the DO starts empty.
  const clientSql = (() => {
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
  })();
  for (const statement of BRIDGE_SCHEMA) clientSql.exec(statement);
  const clientStore = new BridgeStore(clientSql as never);
  clientStore.ingestBatch(events, Date.now());

  const headers = new Headers();
  headers.set(HDR_BINDING, BINDING);
  headers.set(HDR_JURISDICTION, "us");
  headers.set(HDR_ROUTED, "restricted");
  headers.set(HDR_IROH, key.publicHex);
  headers.set("upgrade", "websocket");
  const res = await doA.fetch(new Request("https://do/v1/bridge/ws", { headers }));
  assert.equal(res.status, 101);
  const pair = FakeWebSocketPair.instances[FakeWebSocketPair.instances.length - 1];
  const clientSocket = pair[0];
  const transport: FrameTransport = {
    send: (d) => clientSocket.send(d),
    onMessage: (cb) =>
      clientSocket.addEventListener("message", (ev) => {
        if (ev.data instanceof ArrayBuffer) cb(new Uint8Array(ev.data));
      }),
    onClose: (cb) => clientSocket.addEventListener("close", () => cb()),
    close: () => clientSocket.close(),
  };
  const client = new BridgeClient(transport, {
    store: clientStore,
    key,
    bindingId: BINDING,
  });
  const result = await client.sync();
  assert.ok(result.token);
  // The DO's store now holds the whole chain, byte for byte.
  const doStore = new BridgeStore(sql as never);
  assert.equal(doStore.chainOf(author.publicHex).next_seq, 3);
  assert.deepEqual(doStore.canonical(), clientStore.canonical());
}
