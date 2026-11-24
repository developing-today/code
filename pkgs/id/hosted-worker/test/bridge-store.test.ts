// The bridge store (src/bridge-store.ts): native ingest semantics
// mirrored on SQLite.
//
// Oracle: crates/id-events' ingest decision procedure — each test
// names the native outcome it mirrors (Accepted / Duplicate / Fork
// / Waiting / Refused), and the canonical shape is the same record
// the #2276 proof of concept compares across stores.
import { test } from "node:test";
import assert from "node:assert/strict";
import { DatabaseSync } from "node:sqlite";
import { BRIDGE_SCHEMA, BridgeStore, type WaitingLimits } from "../src/bridge-store.ts";
import { importIdKey, type IdKey } from "../src/ed25519.ts";
import { signEvent, type SignedEvent } from "../src/event.ts";
import { encodeCanonical, type CborValue } from "../src/canonical-cbor.ts";

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

function makeStore(limits?: WaitingLimits): BridgeStore {
  const sql = fakeSql();
  for (const statement of BRIDGE_SCHEMA) sql.exec(statement);
  sql.exec("CREATE TABLE IF NOT EXISTS bridge_meta (key TEXT PRIMARY KEY, value TEXT NOT NULL)");
  return new BridgeStore(sql as never, limits);
}

const NOW = 1_000_000;

async function chainOf(key: IdKey, count: number, kind = "message"): Promise<SignedEvent[]> {
  const events: SignedEvent[] = [];
  for (let seq = 0; seq < count; seq++) {
    events.push(
      await signEvent(key, {
        seq,
        backlink: seq === 0 ? null : events[seq - 1].idBytes,
        kind,
        body: new TextEncoder().encode(`body ${seq}`),
      }),
    );
  }
  return events;
}

test("a chain is accepted in order; heads and canonical shape follow", async () => {
  const key = await importIdKey(new Uint8Array(32).fill(21));
  const store = makeStore();
  const events = await chainOf(key, 3);
  const outcomes = store.ingestBatch(events, NOW);
  assert.deepEqual(outcomes.map((o) => o.kind), ["accepted", "accepted", "accepted"]);
  const chain = store.chainOf(key.publicHex);
  assert.equal(chain.next_seq, 3);
  assert.equal(chain.head, events[2].id);
  assert.equal(chain.frozen, false);
  const shape = store.canonical();
  assert.deepEqual(shape.applied[key.publicHex], events.map((e) => e.id));
  assert.deepEqual(shape.waiting, []);
});

test("re-delivery is a duplicate, never a second copy", async () => {
  const key = await importIdKey(new Uint8Array(32).fill(22));
  const store = makeStore();
  const [e0] = await chainOf(key, 1);
  store.ingestBatch([e0], NOW);
  const before = store.canonical();
  const outcomes = store.ingestBatch([e0], NOW + 1);
  assert.equal(outcomes[0].kind, "duplicate");
  assert.deepEqual(store.canonical(), before);
  assert.equal(store.storedBytes(), e0.bytes.length);
});

test("out-of-order delivery waits, then releases in chain order", async () => {
  const key = await importIdKey(new Uint8Array(32).fill(23));
  const store = makeStore();
  const [e0, e1, e2] = await chainOf(key, 3);
  const first = store.ingestBatch([e2, e1], NOW);
  assert.deepEqual(first.map((o) => o.kind), ["waiting", "waiting"]);
  const second = store.ingestBatch([e0], NOW + 1);
  assert.equal(second[0].kind, "accepted");
  if (second[0].kind === "accepted") {
    assert.deepEqual(second[0].released, [e1.id, e2.id]);
  }
  // Replay identity: the shape equals the in-order store's shape.
  const inOrder = makeStore();
  inOrder.ingestBatch([e0, e1, e2], NOW);
  assert.deepEqual(store.canonical(), inOrder.canonical());
});

test("an after-dependency holds the dependent until the dep lands", async () => {
  const key = await importIdKey(new Uint8Array(32).fill(24));
  const other = await importIdKey(new Uint8Array(32).fill(25));
  const store = makeStore();
  const [dep] = await chainOf(other, 1);
  const dependent = await signEvent(key, { seq: 0, kind: "message", after: [dep.idBytes] });
  const first = store.ingestBatch([dependent], NOW);
  assert.equal(first[0].kind, "waiting");
  const second = store.ingestBatch([dep], NOW + 1);
  assert.equal(second[0].kind, "accepted");
  if (second[0].kind === "accepted") assert.deepEqual(second[0].released, [dependent.id]);
  assert.equal(store.chainOf(key.publicHex).next_seq, 1);
});

