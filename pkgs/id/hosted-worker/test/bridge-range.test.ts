// Range reconciliation (src/range.ts).
//
// Oracle: range.rs's own simulation semantics — two parties must
// learn exactly the symmetric difference of their ID sets, through
// the same frames as the wire, whatever their local K/T are.
import { test } from "node:test";
import assert from "node:assert/strict";
import {
  checkBounds,
  checkParams,
  decodeFlight,
  DEFAULT_RANGE_PARAMS,
  frameFlight,
  keyBytes,
  RangeParty,
  simulateExchange,
  type RangeParams,
  type SyncKey,
} from "../src/range.ts";
import { blake3 } from "../../../hosted-primitives/src/blake3.ts";
import { bytesToHex } from "../src/ed25519.ts";

/** Deterministic PRNG (mulberry32) so the sets are reproducible. */
function rng(seed: number) {
  let a = seed >>> 0;
  return () => {
    a |= 0;
    a = (a + 0x6d2b79f5) | 0;
    let t = Math.imul(a ^ (a >>> 15), 1 | a);
    t = (t + Math.imul(t ^ (t >>> 7), 61 | t)) ^ t;
    return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
  };
}

function digest(seed: string, n: number): Uint8Array {
  const input = new TextEncoder().encode(`${seed}:${n}`);
  return blake3(input);
}

/** Keys spread over the key space, like range.rs's test sets: the
 * author and id are digests, the sequence is the index. */
function makeSet(seed: string, count: number): SyncKey[] {
  const out: SyncKey[] = [];
  for (let n = 0; n < count; n++) {
    out.push({
      author: bytesToHex(digest(`author-${seed}`, n % 4)),
      seq: n,
      id: bytesToHex(digest(`id-${seed}`, n)),
    });
  }
  return out;
}

function symdiff(a: SyncKey[], b: SyncKey[]): Set<string> {
  const aIds = new Set(a.map((k) => k.id));
  const bIds = new Set(b.map((k) => k.id));
  const out = new Set<string>();
  for (const id of aIds) if (!bIds.has(id)) out.add(id);
  for (const id of bIds) if (!aIds.has(id)) out.add(id);
  return out;
}

test("two parties learn exactly the symmetric difference", () => {
  const rand = rng(42);
  for (let trial = 0; trial < 12; trial++) {
    const base = makeSet("base", 60);
    const a = base.filter(() => rand() < 0.8);
    const b = base.filter(() => rand() < 0.8);
    const truth = symdiff(a, b);
    const sim = simulateExchange(new RangeParty(a), DEFAULT_RANGE_PARAMS, new RangeParty(b), DEFAULT_RANGE_PARAMS);
    assert.deepEqual(sim.learnedA, truth, `trial ${trial}: A's learned set`);
    assert.deepEqual(sim.learnedB, truth, `trial ${trial}: B's learned set`);
  }
});

test("different local K and T still converge", () => {
  const a = makeSet("a", 50);
  const b = makeSet("b", 50);
  const truth = symdiff(a, b);
  const tight: RangeParams = { k: 2, t: 1, frameBudget: 1024, sessionBudget: 1 << 20 };
  const sim = simulateExchange(new RangeParty(a), tight, new RangeParty(b), DEFAULT_RANGE_PARAMS);
  assert.deepEqual(sim.learnedA, truth);
  assert.deepEqual(sim.learnedB, truth);
  assert.ok(sim.maxFrame <= DEFAULT_RANGE_PARAMS.frameBudget);
});

test("identical sets learn nothing and end on skips", () => {
  const set = makeSet("same", 30);
  const sim = simulateExchange(new RangeParty(set), DEFAULT_RANGE_PARAMS, new RangeParty(set), DEFAULT_RANGE_PARAMS);
  assert.equal(sim.learnedA.size, 0);
  assert.equal(sim.learnedB.size, 0);
  assert.equal(sim.flights, 2);
});

test("reconciliation cost is sublinear in the set size", () => {
  // The #2276 bound: 100 events over 4 authors reconcile in under
  // 6,400 frame bytes when the sets are identical.
  const set = makeSet("cost", 100);
  const sim = simulateExchange(new RangeParty(set), DEFAULT_RANGE_PARAMS, new RangeParty(set), DEFAULT_RANGE_PARAMS);
  assert.ok(sim.bytesA + sim.bytesB < 6400, `cost ${sim.bytesA + sim.bytesB}`);
});

test("frames round-trip and bounds are checked", () => {
  const party = new RangeParty(makeSet("frames", 40));
  const entries = [party.fullRange()];
  const frames = frameFlight(entries, 256);
  assert.deepEqual(decodeFlight(frames), entries);
  // Bounds must strictly increase.
  assert.throws(() =>
    checkBounds([
      { upper: new Uint8Array([5]), mode: { t: "skip" } },
      { upper: new Uint8Array([5]), mode: { t: "skip" } },
    ]),
  );
  // Nothing follows +infinity.
  assert.throws(() =>
    checkBounds([
      { upper: null, mode: { t: "skip" } },
      { upper: new Uint8Array([5]), mode: { t: "skip" } },
    ]),
  );
  // A bound longer than a key is refused.
  assert.throws(() => checkBounds([{ upper: new Uint8Array(73), mode: { t: "skip" } }]));
});

test("malformed frames are refused", () => {
  const party = new RangeParty(makeSet("bad", 5));
  const frames = frameFlight([party.fullRange()], 4096);
  const good = frames[0];
  const trailing = new Uint8Array([...good, 0]);
  assert.throws(() => decodeFlight([trailing]));
  const badFlag = new Uint8Array(good);
  badFlag[0] = 7;
  assert.throws(() => decodeFlight([badFlag]));
});

test("params are validated like range.rs", () => {
  assert.throws(() => checkParams({ ...DEFAULT_RANGE_PARAMS, k: 1 }));
  assert.throws(() => checkParams({ ...DEFAULT_RANGE_PARAMS, t: 0 }));
  assert.throws(() => checkParams({ ...DEFAULT_RANGE_PARAMS, frameBudget: 100 }));
  assert.equal(checkParams({ ...DEFAULT_RANGE_PARAMS }).k, 16);
});

test("key bytes are author || seq_be64 || id", () => {
  const key: SyncKey = { author: "aa".repeat(32), seq: 0x0102, id: "bb".repeat(32) };
  const bytes = keyBytes(key);
  assert.equal(bytes.length, 72);
  assert.equal(bytes[32 + 6], 0x01);
  assert.equal(bytes[32 + 7], 0x02);
  assert.equal(bytesToHex(bytes.subarray(40)), "bb".repeat(32));
});
