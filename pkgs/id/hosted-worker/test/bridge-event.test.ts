// The signed event codec (src/event.ts) and the Ed25519 seam.
//
// Oracle: chain.rs's decode rules, and cross-language fixtures —
// Rust- and TypeScript-signed events share one byte-exact fixture
// file consumed by both language suites.
import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { encodeCanonical, type CborValue } from "../src/canonical-cbor.ts";
import { bytesToHex, hexToBytes, importIdKey, verifyEd25519 } from "../src/ed25519.ts";
import { decodeEvent, Malformed, parseEvent, signEvent } from "../src/event.ts";

interface BridgeEventVector {
  name: string;
  bytes_hex: string;
  id_hex: string;
  author_hex?: string;
  kind?: string;
  checkpoint_segment_seq?: number;
}

interface SharedVectors {
  bridge_events: {
    typescript_signed: BridgeEventVector[];
    rust_signed: BridgeEventVector[];
  };
}

const SHARED_VECTORS = JSON.parse(
  readFileSync(new URL("../../tests/fixtures/hosted-second-version-g1.json", import.meta.url), "utf8"),
) as SharedVectors;

const seed = (n: number) => new Uint8Array(32).fill(n);

test("ed25519 seam verifies the RFC 8032 test vector", async () => {
  const pub = hexToBytes("d75a980182b10ab7d54bfed3c964073a0ee172f3daa62325af021a68f707511a");
  const sig = hexToBytes(
    "e5564300c360ac729086e2cc806e828a84877f1eb8e5d974d873e065224901555fb8821590a33bacc61e39701cf9b46bd25bf5f0595bbe24655141438e7a100b",
  );
  assert.equal(await verifyEd25519(pub, sig, new Uint8Array(0)), true);
  assert.equal(await verifyEd25519(pub, sig, new Uint8Array([1])), false);
});

test("sign and decode round-trip a chain", async () => {
  const key = await importIdKey(seed(9));
  const a = await signEvent(key, { seq: 0, kind: "message", body: new TextEncoder().encode("hello") });
  const shared = SHARED_VECTORS.bridge_events.typescript_signed;
  assert.equal(shared.length, 2, "shared bridge fixture retains both TypeScript-signed events");
  assert.equal(bytesToHex(a.bytes), shared[0].bytes_hex);
  assert.equal(a.id, shared[0].id_hex);
  assert.equal(a.authorHex, key.publicHex);
  assert.equal(a.seq, 0);
  assert.equal(a.payloadSize, 5n);
  const b = await signEvent(key, { seq: 1, backlink: a.idBytes, kind: "message", after: [a.idBytes] });
  assert.equal(bytesToHex(b.bytes), shared[1].bytes_hex);
  assert.equal(b.id, shared[1].id_hex);
  assert.equal(b.backlinkHex, a.id);
  assert.deepEqual(b.afterHex, [a.id]);
  // Decode is a pure function of the bytes.
  const again = await decodeEvent(a.bytes);
  assert.equal(again.id, a.id);
  assert.equal(parseEvent(b.bytes).id, b.id);
});

test("a corrupted signature is refused", async () => {
  const key = await importIdKey(seed(9));
  const a = await signEvent(key, { seq: 0, kind: "message" });
  const corrupted = new Uint8Array(a.bytes);
  corrupted[40] ^= 0xff; // inside the signature
  await assert.rejects(() => decodeEvent(corrupted), Malformed);
});

test("an event re-keyed to another author is refused", async () => {
  const key = await importIdKey(seed(9));
  const other = await importIdKey(seed(10));
  const a = await signEvent(key, { seq: 0, kind: "message" });
  // Swap the verifying key for another key: the signature no longer
  // verifies against the named author (wrong-key delivery).
  const tampered = new Uint8Array(a.bytes);
  tampered.set(other.publicRaw, 4); // after the 86 01 58 20 header bytes
  await assert.rejects(() => decodeEvent(tampered), /signature does not verify/);
});

