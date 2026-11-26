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
import type { CborValue } from "../src/canonical-cbor.ts";
import {
  BridgeClient,
  decodeBatch,
  DEFAULT_BRIDGE_LIMITS,
  encodeBatch,
  type FrameTransport,
} from "../src/bridge-session.ts";

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
  return { doA: new UserDO(state as never, { BLOBS_US: bucket, ...env } as never), bucket, sql };
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
  private attachment: string | null = null;
  private isClosed = false;
  private closeEmitted = false;
  accepted = false;
  /** Set once the DO hands this socket to the hibernation API: the
   * runtime then delivers its events to whichever instance of the
   * object is current, one at a time, never to listeners. */
  host: (() => UserDO) | null = null;
  private chain: Promise<void> = Promise.resolve();

  accept(): void {
    this.accepted = true;
  }

  serializeAttachment(value: unknown): void {
    // Structured-clone semantics: the attachment is a copy.
    this.attachment = JSON.stringify(value);
  }

  deserializeAttachment(): unknown {
    return this.attachment === null ? null : JSON.parse(this.attachment);
  }

  send(data: Uint8Array): void {
    if (this.isClosed) return;
    const copy = new Uint8Array(data);
    const peer = this.peer;
    queueMicrotask(() => peer.emit("message", { data: copy.buffer }));
  }

  close(): void {
    if (this.isClosed) return;
    this.isClosed = true;
    const peer = this.peer;
    queueMicrotask(() => {
      if (!this.host) this.emit("close", {});
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
    if (type === "close") {
      if (this.closeEmitted) return;
      this.closeEmitted = true;
    }
    const host = this.host;
    if (host) {
      this.chain = this.chain.then(async () => {
        if (type === "message") await host().webSocketMessage(this, ev.data as ArrayBuffer);
        else if (type === "close") await host().webSocketClose(this, 1000, "");
      });
      return;
    }
    const list = this.listeners.get(type) ?? [];
    if (type === "message" && list.length === 0) {
      this.pendingMessages.push(ev);
      return;
    }
    for (const cb of list) cb(ev);
  }

  settled(): Promise<void> {
    return this.chain;
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

/** A DO whose state offers the hibernation API. With `evict`, every
 * frame the runtime delivers first replaces the object with a fresh
 * instance over the same storage, as an eviction would (#15484). */
function makeHibernatingDO(opts: { evict?: boolean } = {}) {
  const bucket = fakeBucket();
  const sql = fakeSql();
  const hibernated: FakeSocket[] = [];
  let evictions = 0;
  const state = {
    id: { name: BINDING },
    storage: { sql },
    blockConcurrencyWhile: async <T>(fn: () => Promise<T>): Promise<T> => fn(),
    acceptWebSocket(ws: FakeSocket) {
      hibernated.push(ws);
      ws.host = () => {
        if (opts.evict) {
          current = new UserDO(state as never, { BLOBS_US: bucket } as never);
          evictions++;
        }
        return current;
      };
    },
  };
  let current = new UserDO(state as never, { BLOBS_US: bucket } as never);
  return { doA: () => current, sql, hibernated, evictions: () => evictions };
}

/** Run `fn` with workerd's WebSocketPair and a Response that takes
 * status 101 (Node's refuses it; workerd's accepts it with a
 * webSocket). */
async function withWorkerdGlobals(fn: () => Promise<void>): Promise<void> {
  (globalThis as Record<string, unknown>).WebSocketPair = FakeWebSocketPair;
  const RealResponse = globalThis.Response;
  class TestResponse extends RealResponse {
    constructor(body?: BodyInit | null, init?: ResponseInit & { webSocket?: unknown }) {
      super(body, init?.status === 101 ? { ...init, status: 200 } : init);
      if (init?.status === 101) Object.defineProperty(this, "status", { value: 101 });
    }
  }
  (globalThis as Record<string, unknown>).Response = TestResponse;
  try {
    await fn();
  } finally {
    (globalThis as Record<string, unknown>).Response = RealResponse;
  }
}

function memorySql() {
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

/** Open the bridge socket on the DO; returns the client end. */
async function openWs(doA: UserDO, irohHex: string): Promise<{ client: FakeSocket; server: FakeSocket }> {
  const headers = new Headers();
  headers.set(HDR_BINDING, BINDING);
  headers.set(HDR_JURISDICTION, "us");
  headers.set(HDR_ROUTED, "restricted");
  headers.set(HDR_IROH, irohHex);
  headers.set("upgrade", "websocket");
  const res = await doA.fetch(new Request("https://do/v1/bridge/ws", { headers }));
  assert.equal(res.status, 101);
  const pair = FakeWebSocketPair.instances[FakeWebSocketPair.instances.length - 1];
  return { client: pair[0], server: pair[1] };
}

function clientTransport(clientSocket: FakeSocket): FrameTransport {
  return {
    send: (d) => clientSocket.send(d),
    onMessage: (cb) =>
      clientSocket.addEventListener("message", (ev) => {
        if (ev.data instanceof ArrayBuffer) cb(new Uint8Array(ev.data));
      }),
    onClose: (cb) => clientSocket.addEventListener("close", () => cb()),
    close: () => clientSocket.close(),
  };
}

function wsSessionRows(sql: ReturnType<typeof fakeSql>): number {
  return Number(sql.exec("SELECT COUNT(*) AS n FROM bridge_ws_sessions").toArray()[0].n);
}

test("the WebSocket leg runs a full session through the real DO route", async () => {
  await withWorkerdGlobals(() => runWsSession({ evict: false }));
});

test("a hibernating bridge session survives an eviction before every frame (#15484)", async () => {
  await withWorkerdGlobals(() => runWsSession({ evict: true }));
});

async function runWsSession(opts: { evict: boolean }): Promise<void> {
  const key = await importIdKey(new Uint8Array(32).fill(61));
  const author = await importIdKey(new Uint8Array(32).fill(65));
  const host = makeHibernatingDO({ evict: opts.evict });
  const { sql } = host;
  const events = await chainOf(author, 3);

  // The client's own store starts with the events; the DO starts empty.
  const clientSql = memorySql();
  for (const statement of BRIDGE_SCHEMA) clientSql.exec(statement);
  const clientStore = new BridgeStore(clientSql as never);
  clientStore.ingestBatch(events, Date.now());

  const { client: clientSocket, server } = await openWs(host.doA(), key.publicHex);
  // Hibernating, never accepted in place: the object may be evicted
  // while the socket stays open.
  assert.equal(server.accepted, false);
  assert.deepEqual(host.hibernated, [server]);
  assert.equal(wsSessionRows(sql), 1);
  const client = new BridgeClient(clientTransport(clientSocket), {
    store: clientStore,
    key,
    bindingId: BINDING,
  });
  const result = await client.sync();
  assert.ok(result.token);
  if (opts.evict) {
    // Every frame woke a fresh instance with no session in memory.
    assert.ok(host.evictions() >= 5, `only ${host.evictions()} evictions`);
  }
  // The DO's store now holds the whole chain, byte for byte.
  const doStore = new BridgeStore(sql as never);
  assert.equal(doStore.chainOf(author.publicHex).next_seq, 3);
  assert.deepEqual(doStore.canonical(), clientStore.canonical());

  // Closing the socket removes the session's row.
  clientSocket.close();
  await new Promise((r) => setTimeout(r, 0));
  await server.settled();
  assert.equal(wsSessionRows(sql), 0);

  // A reconnecting client resumes against the same DO state with its
  // token, and has nothing left to send.
  const next = await openWs(host.doA(), key.publicHex);
  const resumed = await new BridgeClient(clientTransport(next.client), {
    store: clientStore,
    key,
    bindingId: BINDING,
    resumeToken: result.token,
  }).sync();
  assert.ok(resumed.token);
  assert.deepEqual(
    resumed.serverHeads.map((h) => [h.author, h.next_seq]),
    [[author.publicHex, 3]],
  );
  assert.equal(new BridgeStore(sql as never).chainOf(author.publicHex).next_seq, 3);
}

test("a socket whose session state is gone is told session_lost and closed (#15484)", async () => {
  await withWorkerdGlobals(async () => {
    const key = await importIdKey(new Uint8Array(32).fill(61));
    const host = makeHibernatingDO({ evict: true });
    const { client, server } = await openWs(host.doA(), key.publicHex);
    const received: Uint8Array[] = [];
    let closed = false;
    client.addEventListener("message", (ev) => received.push(new Uint8Array(ev.data as ArrayBuffer)));
    client.addEventListener("close", () => {
      closed = true;
    });
    await new Promise((r) => setTimeout(r, 0));
    // The CHALLENGE arrived; then the row is lost while hibernating.
    assert.equal(received.length, 1);
    host.sql.exec("DELETE FROM bridge_ws_sessions");
    client.send(new Uint8Array([0x02, 0x7b, 0x7d]));
    await new Promise((r) => setTimeout(r, 0));
    await server.settled();
    await new Promise((r) => setTimeout(r, 0));
    assert.equal(received.length, 2);
    assert.equal(received[1][0], 0x1f);
    const body = JSON.parse(new TextDecoder().decode(received[1].subarray(1))) as { code: string };
    assert.equal(body.code, "session_lost");
    assert.ok(closed);
  });
});

test("a damaged session row fails closed the same way (#15484)", async () => {
  await withWorkerdGlobals(async () => {
    const key = await importIdKey(new Uint8Array(32).fill(61));
    const host = makeHibernatingDO({ evict: true });
    const { client, server } = await openWs(host.doA(), key.publicHex);
    const received: Uint8Array[] = [];
    client.addEventListener("message", (ev) => received.push(new Uint8Array(ev.data as ArrayBuffer)));
    await new Promise((r) => setTimeout(r, 0));
    host.sql.exec("UPDATE bridge_ws_sessions SET state = ?", JSON.stringify({ v: 1, authed: "yes" }));
    client.send(new Uint8Array([0x02, 0x7b, 0x7d]));
    await new Promise((r) => setTimeout(r, 0));
    await server.settled();
    await new Promise((r) => setTimeout(r, 0));
    const last = received[received.length - 1];
    assert.equal(last[0], 0x1f);
    assert.equal((JSON.parse(new TextDecoder().decode(last.subarray(1))) as { code: string }).code, "session_lost");
    assert.equal(wsSessionRows(host.sql), 0);
  });
});

test("without the hibernation API the WebSocket leg fails closed (#15484)", async () => {
  await withWorkerdGlobals(async () => {
    const key = await importIdKey(new Uint8Array(32).fill(61));
    const { doA } = makeDO();
    const headers = new Headers();
    headers.set(HDR_BINDING, BINDING);
    headers.set(HDR_JURISDICTION, "us");
    headers.set(HDR_ROUTED, "restricted");
    headers.set(HDR_IROH, key.publicHex);
    headers.set("upgrade", "websocket");
    const res = await doA.fetch(new Request("https://do/v1/bridge/ws", { headers }));
    assert.equal(res.status, 500);
    assert.equal(((await res.json()) as { error: string }).error, "websocket_hibernation_unavailable");
  });
});

// A checkpoint event padded to an exact byte size: the segment
// map tolerates extra entries, and the event tree caps each byte
// string at 512 bytes, so the pad is an array of 512-byte strings
// whose total the sizing loop tunes against the signed size.
async function sizedCheckpoint(
  key: IdKey,
  seq: number,
  prevEvent: SignedEvent | null,
  target: number,
): Promise<SignedEvent> {
  const T = (v: string): CborValue => ({ kind: "text", value: v });
  const B = (v: Uint8Array): CborValue => ({ kind: "bytes", value: v });
  const U = (v: bigint): CborValue => ({ kind: "uint", value: v });
  let padPayload = target - 400;
  for (let attempt = 0; attempt < 8; attempt++) {
    const parts: Uint8Array[] = [];
    let left = padPayload;
    while (left > 512) {
      parts.push(new Uint8Array(512));
      left -= 512;
    }
    if (left > 0) parts.push(new Uint8Array(left));
    const segment: CborValue = {
      kind: "map",
      value: [
        [T("segment_seq"), U(BigInt(seq))],
        [T("prev_checkpoint"), prevEvent ? B(prevEvent.idBytes) : { kind: "null" }],
        [T("root"), B(new Uint8Array(32).fill(2))],
        [T("zzpad"), { kind: "array", value: parts.map((p): CborValue => B(p)) }],
      ],
    };
    const header: CborValue = {
      kind: "map",
      value: [
        [T("session_id"), B(new Uint8Array(32).fill(1))],
        [T("purpose"), T("collab")],
      ],
    };
    const extra: [string, CborValue][] = [
      ["checkpoint", { kind: "map", value: [[T("header"), header], [T("segment"), segment]] }],
    ];
    const candidate = await signEvent(key, {
      seq,
      backlink: prevEvent ? prevEvent.idBytes : null,
      kind: "session.checkpoint",
      extra,
    });
    if (candidate.bytes.length === target) return candidate;
    padPayload += target - candidate.bytes.length;
  }
  throw new Error("checkpoint pad did not converge");
}

test("pull budgets the encoded batch size, not the payload sum", async () => {
  const key = await importIdKey(new Uint8Array(32).fill(61));
  const author = await importIdKey(new Uint8Array(32).fill(67));
  const { doA } = makeDO();
  // 64 checkpoint events of exactly 16,384 bytes: the payload sum
  // (64 x 16384) is exactly the 1 MiB bound, so payload accounting
  // answers one page whose encoded size (4 + 64 x 16388) crosses
  // the bound every conforming decoder enforces (#14644). Framed
  // accounting pages at 63 + 1.
  const events: SignedEvent[] = [];
  for (let seq = 0; seq < 64; seq++) {
    events.push(await sizedCheckpoint(author, seq, seq === 0 ? null : events[seq - 1], 16384));
  }
  assert.ok(events.every((e) => e.bytes.length === 16384));
  // Push in two batches that each fit the bound.
  for (const chunk of [events.slice(0, 32), events.slice(32)]) {
    const batch = encodeBatch(chunk.map((e) => e.bytes));
    const nonce = await challenge(doA, key.publicHex);
    const headers = await proofHeaders(key, nonce, "push", batch);
    const push = await doA.fetch(
      routed("/v1/bridge/push", key.publicHex, { method: "POST", headers, body: batch as BodyInit }),
    );
    assert.equal(push.status, 200);
  }
  const pulled: Uint8Array[] = [];
  const pageSizes: number[] = [];
  let nextSeq = 0;
  for (let page = 0; page < 4; page++) {
    const pullBody = new TextEncoder().encode(
      JSON.stringify({ positions: [{ author: author.publicHex, next_seq: nextSeq }] }),
    );
    const pullNonce = await challenge(doA, key.publicHex);
    const pullHeaders = await proofHeaders(key, pullNonce, "pull", pullBody);
    const pull = await doA.fetch(
      routed("/v1/bridge/pull", key.publicHex, {
        method: "POST",
        headers: pullHeaders,
        body: pullBody as BodyInit,
      }),
    );
    assert.equal(pull.status, 200);
    const raw = new Uint8Array(await pull.arrayBuffer());
    assert.ok(raw.length <= DEFAULT_BRIDGE_LIMITS.maxBatchBytes, "the page fits the encoded bound");
    const batchEvents = decodeBatch(raw, DEFAULT_BRIDGE_LIMITS);
    pageSizes.push(batchEvents.length);
    pulled.push(...batchEvents);
    nextSeq += batchEvents.length;
    if (pull.headers.get("x-bridge-more") === "0") break;
  }
  assert.deepEqual(pageSizes, [63, 1]);
  assert.deepEqual(
    pulled,
    events.map((e) => e.bytes),
  );
});
