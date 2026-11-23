// The signed event codec (src/event.ts) and the Ed25519 seam.
//
// Oracle: chain.rs's decode rules, and cross-language fixtures —
// events signed by the Rust signer in
// pkgs/id/tests/bridge_fixtures.rs are pinned here byte for byte,
// and the TypeScript-signed events below are pinned there.
import { test } from "node:test";
import assert from "node:assert/strict";
import { encodeCanonical, type CborValue } from "../src/canonical-cbor.ts";
import { bytesToHex, hexToBytes, importIdKey, verifyEd25519 } from "../src/ed25519.ts";
import { decodeEvent, Malformed, parseEvent, signEvent } from "../src/event.ts";

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
  assert.equal(a.authorHex, key.publicHex);
  assert.equal(a.seq, 0);
  assert.equal(a.payloadSize, 5n);
  const b = await signEvent(key, { seq: 1, backlink: a.idBytes, kind: "message", after: [a.idBytes] });
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
  const items: CborValue[] = [
    U(1n),
    B(new Uint8Array(32).fill(3)),
    B(new Uint8Array(64).fill(4)),
    U(0n),
    U(seq),
  ];
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
      parseEvent(
        handBuilt(baseExt().map(([k, v]) => [k, k.kind === "text" && k.value === "kind" ? T("tag.set") : v])),
      ),
    /needs its tag payload/,
  );
  // 33 `after` hashes.
  const many = Array.from({ length: 33 }, (_, i) => B(new Uint8Array(32).fill(i + 1)));
  assert.throws(
    () =>
      parseEvent(
        handBuilt(baseExt().map(([k, v]) => [k, k.kind === "text" && k.value === "after" ? { kind: "array", value: many } as CborValue : v])),
      ),
    /most allowed is 32/,
  );
  // A duplicated `after` hash.
  const dup = [B(new Uint8Array(32).fill(7)), B(new Uint8Array(32).fill(7))];
  assert.throws(
    () =>
      parseEvent(
        handBuilt(baseExt().map(([k, v]) => [k, k.kind === "text" && k.value === "after" ? { kind: "array", value: dup } as CborValue : v])),
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
      parseEvent(
        handBuilt(baseExt().map(([k, v]) => [k, k.kind === "text" && k.value === "kind" ? T("café") : v])),
      ),
    /not NFC/,
  );
  // seq 0 with a backlink-shaped extra field: the extra item is not
  // the extensions map, so the header is malformed.
  const withBacklink = encodeCanonical({
    kind: "array",
    value: [U(1n), B(new Uint8Array(32)), B(new Uint8Array(64)), U(0n), U(0n), B(new Uint8Array(32)), { kind: "map", value: baseExt() }],
  });
  assert.throws(() => parseEvent(withBacklink), Malformed);
});