test("malformed shapes are refused with named reasons", async () => {
  const key = await importIdKey(seed(9));
  const a = await signEvent(key, { seq: 0, kind: "message" });
  // Truncated.
  await assert.rejects(() => decodeEvent(a.bytes.subarray(0, 20)), Malformed);
  // Trailing garbage.
  const trailed = new Uint8Array(a.bytes.length + 1);
  trailed.set(a.bytes);
  await assert.rejects(() => decodeEvent(trailed), Malformed);
});

const T = (v: string): CborValue => ({ kind: "text", value: v });
const B = (v: Uint8Array): CborValue => ({ kind: "bytes", value: v });
const U = (v: bigint): CborValue => ({ kind: "uint", value: v });

/** Builds a header by hand (unsigned garbage signature) so a
 * structural rule can be exercised in isolation: parseEvent checks
 * structure without the signature. */
function handBuilt(extPairs: [CborValue, CborValue][], seq = 0n): Uint8Array {
  const items: CborValue[] = [U(1n), B(new Uint8Array(32).fill(3)), B(new Uint8Array(64).fill(4)), U(0n), U(seq)];
  if (seq > 0n) items.push(B(new Uint8Array(32).fill(5)));
  items.push({ kind: "map", value: extPairs });
  return encodeCanonical({ kind: "array", value: items });
}

function baseExt(): [CborValue, CborValue][] {
  return [
    [T("domain"), T("id.events.v1")],
    [T("kind"), T("message")],
    [T("group"), { kind: "null" }],
    [T("after"), { kind: "array", value: [] }],
  ];
}

test("structural extension rules (parseEvent, signature aside)", () => {
  // The well-formed baseline parses.
  assert.equal(parseEvent(handBuilt(baseExt())).kind, "message");
  // Missing `group` (serde has no default for it).
  assert.throws(
    () => parseEvent(handBuilt(baseExt().filter(([k]) => !(k.kind === "text" && k.value === "group")))),
    /missing group/,
  );
  // Wrong domain.
  assert.throws(
    () =>
      parseEvent(
        handBuilt(baseExt().map(([k, v]) => [k, k.kind === "text" && k.value === "domain" ? T("id.events.v2") : v])),
      ),
    /domain tag/,
  );
  // Unknown key (deny_unknown_fields).
  assert.throws(() => parseEvent(handBuilt([...baseExt(), [T("bogus"), U(1n)]])), /unknown extension key/);
  // A payload key on a kind outside its family.
  assert.throws(
    () => parseEvent(handBuilt([...baseExt(), [T("tag"), { kind: "map", value: [] }]])),
    /carries a tag payload/,
  );
  // A tag kind without its payload.
  assert.throws(
    () =>
      parseEvent(handBuilt(baseExt().map(([k, v]) => [k, k.kind === "text" && k.value === "kind" ? T("tag.set") : v]))),
    /needs its tag payload/,
  );
  // 33 `after` hashes.
  const many = Array.from({ length: 33 }, (_, i) => B(new Uint8Array(32).fill(i + 1)));
  assert.throws(
    () =>
      parseEvent(
        handBuilt(
          baseExt().map(([k, v]) => [
            k,
            k.kind === "text" && k.value === "after" ? ({ kind: "array", value: many } as CborValue) : v,
          ]),
        ),
      ),
    /most allowed is 32/,
  );
  // A duplicated `after` hash.
  const dup = [B(new Uint8Array(32).fill(7)), B(new Uint8Array(32).fill(7))];
  assert.throws(
    () =>
      parseEvent(
        handBuilt(
          baseExt().map(([k, v]) => [
            k,
            k.kind === "text" && k.value === "after" ? ({ kind: "array", value: dup } as CborValue) : v,
          ]),
        ),
      ),
    /same hash twice/,
  );
  // A kind of 65 bytes.
  assert.throws(
    () =>
      parseEvent(
        handBuilt(baseExt().map(([k, v]) => [k, k.kind === "text" && k.value === "kind" ? T("k".repeat(65)) : v])),
      ),
    /1 to 64 bytes/,
  );
  // A non-NFC kind.
  assert.throws(
    () =>
      parseEvent(handBuilt(baseExt().map(([k, v]) => [k, k.kind === "text" && k.value === "kind" ? T("café") : v]))),
    /not NFC/,
  );
  // seq 0 with a backlink-shaped extra field: the extra item is not
  // the extensions map, so the header is malformed.
  const withBacklink = encodeCanonical({
    kind: "array",
    value: [
      U(1n),
      B(new Uint8Array(32)),
      B(new Uint8Array(64)),
      U(0n),
      U(0n),
      B(new Uint8Array(32)),
      { kind: "map", value: baseExt() },
    ],
  });
  assert.throws(() => parseEvent(withBacklink), Malformed);
});

