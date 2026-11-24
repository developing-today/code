// Bridge session acceptance (src/bridge-session.ts): the #3818
// acceptance list driven over an in-memory transport pair — the
// same protocol code the Durable Object serves over WebSocket.
//
// Oracle: the #2276 proof of concept's session scenarios
// (tests/bridge_poc.rs), restated against the production store:
// convergence is canonical-shape equality, forks are proof
// equality, withholding is the evidence record.
import { test } from "node:test";
import assert from "node:assert/strict";
import { DatabaseSync } from "node:sqlite";
import { BRIDGE_SCHEMA, BridgeStore } from "../src/bridge-store.ts";
import { importIdKey, type IdKey } from "../src/ed25519.ts";
import { parseEvent, signEvent, type SignedEvent } from "../src/event.ts";
import {
  BridgeClient,
  BridgeSessionServer,
  decodeBatch,
  DEFAULT_BRIDGE_LIMITS,
  encodeBatch,
  syncWithRetry,
  TAG,
  type FrameTransport,
} from "../src/bridge-session.ts";
import { DEFAULT_RANGE_PARAMS } from "../src/range.ts";

const BINDING = "aa".repeat(16);
const NOW = 2_000_000;

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

function makeStore(): BridgeStore {
  const sql = fakeSql();
  for (const statement of BRIDGE_SCHEMA) sql.exec(statement);
  return new BridgeStore(sql as never);
}

class End implements FrameTransport {
  peer!: End;
  private msgCbs: ((d: Uint8Array) => void | Promise<void>)[] = [];
  private closeCbs: (() => void)[] = [];
  private pending: Uint8Array[] = [];
  private closedFlag = false;

  send(data: Uint8Array): void {
    const peer = this.peer;
    queueMicrotask(() => peer.receive(data));
  }

  private receive(data: Uint8Array): void {
    // Messages already sent are delivered even if close() raced
    // them, as a real socket flushes before its close completes.
    if (this.msgCbs.length === 0) {
      this.pending.push(data);
      return;
    }
    for (const cb of this.msgCbs) void cb(data);
  }

  onMessage(cb: (d: Uint8Array) => void | Promise<void>): void {
    this.msgCbs.push(cb);
    if (this.pending.length > 0) {
      const pending = this.pending.splice(0);
      queueMicrotask(() => pending.forEach((d) => this.receive(d)));
    }
  }

  onClose(cb: () => void): void {
    this.closeCbs.push(cb);
    if (this.closedFlag) queueMicrotask(cb);
  }

  close(): void {
    if (this.closedFlag) return;
    this.closedFlag = true;
    const peer = this.peer;
    peer.closedFlag = true;
    queueMicrotask(() => {
      for (const cb of this.closeCbs) cb();
      for (const cb of peer.closeCbs) cb();
    });
  }
}

function transportPair(): [End, End] {
  const a = new End();
  const b = new End();
  a.peer = b;
  b.peer = a;
  return [a, b];
}

async function chainOf(key: IdKey, count: number, store?: BridgeStore): Promise<SignedEvent[]> {
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
  if (store) store.ingestBatch(events, NOW);
  return events;
}

function serve(
  store: BridgeStore,
  boundKey: IdKey,
  transport: FrameTransport,
  params = DEFAULT_RANGE_PARAMS,
): BridgeSessionServer {
  return new BridgeSessionServer({
    store,
    bindingId: BINDING,
    irohHex: boundKey.publicHex,
    transport,
    params,
    now: () => NOW,
  });
}

async function runSession(
  clientStore: BridgeStore,
  serverStore: BridgeStore,
  clientKey: IdKey,
  boundKey: IdKey,
  clientTransport?: FrameTransport,
): Promise<Awaited<ReturnType<BridgeClient["sync"]>>> {
  const [clientEnd, serverEnd] = transportPair();
  serve(serverStore, boundKey, serverEnd);
  const client = new BridgeClient(clientTransport ?? clientEnd, {
    store: clientStore,
    key: clientKey,
    bindingId: BINDING,
    now: () => NOW,
  });
  return client.sync();
}

test("convergence in both directions in one session", async () => {
  const bound = await importIdKey(new Uint8Array(32).fill(51));
  const alice = await importIdKey(new Uint8Array(32).fill(41));
  const bob = await importIdKey(new Uint8Array(32).fill(42));
  const clientStore = makeStore();
  const serverStore = makeStore();
  const aliceEvents = await chainOf(alice, 3, clientStore);
  const bobEvents = await chainOf(bob, 2, serverStore);
  // One event exists on both sides already (a prior partial sync).
  serverStore.ingestBatch([aliceEvents[0]], NOW);

  const result = await runSession(clientStore, serverStore, bound, bound);
  assert.deepEqual(clientStore.canonical(), serverStore.canonical());
  assert.equal(clientStore.chainOf(alice.publicHex).next_seq, 3);
  assert.equal(clientStore.chainOf(bob.publicHex).next_seq, 2);
  assert.ok(result.token, "a resume token is issued");
  assert.deepEqual(result.withholding, []);
  // Every accepted object kept its original bytes and signature.
  for (const e of [...aliceEvents, ...bobEvents]) {
    assert.deepEqual(serverStore.heldEvent(e.id)?.bytes, e.bytes);
    assert.deepEqual(clientStore.heldEvent(e.id)?.bytes, e.bytes);
  }
});