test("an equal-height fork freezes the chain at the fork point", async () => {
  const key = await importIdKey(new Uint8Array(32).fill(26));
  const store = makeStore();
  const [e0] = await chainOf(key, 1);
  const a1 = await signEvent(key, { seq: 1, backlink: e0.idBytes, kind: "message", body: new TextEncoder().encode("a") });
  const b1 = await signEvent(key, { seq: 1, backlink: e0.idBytes, kind: "message", body: new TextEncoder().encode("b") });
  store.ingestBatch([e0, a1], NOW);
  const forkOutcome = store.ingestBatch([b1], NOW + 1);
  assert.equal(forkOutcome[0].kind, "fork");
  const chain = store.chainOf(key.publicHex);
  assert.equal(chain.frozen, true);
  assert.equal(chain.next_seq, 1);
  assert.equal(chain.head, e0.id);
  // The proof is the two lowest IDs at the fork point.
  const [first, second] = [a1.id, b1.id].sort();
  assert.deepEqual(store.canonical().proofs, [`${key.publicHex}:1:${first}:${second}`]);
  // A successor on either branch is refused: the chain is frozen.
  const a2 = await signEvent(key, { seq: 2, backlink: a1.idBytes, kind: "message" });
  const refused = store.ingestBatch([a2], NOW + 2);
  assert.equal(refused[0].kind, "refused");
  if (refused[0].kind === "refused") assert.equal(refused[0].reason, "frozen");
  // A third rival at the fork point with a lower ID replaces the
  // proof (the two-lowest rule); with a higher ID it is refused.
  const c1 = await signEvent(key, { seq: 1, backlink: e0.idBytes, kind: "message", body: new TextEncoder().encode("c") });
  const third = store.ingestBatch([c1], NOW + 3);
  const expectedPair = [a1.id, b1.id, c1.id].sort().slice(0, 2);
  const pairChanged = expectedPair[0] !== first || expectedPair[1] !== second;
  if (pairChanged) {
    assert.equal(third[0].kind, "fork");
    assert.deepEqual(store.canonical().proofs, [`${key.publicHex}:1:${expectedPair[0]}:${expectedPair[1]}`]);
  } else {
    assert.equal(third[0].kind, "refused");
  }
});

test("a held rival forks with the newcomer (proof from the pool)", async () => {
  const key = await importIdKey(new Uint8Array(32).fill(27));
  const store = makeStore();
  const [e0] = await chainOf(key, 1);
  store.ingestBatch([e0], NOW);
  // a1 waits on a missing after-dep; b1 arrives at the same height.
  const missing = new Uint8Array(32).fill(99);
  const a1 = await signEvent(key, { seq: 1, backlink: e0.idBytes, kind: "message", after: [missing] });
  assert.equal(store.ingestBatch([a1], NOW)[0].kind, "waiting");
  const b1 = await signEvent(key, { seq: 1, backlink: e0.idBytes, kind: "message", body: new TextEncoder().encode("b") });
  const outcome = store.ingestBatch([b1], NOW + 1);
  assert.equal(outcome[0].kind, "fork");
  const [first, second] = [a1.id, b1.id].sort();
  assert.deepEqual(store.canonical().proofs, [`${key.publicHex}:1:${first}:${second}`]);
});

test("a fork proof header is present for dependencies and duplicates", async () => {
  const key = await importIdKey(new Uint8Array(32).fill(28));
  const store = makeStore();
  const [e0] = await chainOf(key, 1);
  const a1 = await signEvent(key, { seq: 1, backlink: e0.idBytes, kind: "message", body: new TextEncoder().encode("a") });
  const b1 = await signEvent(key, { seq: 1, backlink: e0.idBytes, kind: "message", body: new TextEncoder().encode("b") });
  store.ingestBatch([e0, a1, b1], NOW);
  // Re-delivering a proof header is a duplicate.
  assert.equal(store.ingestBatch([a1], NOW + 1)[0].kind, "duplicate");
  // An event depending (after) on a proof header does not wait for it.
  const other = await importIdKey(new Uint8Array(32).fill(29));
  const dependent = await signEvent(other, { seq: 0, kind: "message", after: [b1.idBytes] });
  assert.equal(store.ingestBatch([dependent], NOW + 2)[0].kind, "accepted");
});