test("Rust-signed events verify and decode (cross-language fixtures)", async () => {
  const vectors = SHARED_VECTORS.bridge_events.rust_signed;
  assert.equal(vectors.length, 3, "shared bridge fixture retains all Rust-signed events");
  const [r1Vector, r2Vector, r3Vector] = vectors;
  assert.ok(r1Vector && r2Vector && r3Vector);
  const r1 = hexToBytes(r1Vector.bytes_hex);
  const r2 = hexToBytes(r2Vector.bytes_hex);
  const r3 = hexToBytes(r3Vector.bytes_hex);
  const d1 = await decodeEvent(r1);
  assert.equal(d1.id, r1Vector.id_hex);
  assert.equal(d1.authorHex, r1Vector.author_hex);
  assert.equal(d1.seq, 0);
  const d2 = await decodeEvent(r2);
  assert.equal(d2.id, r2Vector.id_hex);
  assert.equal(d2.seq, 1);
  assert.equal(d2.backlinkHex, d1.id);
  assert.deepEqual(d2.afterHex, [d1.id]);
  const d3 = await decodeEvent(r3);
  assert.equal(d3.id, r3Vector.id_hex);
  assert.equal(d3.kind, r3Vector.kind);
  assert.ok(d3.checkpoint);
  assert.equal(d3.checkpoint.segmentSeq, BigInt(r3Vector.checkpoint_segment_seq!));
  assert.equal(d3.checkpoint.prev, null);
});

test("checkpoint payload extraction", async () => {
  const key = await importIdKey(seed(11));
  const header: CborValue = {
    kind: "map",
    value: [
      [T("session_id"), B(new Uint8Array(32).fill(1))],
      [T("purpose"), T("collab")],
    ],
  };
  const segment = (seq: bigint, prev: CborValue): CborValue => ({
    kind: "map",
    value: [
      [T("segment_seq"), U(seq)],
      [T("prev_checkpoint"), prev],
      [T("root"), B(new Uint8Array(32).fill(2))],
    ],
  });
  const cp0 = await signEvent(key, {
    seq: 0,
    kind: "session.checkpoint",
    extra: [
      [
        "checkpoint",
        {
          kind: "map",
          value: [
            [T("header"), header],
            [T("segment"), segment(0n, { kind: "null" })],
          ],
        },
      ],
    ],
  });
  assert.equal(cp0.checkpoint?.segmentSeq, 0n);
  assert.equal(cp0.checkpoint?.prev, null);
  const cp1 = await signEvent(key, {
    seq: 1,
    backlink: cp0.idBytes,
    kind: "session.checkpoint",
    extra: [
      [
        "checkpoint",
        {
          kind: "map",
          value: [
            [T("header"), header],
            [T("segment"), segment(1n, B(cp0.idBytes))],
          ],
        },
      ],
    ],
  });
  assert.equal(bytesToHex(cp1.checkpoint?.prev as Uint8Array), cp0.id);
  assert.equal(
    bytesToHex(cp1.checkpoint?.headerEncoding as Uint8Array),
    bytesToHex(cp0.checkpoint?.headerEncoding as Uint8Array),
  );
});
