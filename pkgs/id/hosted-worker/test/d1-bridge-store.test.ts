// The D1 bridge store (src/d1-bridge-store.ts), #15538 slice 3.
//
// Oracle: exact event bytes and the chain head are returned only by
// the D1 binding the event was appended through. Each tenant D1
// binding is modelled here as its own in-memory SQLite database
// behind the asynchronous D1 statement API; the Wrangler-local D1
// proof of the same isolation is pkgs/id/worker-poc/d1-journal-poc
// (smoke.mjs, "cross-binding isolation").
import { test } from "node:test";
import assert from "node:assert/strict";
import { DatabaseSync } from "node:sqlite";
import { D1BridgeStore, D1_BRIDGE_SCHEMA, type D1DatabaseLike, type D1PreparedStatementLike } from "../src/d1-bridge-store.ts";
import { importIdKey, type IdKey } from "../src/ed25519.ts";
import { signEvent, type SignedEvent } from "../src/event.ts";

function sqliteValue(value: unknown): unknown {
  // D1 takes BLOBs as ArrayBuffer; node:sqlite binds typed arrays.
  return value instanceof ArrayBuffer ? new Uint8Array(value) : value;
}

/** One tenant's D1 binding: a private SQLite database exposed only
 * through D1's asynchronous prepare/bind/first/all/run/batch API. */
function fakeD1(): D1DatabaseLike {
  const db = new DatabaseSync(":memory:");
  const statement = (query: string, params: unknown[] = []): D1PreparedStatementLike & { exec(): void } => ({
    bind: (...values: unknown[]) => statement(query, values),
    async first<T>() {
      const row = db.prepare(query).get(...(params.map(sqliteValue) as never[]));
      return (row ?? null) as T | null;
    },
    async all<T>() {
      return { results: db.prepare(query).all(...(params.map(sqliteValue) as never[])) as T[] };
    },
    async run() {
      return db.prepare(query).run(...(params.map(sqliteValue) as never[]));
    },
    exec() {
      db.prepare(query).run(...(params.map(sqliteValue) as never[]));
    },
  });
  return {
    prepare: (query: string) => statement(query),
    async batch(statements) {
      // D1 runs a batch as one transaction: all or nothing.
      db.exec("BEGIN");
      try {
        for (const s of statements) (s as ReturnType<typeof statement>).exec();
        db.exec("COMMIT");
      } catch (error) {
        db.exec("ROLLBACK");
        throw error;
      }
      return statements.map(() => ({ success: true }));
    },
  };
}

async function chainOf(key: IdKey, count: number): Promise<SignedEvent[]> {
  const events: SignedEvent[] = [];
  for (let seq = 0; seq < count; seq++) {
    events.push(
      await signEvent(key, {
        seq,
        backlink: seq === 0 ? null : events[seq - 1].idBytes,
        kind: "message",
        body: new TextEncoder().encode(`d1 body ${seq}`),
      }),
    );
  }
  return events;
}

test("the slice schema is the bridge_events and bridge_chains DDL of BRIDGE_SCHEMA", () => {
  assert.equal(D1_BRIDGE_SCHEMA.length, 3);
  assert.match(D1_BRIDGE_SCHEMA[0], /CREATE TABLE IF NOT EXISTS bridge_events \(/);
  assert.match(D1_BRIDGE_SCHEMA[1], /bridge_events_by_id ON bridge_events\(id\)/);
  assert.match(D1_BRIDGE_SCHEMA[2], /CREATE TABLE IF NOT EXISTS bridge_chains \(/);
});

test("an appended signed event and its chain head round-trip through the tenant's binding", async () => {
  const key = await importIdKey(new Uint8Array(32).fill(51));
  const store = new D1BridgeStore(fakeD1());
  const [e0, e1] = await chainOf(key, 2);

  assert.deepEqual(await store.append(e0.bytes), { kind: "accepted", id: e0.id, author: key.publicHex, seq: 0 });
  assert.deepEqual(await store.append(e1.bytes), { kind: "accepted", id: e1.id, author: key.publicHex, seq: 1 });

  const at1 = await store.storedAt(key.publicHex, 1);
  assert.ok(at1);
  assert.equal(at1.id, e1.id);
  assert.deepEqual(at1.bytes, e1.bytes, "stored bytes are the exact signed bytes");
  assert.deepEqual(await store.chainOf(key.publicHex), { author: key.publicHex, next_seq: 2, head: e1.id });
  assert.deepEqual((await store.eventById(e0.id))?.bytes, e0.bytes);
});

test("a second tenant's D1 binding cannot read the first tenant's event or chain head", async () => {
  const key = await importIdKey(new Uint8Array(32).fill(52));
  const tenantA = new D1BridgeStore(fakeD1());
  const tenantB = new D1BridgeStore(fakeD1());
  const [e0] = await chainOf(key, 1);

  assert.equal((await tenantA.append(e0.bytes)).kind, "accepted");

  assert.equal(await tenantB.storedAt(key.publicHex, 0), null);
  assert.equal(await tenantB.eventById(e0.id), null);
  assert.deepEqual(await tenantB.chainOf(key.publicHex), { author: key.publicHex, next_seq: 0, head: null });
  // And tenant A still holds it: the isolation is not a lost write.
  assert.deepEqual((await tenantA.storedAt(key.publicHex, 0))?.bytes, e0.bytes);
});

test("re-delivery is a duplicate and leaves the head where it was", async () => {
  const key = await importIdKey(new Uint8Array(32).fill(53));
  const store = new D1BridgeStore(fakeD1());
  const [e0] = await chainOf(key, 1);
  await store.append(e0.bytes);
  assert.deepEqual(await store.append(e0.bytes), { kind: "duplicate", id: e0.id });
  assert.deepEqual(await store.chainOf(key.publicHex), { author: key.publicHex, next_seq: 1, head: e0.id });
});

test("a gap or a rival at a stored position is refused, never stored or resolved", async () => {
  const key = await importIdKey(new Uint8Array(32).fill(54));
  const store = new D1BridgeStore(fakeD1());
  const [e0, e1] = await chainOf(key, 2);

  const gap = await store.append(e1.bytes);
  assert.equal(gap.kind, "refused");
  assert.equal(gap.kind === "refused" && gap.reason, "not_next");
  assert.equal(await store.storedAt(key.publicHex, 1), null);

  await store.append(e0.bytes);
  const rival = await signEvent(key, { seq: 0, kind: "message", body: new TextEncoder().encode("rival") });
  const outcome = await store.append(rival.bytes);
  assert.equal(outcome.kind === "refused" && outcome.reason, "not_next");
  assert.deepEqual(await store.chainOf(key.publicHex), { author: key.publicHex, next_seq: 1, head: e0.id });
});

test("bytes whose signature does not verify are refused as malformed", async () => {
  const key = await importIdKey(new Uint8Array(32).fill(55));
  const store = new D1BridgeStore(fakeD1());
  const [e0] = await chainOf(key, 1);
  const tampered = new Uint8Array(e0.bytes);
  tampered[tampered.length - 1] ^= 0x01;
  const outcome = await store.append(tampered);
  assert.equal(outcome.kind, "refused");
  assert.equal(outcome.kind === "refused" && outcome.reason, "malformed");
  assert.deepEqual(await store.chainOf(key.publicHex), { author: key.publicHex, next_seq: 0, head: null });
});