test("an equal-height fork is detected by proof exchange, not heights", async () => {
  const bound = await importIdKey(new Uint8Array(32).fill(51));
  const alice = await importIdKey(new Uint8Array(32).fill(43));
  const clientStore = makeStore();
  const serverStore = makeStore();
  const [e0] = await chainOf(alice, 1);
  const a1 = await signEvent(alice, {
    seq: 1,
    backlink: e0.idBytes,
    kind: "message",
    body: new TextEncoder().encode("a"),
  });
  const b1 = await signEvent(alice, {
    seq: 1,
    backlink: e0.idBytes,
    kind: "message",
    body: new TextEncoder().encode("b"),
  });
  clientStore.ingestBatch([e0, a1], NOW);
  serverStore.ingestBatch([e0, b1], NOW);
  await runSession(clientStore, serverStore, bound, bound);
  assert.deepEqual(clientStore.canonical(), serverStore.canonical());
  const chain = clientStore.chainOf(alice.publicHex);
  assert.equal(chain.frozen, true);
  assert.equal(chain.next_seq, 1);
  const [first, second] = [a1.id, b1.id].sort();
  assert.deepEqual(clientStore.canonical().proofs, [`${alice.publicHex}:1:${first}:${second}`]);
});

test("withholding is detected from heads and the frontier", async () => {
  const bound = await importIdKey(new Uint8Array(32).fill(51));
  const alice = await importIdKey(new Uint8Array(32).fill(44));
  const mallory = await importIdKey(new Uint8Array(32).fill(45));
  const clientStore = makeStore();
  const serverStore = makeStore();
  await chainOf(alice, 2, serverStore);
  const malloryEvents = await chainOf(mallory, 4, serverStore);

  // A relay that strips Mallory's events from batches in flight.
  const [clientEnd, serverEnd] = transportPair();
  serve(serverStore, bound, serverEnd);
  const relay: FrameTransport = {
    send: (d) => clientEnd.send(d),
    onMessage: (cb) =>
      clientEnd.onMessage((d) => {
        if (d[0] === TAG.batch) {
          const events = decodeBatch(d.subarray(1), DEFAULT_BRIDGE_LIMITS).filter(
            (raw) => parseEvent(raw).authorHex !== mallory.publicHex,
          );
          const out = new Uint8Array(1 + encodeBatch(events).length);
          out[0] = TAG.batch;
          out.set(encodeBatch(events), 1);
          cb(out);
        } else {
          cb(d);
        }
      }),
    onClose: (cb) => clientEnd.onClose(cb),
    close: () => clientEnd.close(),
  };
  const client = new BridgeClient(relay, {
    store: clientStore,
    key: bound,
    bindingId: BINDING,
    now: () => NOW,
  });
  const result = await client.sync();
  assert.equal(clientStore.chainOf(alice.publicHex).next_seq, 2, "honest author converged");
  assert.equal(result.withholding.length, 1);
  const record = result.withholding[0];
  assert.equal(record.author, mallory.publicHex);
  assert.equal(record.announced_next_seq, 4);
  assert.equal(record.announced_head, malloryEvents[3].id);
  assert.equal(record.actual_next_seq, 0);
  assert.deepEqual(record.undelivered, [malloryEvents[3].id]);
});

test("replay into empty stores yields identical tables, both directions", async () => {
  const bound = await importIdKey(new Uint8Array(32).fill(51));
  const alice = await importIdKey(new Uint8Array(32).fill(46));
  const source = makeStore();
  const [e0] = await chainOf(alice, 1, source);
  const a1 = await signEvent(alice, {
    seq: 1,
    backlink: e0.idBytes,
    kind: "message",
    body: new TextEncoder().encode("a"),
  });
  const b1 = await signEvent(alice, {
    seq: 1,
    backlink: e0.idBytes,
    kind: "message",
    body: new TextEncoder().encode("b"),
  });
  source.ingestBatch([a1, b1], NOW);
  // A waiting event whose dependency never arrives.
  const waiting = await signEvent(alice, { seq: 0, kind: "message", after: [new Uint8Array(32).fill(99)] });
  const other = await importIdKey(new Uint8Array(32).fill(47));
  const waiting2 = await signEvent(other, { seq: 0, kind: "message", after: [waiting.idBytes] });
  source.ingestBatch([waiting2], NOW);

  const emptyA = makeStore();
  await runSession(emptyA, source, bound, bound);
  assert.deepEqual(emptyA.canonical(), source.canonical());
  const emptyB = makeStore();
  await runSession(emptyB, emptyA, bound, bound);
  assert.deepEqual(emptyB.canonical(), source.canonical());
});