test("Rust-signed events verify and decode (cross-language fixtures)", async () => {
  // Pinned from pkgs/id/tests/bridge_fixtures.rs
  // (rust_signed_events_for_the_hosted_bridge, run on the laptop
  // under the fleet build lock): key seed [7;32], so the two
  // implementations are pinned to each other in both directions.
  const r1 = hexToBytes(
    "86015820ea4a6c63e29c520abef5507b132ec5f9954776aebebe7b92421eea691446d22c5840d2734c4edc475ae59e4a0c1564d2d56c947a5745a11875a7a9024d92cc87d8a5ba60585efb192b968346b5ec659f825d8cffc778a1b610e8407eaa7b5dc1ef000000a4646b696e64676d657373616765656166746572806567726f7570f666646f6d61696e6c69642e6576656e74732e7631",
  );
  const r2 = hexToBytes(
    "88015820ea4a6c63e29c520abef5507b132ec5f9954776aebebe7b92421eea691446d22c5840d8b48098f2f9342ad54e061477d3f99f1b98a4fc442d054126ec789a6602ccd8c89b8f8725a23072f38897f5925e29eada3a713c5ff3327385c4254ce9117f0209582057560fde78cf6da97615969f90f2589f568d389140542e5e6a427d27f3b907ac015820e53b05183acd33ee8377ac6017a013ea111652baed8945d84d80c8cbc2a062e9a4646b696e64676d657373616765656166746572815820e53b05183acd33ee8377ac6017a013ea111652baed8945d84d80c8cbc2a062e96567726f7570f666646f6d61696e6c69642e6576656e74732e7631",
  );
  const r3 = hexToBytes(
    "87015820ea4a6c63e29c520abef5507b132ec5f9954776aebebe7b92421eea691446d22c58405821b6cca2465408ef2e9f041c4f6c9533cb4ba266359583842e928d3c60fee6aaa5de818ed7ca63851dc24cbc6b9eb4406520f7ded3b25ab2136785ba30b10200025820698b10b97ad8e72a5dfd52356af286dd5161937666af047aba9fd673757d901aa5646b696e647273657373696f6e2e636865636b706f696e74656166746572806567726f7570f666646f6d61696e6c69642e6576656e74732e76316a636865636b706f696e74a266686561646572a7656e6f6e636558209a277e65f53a6cd9c4a2e31528a78713dd6cc6dab1118f50b6e2c41ded113cda67707572706f73656e636f6c6c61622065646974696e67696d61635f73636f70656e5065725061727469636970616e746a73657373696f6e5f69645820b9ca9e3c221d11c13424731063c2fca488c5cf507d92af6bf04375cb52371d226a737461727465645f6d731903e86c7061727469636970616e7473815820ea4a6c63e29c520abef5507b132ec5f9954776aebebe7b92421eea691446d22c726d61635f6b65795f636f6d6d69746d656e745820a53d167ccd3f9fffbe81f6024547c2551bcf221f62f7f289b92940f10d974c87677365676d656e74a764726f6f74582092a2b787a06d7272df43eaf87acc3b9c1d315d79d599d61c285983483e43199866656e645f6d73190af06873746172745f6d731903e86a61756469745f626c6f62f66b6576656e745f636f756e741901006b7365676d656e745f736571006f707265765f636865636b706f696e74f6",
  );
  const d1 = await decodeEvent(r1);
  assert.equal(d1.id, "e53b05183acd33ee8377ac6017a013ea111652baed8945d84d80c8cbc2a062e9");
  assert.equal(d1.authorHex, "ea4a6c63e29c520abef5507b132ec5f9954776aebebe7b92421eea691446d22c");
  assert.equal(d1.seq, 0);
  const d2 = await decodeEvent(r2);
  assert.equal(d2.id, "698b10b97ad8e72a5dfd52356af286dd5161937666af047aba9fd673757d901a");
  assert.equal(d2.seq, 1);
  assert.equal(d2.backlinkHex, d1.id);
  assert.deepEqual(d2.afterHex, [d1.id]);
  const d3 = await decodeEvent(r3);
  assert.equal(d3.id, "1131869a4e2f550e4a8fbddd7d4edae53d971969ef02e7f685951c7fbdc67e46");
  assert.equal(d3.kind, "session.checkpoint");
  assert.ok(d3.checkpoint);
  assert.equal(d3.checkpoint.segmentSeq, 0n);
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
    extra: [["checkpoint", { kind: "map", value: [[T("header"), header], [T("segment"), segment(0n, { kind: "null" })]] }]],
  });
  assert.equal(cp0.checkpoint?.segmentSeq, 0n);
  assert.equal(cp0.checkpoint?.prev, null);
  const cp1 = await signEvent(key, {
    seq: 1,
    backlink: cp0.idBytes,
    kind: "session.checkpoint",
    extra: [["checkpoint", { kind: "map", value: [[T("header"), header], [T("segment"), segment(1n, B(cp0.idBytes))]] }]],
  });
  assert.equal(bytesToHex(cp1.checkpoint?.prev as Uint8Array), cp0.id);
  assert.equal(
    bytesToHex(cp1.checkpoint?.headerEncoding as Uint8Array),
    bytesToHex(cp0.checkpoint?.headerEncoding as Uint8Array),
  );
});