test("waiting caps refuse with waiting_full", async () => {
  const store = makeStore({ perAuthor: 2, total: 100, maxAgeMs: 60_000 });
  const missing = new Uint8Array(32).fill(77);
  // Distinct events at seq 0 would fork; use distinct authors instead.
  const keys = await Promise.all([31, 32, 33].map((n) => importIdKey(new Uint8Array(32).fill(n))));
  const outcomes = [];
  for (const k of keys) {
    outcomes.push(store.ingestBatch([await signEvent(k, { seq: 0, kind: "message", after: [missing] })], NOW)[0]);
  }
  assert.deepEqual(outcomes.map((o) => o.kind), ["waiting", "waiting", "waiting"]);
  // Per-author cap: a second author is fine, the capped author is not.
  const capped = makeStore({ perAuthor: 1, total: 100, maxAgeMs: 60_000 });
  const k1 = await importIdKey(new Uint8Array(32).fill(34));
  const w1 = await signEvent(k1, { seq: 0, kind: "message", after: [missing] });
  const w2 = await signEvent(k1, { seq: 1, backlink: w1.idBytes, kind: "message", after: [missing] });
  assert.equal(capped.ingestBatch([w1], NOW)[0].kind, "waiting");
  const refused = capped.ingestBatch([w2], NOW)[0];
  assert.equal(refused.kind, "refused");
  if (refused.kind === "refused") assert.equal(refused.reason, "waiting_full");
});

test("waiting entries expire at the age cap", async () => {
  const key = await importIdKey(new Uint8Array(32).fill(35));
  const store = makeStore({ perAuthor: 10, total: 100, maxAgeMs: 1_000 });
  const missing = new Uint8Array(32).fill(78);
  const w = await signEvent(key, { seq: 0, kind: "message", after: [missing] });
  assert.equal(store.ingestBatch([w], NOW)[0].kind, "waiting");
  assert.deepEqual(store.canonical().waiting, [w.id]);
  // A later batch past the cap sweeps the expired row first.
  const [e0] = await chainOf(key, 1);
  store.ingestBatch([e0], NOW + 1_001);
  assert.deepEqual(store.canonical().waiting, []);
});

const T = (v: string): CborValue => ({ kind: "text", value: v });
const B = (v: Uint8Array): CborValue => ({ kind: "bytes", value: v });
const U = (v: bigint): CborValue => ({ kind: "uint", value: v });

function checkpointDraft(segmentSeq: bigint, prev: Uint8Array | null, headerFill = 1) {
  const header: CborValue = {
    kind: "map",
    value: [
      [T("session_id"), B(new Uint8Array(32).fill(headerFill))],
      [T("purpose"), T("collab")],
    ],
  };
  const segment: CborValue = {
    kind: "map",
    value: [
      [T("segment_seq"), U(segmentSeq)],
      [T("prev_checkpoint"), prev ? B(prev) : { kind: "null" }],
      [T("root"), B(new Uint8Array(32).fill(2))],
    ],
  };
  return [["checkpoint", { kind: "map", value: [[T("header"), header], [T("segment"), segment]] }]] as [string, CborValue][];
}

test("checkpoint predecessors: accept the next segment, refuse a skip, wait on a gap", async () => {
  const key = await importIdKey(new Uint8Array(32).fill(36));
  const store = makeStore();
  const cp0 = await signEvent(key, { seq: 0, kind: "session.checkpoint", extra: checkpointDraft(0n, null) });
  assert.equal(store.ingestBatch([cp0], NOW)[0].kind, "accepted");
  const cp1 = await signEvent(key, { seq: 1, backlink: cp0.idBytes, kind: "session.checkpoint", extra: checkpointDraft(1n, cp0.idBytes) });
  assert.equal(store.ingestBatch([cp1], NOW)[0].kind, "accepted");
  // Segment 5 naming cp1 as predecessor: not the previous segment.
  const cpSkip = await signEvent(key, { seq: 2, backlink: cp1.idBytes, kind: "session.checkpoint", extra: checkpointDraft(5n, cp1.idBytes) });
  const refused = store.ingestBatch([cpSkip], NOW)[0];
  assert.equal(refused.kind, "refused");
  if (refused.kind === "refused") assert.equal(refused.reason, "malformed");
  // A checkpoint naming an unknown predecessor waits.
  const other = await importIdKey(new Uint8Array(32).fill(37));
  const orphan = await signEvent(other, { seq: 0, kind: "session.checkpoint", extra: checkpointDraft(1n, new Uint8Array(32).fill(55)) });
  assert.equal(store.ingestBatch([orphan], NOW)[0].kind, "waiting");
  // A checkpoint naming ANOTHER author's checkpoint as predecessor —
  // even with the same session header copied — is not the previous
  // segment of its own session chain: refused as malformed.
  const third = await importIdKey(new Uint8Array(32).fill(38));
  const foreign = await signEvent(third, { seq: 0, kind: "session.checkpoint", extra: checkpointDraft(1n, cp0.idBytes) });
  const foreignOutcome = store.ingestBatch([foreign], NOW)[0];
  assert.equal(foreignOutcome.kind, "refused");
  if (foreignOutcome.kind === "refused") assert.equal(foreignOutcome.reason, "malformed");
  // The canonical encoding helper is exercised by signEvent; assert
  // the encoding of a null is the CBOR null byte.
  assert.equal(encodeCanonical({ kind: "null" })[0], 0xf6);
});