test("a dropped session retries, resumes, and never duplicates", async () => {
  const bound = await importIdKey(new Uint8Array(32).fill(51));
  const alice = await importIdKey(new Uint8Array(32).fill(48));
  const clientStore = makeStore();
  const serverStore = makeStore();
  await chainOf(alice, 4, serverStore);

  let attempts = 0;
  const result = await syncWithRetry(
    () => {
      attempts += 1;
      const [clientEnd, serverEnd] = transportPair();
      serve(serverStore, bound, serverEnd);
      if (attempts === 1) {
        // Kill the link as the first batch crosses.
        const killer: FrameTransport = {
          send: (d) => clientEnd.send(d),
          onMessage: (cb) =>
            clientEnd.onMessage((d) => {
              cb(d);
              if (d[0] === TAG.batch) clientEnd.close();
            }),
          onClose: (cb) => clientEnd.onClose(cb),
          close: () => clientEnd.close(),
        };
        return killer;
      }
      return clientEnd;
    },
    { store: clientStore, key: bound, bindingId: BINDING, now: () => NOW },
    { attempts: 3, baseDelayMs: 1, sleep: async () => {} },
  );
  assert.equal(attempts, 2);
  assert.equal(clientStore.chainOf(alice.publicHex).next_seq, 4);
  assert.deepEqual(clientStore.canonical(), serverStore.canonical());
  assert.ok(result.token);
  // Re-delivery of the whole journal is all duplicates.
  const again = clientStore.ingestBatch(
    serverStore.heldEvents().map((h) => parseEvent(h.bytes)),
    NOW,
  );
  assert.ok(again.every((o) => o.kind === "duplicate"));
});

test("a resume token carries the next session; a stale one falls back to HELLO", async () => {
  const bound = await importIdKey(new Uint8Array(32).fill(51));
  const alice = await importIdKey(new Uint8Array(32).fill(49));
  const clientStore = makeStore();
  const serverStore = makeStore();
  await chainOf(alice, 2, serverStore);
  const first = await runSession(clientStore, serverStore, bound, bound);
  assert.ok(first.token);
  // New events land on the server; the resumed session picks them up.
  const more = await chainOf(alice, 4, serverStore);
  void more;
  const [clientEnd, serverEnd] = transportPair();
  serve(serverStore, bound, serverEnd);
  const resumed = await new BridgeClient(clientEnd, {
    store: clientStore,
    key: bound,
    bindingId: BINDING,
    resumeToken: first.token,
    now: () => NOW,
  }).sync();
  assert.equal(clientStore.chainOf(alice.publicHex).next_seq, 4);
  assert.ok(resumed.token);
  // A bogus token: the retry wrapper clears it and syncs via HELLO.
  const carol = await importIdKey(new Uint8Array(32).fill(50));
  await chainOf(carol, 1, serverStore);
  const recovered = await syncWithRetry(
    () => {
      const [c, s] = transportPair();
      serve(serverStore, bound, s);
      return c;
    },
    { store: clientStore, key: bound, bindingId: BINDING, resumeToken: "ff".repeat(32), now: () => NOW },
    { attempts: 3, baseDelayMs: 1, sleep: async () => {} },
  );
  assert.equal(clientStore.chainOf(carol.publicHex).next_seq, 1);
  assert.ok(recovered.token);
});

test("the wrong ID key is refused at AUTH", async () => {
  const bound = await importIdKey(new Uint8Array(32).fill(51));
  const impostor = await importIdKey(new Uint8Array(32).fill(52));
  const clientStore = makeStore();
  const serverStore = makeStore();
  await assert.rejects(() => runSession(clientStore, serverStore, impostor, bound), /auth_failed/);
});

test("a blown range budget ends the session with a named error", async () => {
  const bound = await importIdKey(new Uint8Array(32).fill(51));
  const alice = await importIdKey(new Uint8Array(32).fill(53));
  const clientStore = makeStore();
  const serverStore = makeStore();
  await chainOf(alice, 30, serverStore);
  const [clientEnd, serverEnd] = transportPair();
  serve(serverStore, bound, serverEnd, { ...DEFAULT_RANGE_PARAMS, sessionBudget: 300 });
  const client = new BridgeClient(clientEnd, {
    store: clientStore,
    key: bound,
    bindingId: BINDING,
    now: () => NOW,
  });
  await assert.rejects(() => client.sync(), /budget_exceeded/);
});

test("batch bounds are enforced by the codec", () => {
  const tooMany = Array.from({ length: DEFAULT_BRIDGE_LIMITS.maxBatchEvents + 1 }, () => new Uint8Array([1]));
  assert.throws(() => decodeBatch(encodeBatch(tooMany), DEFAULT_BRIDGE_LIMITS), /over the .* bound/);
  const one = encodeBatch([new Uint8Array([7, 8, 9])]);
  assert.deepEqual(decodeBatch(one, DEFAULT_BRIDGE_LIMITS), [new Uint8Array([7, 8, 9])]);
  const trailed = new Uint8Array([...one, 0]);
  assert.throws(() => decodeBatch(trailed, DEFAULT_BRIDGE_LIMITS), /trailing/);
});
